# 生成 models/test_data.json, 供 src-tauri/tests/integration.rs 做
# Rust mel / ONNX 推理 与 Python torchfcpe 参考实现的一致性校验。
#
# 数据来源: 从一首真实人声歌曲截取前 N 秒, 用 fcpe.pt 自带的 wav2mel + torch 模型
# 生成参考 mel 与 latent。文件较大 (~5MB), 已被 .gitignore 排除。
#
# 用法: D:/Agent/pitch/.venv/Scripts/python.exe scripts/gen_test_data.py \
#   --audio "岡村孝子 - ドラマ_vocals_noreverb_vocals.flac" --seconds 5 --out models/test_data.json

import argparse
import json
import pathlib
import sys

import numpy as np
import torch
import librosa

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from export_fcpe_onnx import build_model, remap_state_dict  # noqa: E402
from torchfcpe.tools import spawn_wav2mel  # noqa: E402
from torchfcpe.tools import DotDict  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--audio", required=True)
    ap.add_argument("--pt", default="models/fcpe.pt")
    ap.add_argument("--seconds", type=float, default=5.0)
    ap.add_argument("--out", default="models/test_data.json")
    args = ap.parse_args()

    wav, _ = librosa.load(args.audio, sr=16000, mono=True, duration=args.seconds)
    print(f"wav: {len(wav)} samples ({len(wav)/16000:.2f}s)")

    ckpt = torch.load(args.pt, map_location="cpu", weights_only=False)
    cfg = ckpt["config"]
    model = build_model(cfg)
    remapped = remap_state_dict(ckpt["model"])
    missing, unexpected = model.load_state_dict(remapped, strict=False)
    missing = [k for k in missing if k != "gaussian_blurred_cent_mask"]
    assert not missing and not unexpected, (missing, unexpected)
    model.eval()

    wav2mel = spawn_wav2mel(DotDict(cfg), "cpu")
    with torch.no_grad():
        audio_tensor = torch.tensor(wav, dtype=torch.float32).unsqueeze(0)
        mel = wav2mel(audio_tensor, 16000)                 # (1, T, 128)
        latent = model(mel)                                # (1, T, 360), sigmoid 已含
    print(f"mel: {tuple(mel.shape)}, latent: {tuple(latent.shape)}")

    out = {
        "wav": [round(float(x), 6) for x in wav],
        "mel": [[[round(float(x), 6) for x in frame] for frame in mel[0]]],
        "latent": [[[round(float(x), 6) for x in frame] for frame in latent[0]]],
    }
    out_path = pathlib.Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(out, f)
    print(f"saved: {out_path} ({out_path.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
