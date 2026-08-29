// 音频播放器: 用专门的播放线程隔离 rodio (cpal stream 在某些平台不 Send)
//
// 主线程通过 mpsc channel 发命令, 播放线程持有 OutputStream + Sink
// 播放位置和状态用 Arc<Mutex<...>> 共享给主线程查询
//
// 解码不使用 rodio::Decoder: 它的 ReadSeekSource::byte_len() 恒为 None,
// 导致 symphonia 对 FLAC 等格式的 seek 永远返回 Unseekable (拖进度条声音不跳转)。
// 这里自建带 byte_len 的 MediaSource + symphonia 格式读取器, seek 正常工作。

use rodio::{Sink, Source};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

enum Cmd {
    Load(String, Sender<Result<f32, String>>),
    Play,
    Pause,
    Seek(f32),
    SetVolume(f32),
    Shutdown,
}

#[derive(Default)]
struct SharedState {
    duration: f32,
    accumulated_secs: f32,
    playback_start: Option<Instant>,
    is_playing: bool,
}

impl SharedState {
    fn position(&self) -> f32 {
        let mut pos = self.accumulated_secs;
        if let Some(start) = self.playback_start {
            pos += start.elapsed().as_secs_f32();
        }
        if self.duration > 0.0 {
            pos.min(self.duration)
        } else {
            pos
        }
    }
}

pub struct AudioPlayer {
    tx: Sender<Cmd>,
    state: Arc<Mutex<SharedState>>,
}

// ── 带 byte_len 的 symphonia 播放源 ──────────────────────────

struct FileMediaSource {
    file: File,
    len: u64,
}

impl MediaSource for FileMediaSource {
    fn is_seekable(&self) -> bool {
        true
    }
    /// symphonia FLAC demuxer 的 seek 需要流长度, rodio 内置包装缺失此信息导致 Unseekable
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl Read for FileMediaSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for FileMediaSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

/// 直接基于 symphonia 的播放解码源 (支持精确 seek)
pub struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    buffer: Vec<f32>,
    pos: usize,
    total_duration: Option<Duration>,
}

impl SymphoniaSource {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("打开音频失败: {}", e))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }
        let mss =
            MediaSourceStream::new(Box::new(FileMediaSource { file, len }), Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("识别音频格式失败: {}", e))?;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "找不到可解码的音轨".to_string())?;
        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2) as u16;
        let total_duration = track
            .codec_params
            .time_base
            .zip(track.codec_params.n_frames)
            .map(|(tb, n)| {
                let t = tb.calc_time(n);
                Duration::from_secs_f64(t.seconds as f64 + t.frac)
            });

        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("无法创建解码器: {}", e))?;

        Ok(Self {
            format: probed.format,
            decoder,
            track_id,
            sample_rate,
            channels,
            buffer: Vec::new(),
            pos: 0,
            total_duration,
        })
    }

    /// 解码下一个包到交错 f32 缓冲; 流结束/致命错误返回 false
    fn decode_next_packet(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(_) => return false,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut sbuf =
                        SampleBuffer::<f32>::new(decoded.frames() as u64, *decoded.spec());
                    sbuf.copy_interleaved_ref(decoded);
                    self.buffer = sbuf.samples().to_vec();
                    self.pos = 0;
                    return true;
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.buffer.len() && !self.decode_next_packet() {
            return None;
        }
        let v = self.buffer[self.pos];
        self.pos += 1;
        Some(v)
    }
}

impl Source for SymphoniaSource {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn current_frame_len(&self) -> Option<usize> {
        Some((self.buffer.len() - self.pos) / self.channels as usize)
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        use rodio::source::SeekError;
        // 恰好 seek 到末尾 symphonia 会报 OutOfRange, 略微回退
        let mut target = pos;
        if let Some(dur) = self.total_duration {
            if dur.as_secs_f64() - target.as_secs_f64() < 0.05 {
                target = Duration::from_secs_f64((dur.as_secs_f64() - 0.05).max(0.0));
            }
        }
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: target.into(),
                    track_id: None,
                },
            )
            .map_err(|e| SeekError::Other(Box::new(e)))?;
        // symphonia 0.5: reset 无返回值
        self.decoder.reset();
        self.buffer.clear();
        self.pos = 0;
        Ok(())
    }
}

impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let (tx, rx) = channel::<Cmd>();
        let state = Arc::new(Mutex::new(SharedState::default()));
        let state_for_thread = Arc::clone(&state);

        // 启动播放线程, 必须在线程内创建 OutputStream (它不 Send)
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        thread::spawn(move || {
            let (_stream, handle) = match rodio::OutputStream::try_default() {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("无法打开音频输出: {}", e)));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));

            let mut sink: Option<Sink> = None;
            let mut current_path: Option<String> = None;
            let mut volume: f32 = 1.0;

            for cmd in rx {
                match cmd {
                    Cmd::Load(path, reply) => {
                        let result = load_sink(&handle, &path, 0.0, volume, true, &mut sink);
                        if let Ok(d) = &result {
                            current_path = Some(path);
                            let mut s = state_for_thread.lock().unwrap();
                            s.duration = *d;
                            s.accumulated_secs = 0.0;
                            s.playback_start = None;
                            s.is_playing = false;
                        }
                        let _ = reply.send(result);
                    }
                    Cmd::Play => {
                        let empty = sink.as_ref().map(|s| s.empty()).unwrap_or(true);
                        if empty {
                            // 播放结束/未加载: 从用户拖动到的位置恢复 (自然播完则从头)
                            let resume = {
                                let st = state_for_thread.lock().unwrap();
                                let within = st.accumulated_secs > 0.05
                                    && (st.duration <= 0.0
                                        || st.accumulated_secs < st.duration - 0.05);
                                if within {
                                    st.accumulated_secs
                                } else {
                                    0.0
                                }
                            };
                            let path = current_path.clone();
                            if let Some(path) = path {
                                if let Ok(d) =
                                    load_sink(&handle, &path, resume, volume, false, &mut sink)
                                {
                                    let mut st = state_for_thread.lock().unwrap();
                                    st.duration = d;
                                    st.accumulated_secs = resume;
                                    st.playback_start = Some(Instant::now());
                                    st.is_playing = true;
                                }
                            }
                        } else if let Some(s) = &sink {
                            if s.is_paused() {
                                s.play();
                                let mut st = state_for_thread.lock().unwrap();
                                st.playback_start = Some(Instant::now());
                                st.is_playing = true;
                            }
                        }
                    }
                    Cmd::Pause => {
                        if let Some(s) = &sink {
                            if !s.is_paused() {
                                s.pause();
                                let mut st = state_for_thread.lock().unwrap();
                                if let Some(start) = st.playback_start.take() {
                                    st.accumulated_secs += start.elapsed().as_secs_f32();
                                }
                                st.is_playing = false;
                            }
                        }
                    }
                    Cmd::Seek(secs) => {
                        let secs = secs.max(0.0);
                        let was_playing = state_for_thread.lock().unwrap().is_playing;
                        let empty = sink.as_ref().map(|s| s.empty()).unwrap_or(true);

                        // 优先原地 seek; sink 已空 (播放结束, try_seek 是 no-op) 或失败则重建
                        let mut ok = false;
                        if !empty {
                            if let Some(s) = &sink {
                                ok = s.try_seek(Duration::from_secs_f32(secs)).is_ok();
                            }
                        }
                        if !ok {
                            if let Some(path) = current_path.clone() {
                                if let Ok(d) =
                                    load_sink(&handle, &path, secs, volume, !was_playing, &mut sink)
                                {
                                    let mut st = state_for_thread.lock().unwrap();
                                    st.duration = d;
                                    ok = true;
                                    let _ = d;
                                }
                            }
                        }
                        if ok {
                            let mut st = state_for_thread.lock().unwrap();
                            st.accumulated_secs = secs;
                            st.playback_start = if was_playing {
                                Some(Instant::now())
                            } else {
                                None
                            };
                        }
                    }
                    Cmd::SetVolume(v) => {
                        let v = v.clamp(0.0, 2.0);
                        volume = v;
                        if let Some(s) = &sink {
                            s.set_volume(v);
                        }
                    }
                    Cmd::Shutdown => break,
                }
            }
        });

        ready_rx
            .recv()
            .map_err(|_| "播放线程未启动".to_string())??;
        Ok(Self { tx, state })
    }

    pub fn load(&self, path: &str) -> Result<f32, String> {
        let (tx, rx) = channel();
        self.tx
            .send(Cmd::Load(path.to_string(), tx))
            .map_err(|_| "播放线程已停止".to_string())?;
        rx.recv().map_err(|_| "播放线程无响应".to_string())?
    }

    pub fn play(&self) -> Result<(), String> {
        self.tx
            .send(Cmd::Play)
            .map_err(|_| "播放线程已停止".to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.tx
            .send(Cmd::Pause)
            .map_err(|_| "播放线程已停止".to_string())
    }

    pub fn seek(&self, secs: f32) -> Result<(), String> {
        self.tx
            .send(Cmd::Seek(secs))
            .map_err(|_| "播放线程已停止".to_string())
    }

    pub fn set_volume(&self, vol: f32) -> Result<(), String> {
        self.tx
            .send(Cmd::SetVolume(vol))
            .map_err(|_| "播放线程已停止".to_string())
    }

    pub fn position(&self) -> f32 {
        self.state.lock().unwrap().position()
    }

    pub fn duration(&self) -> f32 {
        self.state.lock().unwrap().duration
    }

    pub fn is_playing(&self) -> bool {
        let st = self.state.lock().unwrap();
        if st.duration > 0.0 && st.position() >= st.duration {
            false
        } else {
            st.is_playing
        }
    }
}

/// 创建新 sink 载入 path 并定位到 secs; start_paused 控制初始状态; 替换旧 sink
fn load_sink(
    handle: &rodio::OutputStreamHandle,
    path: &str,
    secs: f32,
    volume: f32,
    start_paused: bool,
    sink_slot: &mut Option<Sink>,
) -> Result<f32, String> {
    let mut source = SymphoniaSource::open(Path::new(path))?;
    let duration = source
        .total_duration()
        .map(|d| d.as_secs_f32())
        .unwrap_or(0.0);
    if secs > 0.05 {
        let _ = source.try_seek(Duration::from_secs_f32(secs));
    }
    let new_sink = Sink::try_new(handle).map_err(|e| format!("创建 Sink 失败: {}", e))?;
    new_sink.append(source);
    new_sink.set_volume(volume);
    if start_paused {
        new_sink.pause();
    }
    if let Some(old) = sink_slot.take() {
        old.stop();
    }
    *sink_slot = Some(new_sink);
    Ok(duration)
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}
