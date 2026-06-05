// 验证 Rust mel + ONNX 推理与 Python 参考数据一致

use pitch_analyzer_tauri_lib::mel::{MelConfig, MelExtractor};
use pitch_analyzer_tauri_lib::decoder::FCPEDecoder;
use pitch_analyzer_tauri_lib::audio::load_audio_16k_mono;
use pitch_analyzer_tauri_lib::dsp::post_process;
use ndarray::Array3;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct TestData {
    wav: Vec<f32>,
    mel: Vec<Vec<Vec<f32>>>,      // (1, T, 128)
    latent: Vec<Vec<Vec<f32>>>,   // (1, T, 360)
}

fn load_test_data() -> TestData {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("models/test_data.json");
    let content = std::fs::read_to_string(&path).unwrap();
    serde_json::from_str(&content).unwrap()
}

#[test]
fn test_mel_matches_python() {
    let td = load_test_data();
    let wav = &td.wav;
    let ref_mel = &td.mel[0]; // (T, 128)

    let extractor = MelExtractor::new(MelConfig::default());
    let rust_mel = extractor.compute(wav);

    let ref_t = ref_mel.len();
    let rust_t = rust_mel.nrows();
    println!("ref frames: {}, rust frames: {}", ref_t, rust_t);

    let min_t = ref_t.min(rust_t);
    let mut max_diff = 0.0f32;
    let mut total_diff = 0.0f32;
    let mut count = 0usize;
    for f in 0..min_t {
        for m in 0..128 {
            let r = ref_mel[f][m];
            let c = rust_mel[(f, m)];
            let diff = (r - c).abs();
            max_diff = max_diff.max(diff);
            total_diff += diff;
            count += 1;
        }
    }
    let mean_diff = total_diff / count as f32;
    println!("mel max_diff: {:.6}, mean_diff: {:.6}", max_diff, mean_diff);

    // 允许一定误差 (reflect pad 细节差异可能导致边界帧不同)
    assert!(mean_diff < 0.1, "mel mean_diff too large: {}", mean_diff);
}

#[test]
fn test_onnx_inference_matches_python() {
    let td = load_test_data();
    let ref_mel = &td.mel[0]; // (T, 128)
    let ref_latent = &td.latent[0]; // (T, 360)
    let t = ref_mel.len();

    // 构建 (1, T, 128) ndarray
    let mut mel_arr = Array3::<f32>::zeros((1, t, 128));
    for f in 0..t {
        for m in 0..128 {
            mel_arr[[0, f, m]] = ref_mel[f][m];
        }
    }

    let model_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("models/fcpe.onnx");

    if !model_path.exists() {
        eprintln!("Skipping ONNX test: model not found at {:?}", model_path);
        return;
    }

    let session = Session::builder()
        .unwrap()
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .unwrap()
        .commit_from_file(&model_path)
        .unwrap();
    let mut session = session;

    let input = Tensor::from_array(mel_arr).unwrap();
    let outputs = session.run(ort::inputs![input]).unwrap();
    let latent = outputs[0].try_extract_array::<f32>().unwrap();

    let mut max_diff = 0.0f32;
    let mut total_diff = 0.0f32;
    let mut count = 0usize;
    for f in 0..t {
        for d in 0..360 {
            let r = ref_latent[f][d];
            let c = latent[[0, f, d]];
            let diff = (r - c).abs();
            max_diff = max_diff.max(diff);
            total_diff += diff;
            count += 1;
        }
    }
    let mean_diff = total_diff / count as f32;
    println!("latent max_diff: {:.8}, mean_diff: {:.8}", max_diff, mean_diff);

    assert!(max_diff < 1e-4, "latent max_diff too large: {}", max_diff);
}

#[test]
fn test_decoder_produces_f0() {
    let td = load_test_data();
    let ref_latent = &td.latent[0]; // (T, 360)

    // 从 config 加载 cent_table
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("models/fcpe_config.json");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let cent_table: Vec<f32> = config["cent_table"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_f64().map(|x| x as f32))
        .collect();

    let decoder = FCPEDecoder::new(cent_table);
    let t = ref_latent.len();
    let mut latent_arr = ndarray::Array2::<f32>::zeros((t, 360));
    for f in 0..t {
        for d in 0..360 {
            latent_arr[(f, d)] = ref_latent[f][d];
        }
    }

    let (f0, conf) = decoder.decode(latent_arr.view(), 0.05);
    let voiced: Vec<(usize, f32)> = f0
        .iter()
        .enumerate()
        .filter(|(_, &f)| f > 0.0)
        .map(|(i, &f)| (i, f))
        .collect();

    println!("total frames: {}, voiced: {}", t, voiced.len());
    if !voiced.is_empty() {
        let avg_f0: f32 = voiced.iter().map(|(_, f)| *f).sum::<f32>() / voiced.len() as f32;
        println!("avg f0: {:.1} Hz (expected: ~100-800 Hz for vocals)", avg_f0);
    }

    // 测试音频是随机的，只有少数帧 voiced 是正常的
    // 主要验证 decoder 不 panic 且输出范围合理
    assert!(voiced.len() > 0, "no voiced frames at all");
    for (_, f) in &voiced {
        assert!(*f > 10.0 && *f < 10000.0, "f0 out of range: {}", f);
    }
}

#[test]
fn test_end_to_end_real_audio() {
    let audio_path = std::path::Path::new("/tmp/test_vocal.flac");
    if !audio_path.exists() {
        eprintln!("Skipping e2e test: no /tmp/test_vocal.flac");
        return;
    }
    let model_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("models/fcpe.onnx");
    if !model_path.exists() {
        eprintln!("Skipping e2e test: no model");
        return;
    }

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("models/fcpe_config.json");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let cent_table: Vec<f32> = config["cent_table"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_f64().map(|x| x as f32))
        .collect();

    // 1. 加载音频
    println!("加载音频...");
    let t = std::time::Instant::now();
    let audio = load_audio_16k_mono(audio_path).unwrap();
    println!("解码耗时: {:?}, 样本数: {}, sr={}",
             t.elapsed(), audio.samples.len(), audio.sample_rate);

    // 2. mel
    let t = std::time::Instant::now();
    let extractor = MelExtractor::new(MelConfig::default());
    let mel = extractor.compute(&audio.samples);
    println!("Mel 耗时: {:?}, shape: {:?}", t.elapsed(), mel.dim());

    // 3. ONNX
    let t = std::time::Instant::now();
    let mel_3d: ndarray::Array3<f32> = mel.insert_axis(ndarray::Axis(0));
    let mut session = Session::builder()
        .unwrap()
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .unwrap()
        .commit_from_file(&model_path)
        .unwrap();
    let input = Tensor::from_array(mel_3d).unwrap();
    let outputs = session.run(ort::inputs![input]).unwrap();
    let latent = outputs[0].try_extract_array::<f32>().unwrap();
    let latent_view = latent.view().into_dimensionality::<ndarray::Ix3>().unwrap();
    let latent_2d = latent_view.index_axis(ndarray::Axis(0), 0);
    println!("ONNX 推理耗时: {:?}", t.elapsed());

    // 4. decode
    let t = std::time::Instant::now();
    let decoder = FCPEDecoder::new(cent_table);
    let (f0, conf) = decoder.decode(latent_2d, 0.3);
    println!("Decode 耗时: {:?}", t.elapsed());

    // 5. post process
    let t = std::time::Instant::now();
    let (times, freqs, midis) = post_process(&f0, &conf, 0.3, 65.0, 1300.0, 11, 15, false);
    println!("Post-process 耗时: {:?}", t.elapsed());

    // 统计
    let voiced: Vec<f32> = midis.iter().filter(|m| !m.is_nan()).copied().collect();
    println!("总帧数: {}, voiced: {} ({:.1}%)",
             midis.len(), voiced.len(),
             100.0 * voiced.len() as f32 / midis.len() as f32);
    if !voiced.is_empty() {
        let min_midi = voiced.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_midi = voiced.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let avg_midi = voiced.iter().sum::<f32>() / voiced.len() as f32;
        println!("MIDI 范围: {:.1} ~ {:.1}, 平均: {:.1}",
                 min_midi, max_midi, avg_midi);
        let avg_freq = freqs.iter().filter(|f| !f.is_nan()).sum::<f32>()
            / voiced.len() as f32;
        println!("平均频率: {:.1} Hz", avg_freq);
        println!("时长: {:.1} 秒", times[times.len()-1]);
    }

    assert!(voiced.len() > 100, "太少 voiced 帧");
}
