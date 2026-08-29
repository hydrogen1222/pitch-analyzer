// 验证播放源 try_seek 是否真正生效 (无头测试, 不需要音频设备)
//
// 方法: seek 后数剩余采样帧数, 与 (总时长 - seek位置) × 采样率 对比。
// 注: rodio::Decoder 的 symphonia 包装 byte_len() 为 None, FLAC seek 报 Unseekable,
//     播放改用自建 SymphoniaSource (带 byte_len), 此测试守护该行为。

use std::path::PathBuf;
use std::time::Duration;

use pitch_analyzer_tauri_lib::playback::SymphoniaSource;
use rodio::source::Source;

fn find_song() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap();
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("flac"))
}

#[test]
fn symphonia_source_try_seek_flac() {
    let Some(song) = find_song() else {
        eprintln!("Skipping: no flac in repo root");
        return;
    };
    println!("song: {}", song.display());

    let mut src = SymphoniaSource::open(&song).unwrap();
    let sr = src.sample_rate();
    let ch = src.channels() as u64;
    let total = src.total_duration().unwrap().as_secs_f64();
    println!("sr: {}, ch: {}, total: {:.2}s", sr, ch, total);
    assert!(total > 60.0, "song too short for seek test");

    // seek 到 60s
    let res = src.try_seek(Duration::from_secs(60));
    println!("try_seek(60s): {:?}", res.as_ref().map(|_| "Ok"));
    res.expect("seek must succeed for flac");

    // 数剩余采样帧 (交错采样数 / 声道数)
    let remaining_samples: u64 = src.by_ref().count() as u64;
    let remaining_secs = remaining_samples as f64 / ch as f64 / sr as f64;
    println!("remaining after seek: {:.2}s", remaining_secs);

    let expect = total - 60.0;
    assert!(
        (remaining_secs - expect).abs() < 1.5,
        "seek did not take effect: expected ~{:.2}s remaining, got {:.2}s",
        expect,
        remaining_secs
    );

    // 边界: seek 到末尾附近 (之前 symphonia 会 OutOfRange)
    let res_end = src2_seek_end(&song, total - 3.0);
    assert!(res_end, "seek near end must succeed");
}

fn src2_seek_end(song: &std::path::Path, secs: f64) -> bool {
    let mut src = SymphoniaSource::open(song).unwrap();
    src.try_seek(Duration::from_secs_f64(secs)).is_ok()
}
