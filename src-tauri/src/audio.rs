// Audio decoding + resampling to 16 kHz mono f32 (FCPE 输入要求)
//
// 用 symphonia 解码任意主流格式 (wav/flac/mp3/ogg/aac)，
// 再用 rubato 做高质量重采样到 16 kHz。

use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const TARGET_SR: u32 = 16_000;

pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// 解码任意音频文件，混音到单声道并重采样到 16 kHz。
pub fn load_audio_16k_mono(path: &Path) -> Result<DecodedAudio, String> {
    let file = File::open(path).map_err(|e| format!("打开音频失败: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("识别音频格式失败: {}", e))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "找不到可解码的音轨".to_string())?;

    let track_id = track.id;
    let src_sr = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "音频缺少采样率信息".to_string())?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("无法创建解码器: {}", e))?;

    let mut mono_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(format!("读取音频包失败: {}", e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => append_mono(&buf, channels, &mut mono_samples),
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("解码失败: {}", e)),
        }
    }

    let samples = if src_sr == TARGET_SR {
        mono_samples
    } else {
        resample_to_16k(&mono_samples, src_sr)?
    };

    Ok(DecodedAudio {
        samples,
        sample_rate: TARGET_SR,
    })
}

/// 把一帧解码后的多声道音频缓冲混音成单声道附加到 mono_samples。
fn append_mono(buf: &AudioBufferRef<'_>, channels: usize, out: &mut Vec<f32>) {
    macro_rules! mix_planar {
        ($buf:expr, $cast:expr) => {{
            let frames = $buf.frames();
            for f in 0..frames {
                let mut acc = 0.0f32;
                for c in 0..channels {
                    acc += $cast($buf.chan(c)[f]);
                }
                out.push(acc / channels as f32);
            }
        }};
    }

    match buf {
        AudioBufferRef::F32(b) => mix_planar!(b, |x: f32| x),
        AudioBufferRef::S16(b) => mix_planar!(b, |x: i16| x as f32 / 32768.0),
        AudioBufferRef::S32(b) => mix_planar!(b, |x: i32| x as f32 / 2147483648.0),
        AudioBufferRef::U8(b) => mix_planar!(b, |x: u8| (x as f32 - 128.0) / 128.0),
        AudioBufferRef::F64(b) => mix_planar!(b, |x: f64| x as f32),
        AudioBufferRef::U16(b) => mix_planar!(b, |x: u16| (x as f32 - 32768.0) / 32768.0),
        AudioBufferRef::U24(b) => mix_planar!(b, |x: symphonia::core::sample::u24| {
            (x.inner() as f32 - 8_388_608.0) / 8_388_608.0
        }),
        AudioBufferRef::U32(b) => mix_planar!(b, |x: u32| (x as f32 - 2_147_483_648.0) / 2_147_483_648.0),
        AudioBufferRef::S8(b) => mix_planar!(b, |x: i8| x as f32 / 128.0),
        AudioBufferRef::S24(b) => mix_planar!(b, |x: symphonia::core::sample::i24| x.inner() as f32 / 8_388_608.0),
    }
}

/// 用 rubato 的高质量 sinc 重采样到 16 kHz。
fn resample_to_16k(samples: &[f32], src_sr: u32) -> Result<Vec<f32>, String> {
    let ratio = TARGET_SR as f64 / src_sr as f64;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    // 单 chunk 一次性处理（FCPE 通常处理几十秒的片段，一次性 OK）
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, samples.len(), 1)
        .map_err(|e| format!("创建重采样器失败: {}", e))?;
    let input = vec![samples.to_vec()];
    let mut out = resampler
        .process(&input, None)
        .map_err(|e| format!("重采样失败: {}", e))?;
    Ok(out.remove(0))
}
