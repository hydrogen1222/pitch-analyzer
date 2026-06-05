// 整合 audio + mel + ONNX + decoder + dsp 的完整 FCPE 分析流水线

use crate::audio::load_audio_16k_mono;
use crate::decoder::FCPEDecoder;
use crate::dsp::post_process;
use crate::mel::{MelConfig, MelExtractor};
use crate::models::{AnalyzerConfig, PitchTrack};
use ndarray::Array3;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;

pub struct PitchAnalyzer {
    session: Mutex<Session>,
    mel_extractor: MelExtractor,
    decoder: FCPEDecoder,
}

impl PitchAnalyzer {
    pub fn new(model_path: &str, cent_table: Vec<f32>) -> Result<Self, Box<dyn std::error::Error>> {
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(num_cpus())?
            .commit_from_file(model_path)?;

        let mel_extractor = MelExtractor::new(MelConfig::default());
        let decoder = FCPEDecoder::new(cent_table);

        Ok(Self {
            session: Mutex::new(session),
            mel_extractor,
            decoder,
        })
    }

    pub fn analyze<F>(
        &self,
        audio_path: &str,
        config: &AnalyzerConfig,
        mut progress_cb: F,
    ) -> Result<PitchTrack, Box<dyn std::error::Error>>
    where
        F: FnMut(f32, &str),
    {
        // 1. 解码音频 → 16 kHz mono
        progress_cb(0.1, "解码音频并重采样...");
        let audio = load_audio_16k_mono(Path::new(audio_path))
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        // 2. mel 频谱 (n_frames, 128)
        progress_cb(0.4, "计算 Mel 频谱...");
        let mel = self.mel_extractor.compute(&audio.samples);
        let (n_frames, n_mels) = mel.dim();
        if n_frames == 0 {
            return Err("音频太短，无法计算 mel".into());
        }

        // 3. ONNX 推理: (1, n_frames, 128) → (1, n_frames, 360)
        progress_cb(0.6, "正在进行 AI 音高估计推理...");
        let mel_3d: Array3<f32> = mel.insert_axis(ndarray::Axis(0));
        let input_tensor = Tensor::from_array(mel_3d)?;

        let mut session = self.session.lock().unwrap();
        let outputs = session.run(ort::inputs![input_tensor])?;
        let latent = outputs[0].try_extract_array::<f32>()?;
        let latent = latent.view().into_dimensionality::<ndarray::Ix3>()?;
        // latent shape: (1, n_frames, 360)
        let latent_2d = latent.index_axis(ndarray::Axis(0), 0);

        // 4. 解码 latent → f0, confidence
        progress_cb(0.8, "解码音高特征数据...");
        let (f0, conf) = self.decoder.decode(latent_2d, config.confidence_threshold);

        // 5. DSP 后处理
        progress_cb(0.9, "应用 DSP 后处理平滑滤波...");
        let (times, frequencies, midis) = post_process(
            &f0,
            &conf,
            config.confidence_threshold,
            config.fmin,
            config.fmax,
            config.median_smoothing,
            config.smoothing,
            config.quantize,
        );

        // 6. 对齐长度
        let min_len = times
            .len()
            .min(frequencies.len())
            .min(conf.len())
            .min(midis.len());

        let _ = n_mels; // 防止 unused warning
        progress_cb(1.0, "分析完成");

        Ok(PitchTrack {
            times: times[..min_len].to_vec(),
            frequencies: frequencies[..min_len].to_vec(),
            midis: midis[..min_len].to_vec(),
            confidences: conf[..min_len].to_vec(),
        })
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1)
}
