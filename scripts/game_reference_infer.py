"""Official-pipeline GAME ONNX reference inference (parity harness side A).

Re-implements the exact graph sequence of openvpi/dataset-tools `game-infer`
(C++/onnxruntime) using Python onnxruntime, as an independent reference for
the Rust decoder parity test (`round3_acceptance::game_official_reference_parity`).

Pipeline (per silent-cut chunk):
    encoder(waveform, duration[, language]) -> x_seg, x_est, maskT
    segmenter D3PM t = 0..1 (8 steps)       -> boundaries
    bd2dur(boundaries, maskT)               -> durations, maskN
    estimator(x_est, boundaries, maskT, maskN, threshold) -> presence, scores

The JSON output is an array of {start, end, midi} seconds notes where
presence > 0.5, matching the `GameReferenceNote` schema consumed by the
Rust side.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
import soundfile as sf  # librosa dependency; available in the venv


def load_config(model_dir: Path) -> dict:
    return json.loads((model_dir / "config.json").read_text(encoding="utf-8"))


def run_encoder(sess, wave: np.ndarray, language: int):
    n = wave.shape[0]
    duration = np.float32(n / 44100.0)
    inputs = {
        "waveform": wave[None, :].astype(np.float32),
        "duration": np.array([duration], dtype=np.float32),
    }
    names = {i.name for i in sess.get_inputs()}
    if "language" in names:
        inputs["language"] = np.array([language], dtype=np.int64)
    outputs = sess.run(["x_seg", "x_est", "maskT"], inputs)
    return outputs[0][0], outputs[1][0], outputs[2][0].astype(np.uint8)


def run_segmenter(sess, x_seg, language, mask_t, threshold, radius):
    t = mask_t.shape[0]
    known = np.zeros(t, dtype=bool)
    current = np.zeros(t, dtype=bool)
    ts = [i / 8.0 for i in range(8)]
    for ts_value in ts:
        current = sess.run(
            ["boundaries"],
            {
                "x_seg": x_seg[None, :, :].astype(np.float32),
                "language": np.array([language], dtype=np.int64),
                "known_boundaries": known[None, :].astype(bool),
                "prev_boundaries": current[None, :].astype(bool),
                "t": np.array([ts_value], dtype=np.float32),
                "maskT": mask_t[None, :].astype(bool),
                "threshold": np.array(threshold, dtype=np.float32),
                "radius": np.array(radius, dtype=np.int64),
            },
        )[0][0].astype(np.uint8)
    return current


def run_bd2dur(sess, boundaries, mask_t):
    durations, mask_n = sess.run(
        ["durations", "maskN"],
        {
            "boundaries": boundaries[None, :].astype(bool),
            "maskT": mask_t[None, :].astype(bool),
        },
    )
    return durations[0], mask_n[0].astype(np.uint8)


def run_estimator(sess, x_est, boundaries, mask_t, mask_n, threshold):
    presence, scores = sess.run(
        ["presence", "scores"],
        {
            "x_est": x_est[None, :, :].astype(np.float32),
            "boundaries": boundaries[None, :].astype(bool),
            "maskT": mask_t[None, :].astype(bool),
            "maskN": mask_n[None, :].astype(bool),
            "threshold": np.array(threshold, dtype=np.float32),
        },
    )
    return presence[0].astype(np.float32), scores[0]


def silence_chunks(wave: np.ndarray, sr: int) -> list[tuple[int, int]]:
    hop = sr // 100
    threshold = 0.02
    min_silence = sr // 5
    min_chunk = sr // 2
    max_chunk = sr * 20

    n_frames = (wave.shape[0] + hop - 1) // hop
    rms = np.array(
        [np.sqrt(np.mean(np.square(wave[i * hop:(i + 1) * hop]))) for i in range(n_frames)]
    )

    chunks: list[tuple[int, int]] = []
    chunk_start = 0
    silence_run = 0
    silence_center: int | None = None
    for ri, r in enumerate(rms):
        pos = min((ri + 1) * hop, wave.shape[0])
        if r < threshold:
            silence_run += hop
            if silence_center is None and silence_run >= min_silence // 2:
                silence_center = pos
        else:
            silence_run = 0
            silence_center = None
        chunk_len = pos - chunk_start
        if chunk_len >= min_chunk and silence_run >= min_silence:
            if (
                silence_center is not None
                and silence_center - chunk_start >= min_chunk
                and wave.shape[0] - silence_center >= min_chunk // 4
            ):
                chunks.append((chunk_start, silence_center))
                chunk_start = silence_center
                silence_run = 0
                silence_center = None
        if chunk_len >= max_chunk:
            chunks.append((chunk_start, pos))
            chunk_start = pos
            silence_run = 0
            silence_center = None
    if wave.shape[0] - chunk_start >= min_chunk // 4:
        chunks.append((chunk_start, wave.shape[0]))
    return chunks


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", required=True)
    ap.add_argument("--audio", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--language", type=int, default=0)
    args = ap.parse_args()

    model_dir = Path(args.model_dir)
    config = load_config(model_dir)
    sr = int(config.get("samplerate", 44100))
    timestep = float(config.get("timestep", 0.01))
    seg_threshold = float(config.get("seg_threshold", 0.2))
    seg_radius = max(1, round(float(config.get("seg_radius_seconds", 0.02)) / timestep))
    est_threshold = float(config.get("est_threshold", 0.2))

    so = ort.SessionOptions()
    so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_EXTENDED
    so.intra_op_num_threads = 4
    enc = ort.InferenceSession(str(model_dir / "encoder.onnx"), so, providers=["CPUExecutionProvider"])
    seg = ort.InferenceSession(str(model_dir / "segmenter.onnx"), so, providers=["CPUExecutionProvider"])
    bd2dur = ort.InferenceSession(str(model_dir / "bd2dur.onnx"), so, providers=["CPUExecutionProvider"])
    est = ort.InferenceSession(str(model_dir / "estimator.onnx"), so, providers=["CPUExecutionProvider"])

    wave, file_sr = sf.read(args.audio, dtype="float32", always_2d=True)
    wave = wave.mean(axis=1)
    if file_sr != sr:
        import librosa

        wave = librosa.resample(wave, orig_sr=file_sr, target_sr=sr)

    notes: list[dict] = []
    for (c_start, c_end) in silence_chunks(wave, sr):
        chunk = wave[c_start:c_end]
        if chunk.shape[0] < sr // 20:
            continue
        x_seg, x_est, mask_t = run_encoder(enc, chunk, args.language)
        boundaries = run_segmenter(seg, x_seg, args.language, mask_t, seg_threshold, seg_radius)
        durations, mask_n = run_bd2dur(bd2dur, boundaries, mask_t)
        if durations.size == 0 or mask_n.size == 0:
            continue
        presence, scores = run_estimator(est, x_est, boundaries, mask_t, mask_n, est_threshold)

        # durations are seconds in the official bd2dur semantics; sanity-guard
        # against a frame-scaled variant the same way the Rust decoder does.
        total = float(np.sum(durations))
        chunk_dur = chunk.shape[0] / sr
        if abs(total - chunk_dur) > chunk_dur * 0.2:
            scale = timestep
            if abs(total * timestep - chunk_dur) > chunk_dur * 0.2:
                scale = chunk_dur / max(total, 1e-6)
        else:
            scale = 1.0

        t = c_start / sr
        for i in range(durations.shape[0]):
            d = float(durations[i]) * scale
            start, end = t, t + d
            t = end
            if i < presence.shape[0] and float(presence[i]) > 0.5:
                notes.append(
                    {
                        "start": round(start, 4),
                        "end": round(end, 4),
                        "midi": round(float(scores[i]), 3),
                    }
                )

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(notes, indent=1), encoding="utf-8")
    print(f"wrote {len(notes)} reference notes -> {out_path}")


if __name__ == "__main__":
    main()
