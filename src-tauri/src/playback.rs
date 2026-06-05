// 音频播放器: 用专门的播放线程隔离 rodio (cpal stream 在某些平台不 Send)
//
// 主线程通过 mpsc channel 发命令, 播放线程持有 OutputStream + Sink
// 播放位置和状态用 Arc<Mutex<...>> 共享给主线程查询

use rodio::{Decoder, OutputStream, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let (tx, rx) = channel::<Cmd>();
        let state = Arc::new(Mutex::new(SharedState::default()));
        let state_for_thread = Arc::clone(&state);

        // 启动播放线程, 必须在线程内创建 OutputStream (它不 Send)
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        thread::spawn(move || {
            let (_stream, handle) = match OutputStream::try_default() {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("无法打开音频输出: {}", e)));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));

            let mut sink: Option<Sink> = None;
            let mut volume: f32 = 1.0;

            for cmd in rx {
                match cmd {
                    Cmd::Load(path, reply) => {
                        let result = (|| -> Result<f32, String> {
                            let file = File::open(&path).map_err(|e| format!("打开音频失败: {}", e))?;
                            let decoder = Decoder::new(BufReader::new(file))
                                .map_err(|e| format!("解码失败: {}", e))?;
                            let duration = decoder
                                .total_duration()
                                .map(|d| d.as_secs_f32())
                                .unwrap_or(0.0);
                            let new_sink = Sink::try_new(&handle)
                                .map_err(|e| format!("创建 Sink 失败: {}", e))?;
                            new_sink.append(decoder);
                            new_sink.pause();
                            new_sink.set_volume(volume);

                            if let Some(old) = sink.take() {
                                old.stop();
                            }
                            sink = Some(new_sink);

                            let mut s = state_for_thread.lock().unwrap();
                            s.duration = duration;
                            s.accumulated_secs = 0.0;
                            s.playback_start = None;
                            s.is_playing = false;
                            Ok(duration)
                        })();
                        let _ = reply.send(result);
                    }
                    Cmd::Play => {
                        if let Some(s) = &sink {
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
                        if let Some(s) = &sink {
                            let _ = s.try_seek(Duration::from_secs_f32(secs.max(0.0)));
                            let mut st = state_for_thread.lock().unwrap();
                            st.accumulated_secs = secs.max(0.0);
                            if !s.is_paused() {
                                st.playback_start = Some(Instant::now());
                            } else {
                                st.playback_start = None;
                            }
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
        self.tx.send(Cmd::Play).map_err(|_| "播放线程已停止".to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.tx.send(Cmd::Pause).map_err(|_| "播放线程已停止".to_string())
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
        self.state.lock().unwrap().is_playing
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}
