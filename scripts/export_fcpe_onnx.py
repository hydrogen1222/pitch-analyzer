# 将 FCPE 的 .pt 权重 (CNChTu/FCPE 发布格式, TransformerF0BCE) 导出为
# 本项目 Rust 端使用的 fcpe.onnx + fcpe_config.json。
#
# Rust 端约定 (decoder.rs / analyzer.rs):
#   输入: mel (1, T, 128)  → 输出: latent (1, T, 360) (已含 sigmoid)
#   解码: argmax + cent_table[argmax±4] 局部加权 → f0 = 10 * 2^(cents/1200)
#
# 权重结构与 torchfcpe 的 CFNaiveMelPE 同构, 仅顶层模块名不同:
#   stack.*              → input_stack.*
#   decoder._layers.N.*  → net.encoder_layers.N.*
#   dense_out.*          → output_proj.*
#   cent_table           → cent_table (直接使用模型自带的)
#
# 用法:
#   uv pip install --python D:/Agent/pitch/.venv/Scripts/python.exe onnx onnxruntime
#   D:/Agent/pitch/.venv/Scripts/python.exe scripts/export_fcpe_onnx.py \
#       --pt models/fcpe.pt --out-dir models

import argparse
import json
import pathlib
import sys

import numpy as np
import torch
import torchfcpe
from torchfcpe.models import CFNaiveMelPE

KEY_MAP_PREFIX = [
    ("decoder._layers.", "net.encoder_layers."),
    ("stack.", "input_stack."),
    ("dense_out.", "output_proj."),
]


def remap_state_dict(sd: dict) -> dict:
    out = {}
    for k, v in sd.items():
        nk = k
        for old, new in KEY_MAP_PREFIX:
            if nk.startswith(old):
                nk = new + nk[len(old):]
                break
        out[nk] = v
    return out


def build_model(cfg: dict) -> CFNaiveMelPE:
    m = cfg["model"]
    model = CFNaiveMelPE(
        input_channels=m["input_channel"],
        out_dims=m["out_dims"],
        hidden_dims=m["n_chans"],
        n_layers=m["n_layers"],
        f0_max=m["f0_max"],
        f0_min=m["f0_min"],
        use_fa_norm=False,
        conv_only=False,
        conv_dropout=0.0,
        atten_dropout=0.0,
        use_harmonic_emb=False,
    )
    return model


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pt", required=True)
    ap.add_argument("--out-dir", required=True)
    args = ap.parse_args()

    out_dir = pathlib.Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    ckpt = torch.load(args.pt, map_location="cpu", weights_only=False)
    cfg = ckpt["config"]
    model = build_model(cfg)

    remapped = remap_state_dict(ckpt["model"])
    missing, unexpected = model.load_state_dict(remapped, strict=False)
    # gaussian_blurred_cent_mask 是派生 buffer, 允许缺失 (会重新注册默认值)
    missing = [k for k in missing if k != "gaussian_blurred_cent_mask"]
    if missing or unexpected:
        print(f"[ERROR] state dict 不匹配 missing={missing} unexpected={unexpected}")
        sys.exit(1)
    model.eval()

    # 用模型自带的 cent_table 覆盖 (Rust 端从 fcpe_config.json 读取)
    cent_table = ckpt["model"]["cent_table"].float().numpy()
    n_bins = cent_table.shape[0]
    f_min = 10.0 * 2 ** (float(cent_table[0]) / 1200.0)
    f_max = 10.0 * 2 ** (float(cent_table[-1]) / 1200.0)
    print(f"cent_table: {n_bins} bins, f0 range {f_min:.2f} ~ {f_max:.2f} Hz")

    # sanity check
    with torch.no_grad():
        mel = torch.rand(1, 200, cfg["mel"]["num_mels"])
        latent = model(mel)
        assert latent.shape == (1, 200, n_bins), latent.shape
        assert latent.min() >= 0.0 and latent.max() <= 1.0
        f0_ref = model.infer(mel, decoder="local_argmax", threshold=0.0)
    print(f"torch sanity OK: latent {tuple(latent.shape)}, f0 range "
          f"{f0_ref[f0_ref > 0].min():.1f} ~ {f0_ref.max():.1f} Hz")

    # 导出 ONNX
    onnx_path = out_dir / "fcpe.onnx"
    wrapper = Wrapper(model)
    torch.onnx.export(
        wrapper,
        (mel,),
        str(onnx_path),
        input_names=["mel"],
        output_names=["latent"],
        dynamic_axes={"mel": {1: "T"}, "latent": {1: "T"}},
        opset_version=17,
        dynamo=False,
    )
    print(f"ONNX saved: {onnx_path} ({onnx_path.stat().st_size / 1e6:.1f} MB)")

    # 数值校验: onnxruntime vs torch
    import onnxruntime as ort
    sess = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    latent_ort = sess.run(["latent"], {"mel": mel.numpy()})[0]
    diff = np.abs(latent_ort - latent.numpy()).max()
    print(f"onnx vs torch max abs diff: {diff:.2e}")
    if diff > 1e-3:
        print("[ERROR] onnx 数值偏差过大")
        sys.exit(1)

    # f0 解码校验: 复刻 Rust decoder.rs 的 local_argmax 路径
    probs = latent_ort[0]  # (T, 360)
    argmax = probs.argmax(axis=-1)
    idx = np.clip(argmax[:, None] + np.arange(-4, 5)[None, :], 0, n_bins - 1)
    w = probs[np.arange(len(argmax))[:, None], idx]  # (T, 9)
    cents = (cent_table[idx] * w).sum(-1) / np.maximum(w.sum(-1), 1e-9)
    f0_ort = 10.0 * 2 ** (cents / 1200.0)
    conf_ort = probs.max(axis=-1)
    voiced = conf_ort > 0.0
    if voiced.any():
        print(f"decoded f0 (voiced): {f0_ort[voiced].min():.1f} ~ {f0_ort.max():.1f} Hz, "
              f"conf {conf_ort.min():.3f} ~ {conf_ort.max():.3f}")

    # 写 fcpe_config.json
    cfg_path = out_dir / "fcpe_config.json"
    with open(cfg_path, "w", encoding="utf-8") as f:
        json.dump({"cent_table": [round(float(x), 4) for x in cent_table]}, f)
    print(f"config saved: {cfg_path} ({cfg_path.stat().st_size / 1e3:.1f} KB)")


class Wrapper(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, mel):
        return self.model(mel)


if __name__ == "__main__":
    main()
