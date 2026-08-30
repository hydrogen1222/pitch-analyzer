"""Run torchaudio's real MMS-FA CTC aligner for one LRC window.

The Rust application deliberately invokes this helper only when a Python
environment is configured.  Missing dependencies or model download errors
are reported as failures so the caller can keep the explicit MoraDP fallback.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def _error(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(2)


def _normalize_phonemes(values: list[str], previous_vowel: str | None = None) -> str:
    """Convert the project's Japanese phoneme labels to MMS uroman chars."""
    labels = set("aie nou ts rmkldghyb pwcvjzf'q x".replace(" ", ""))
    out: list[str] = []
    for raw in values:
        p = str(raw).strip()
        if not p:
            continue
        lower = p.lower()
        if lower in {"sil", "sp"}:
            continue
        if lower == ":":
            # The project represents a long-vowel mora as a duration marker.
            # MMS-FA has no duration label, so constrain it with the preceding
            # vowel (the timeline still contains a separate mora span).
            out.append(previous_vowel or "a")
            continue
        # The Japanese phoneme provider uses N for moraic nasal and cl for
        # gemination. MMS accepts the corresponding uroman character labels.
        if lower == "n" or p == "N":
            lower = "n"
        elif lower == "cl":
            lower = "t"
        for ch in lower:
            if ch in labels:
                out.append(ch)
    return "".join(out)


def main() -> None:
    try:
        request = json.load(sys.stdin)
        audio_path = Path(request["audio_path"])
        window_start = float(request["window_start"])
        window_end = float(request["window_end"])
        mora_requests = request["moras"]
    except Exception as exc:  # pragma: no cover - exercised by Rust protocol
        _error(f"invalid MMS FA request: {exc}")

    if not audio_path.is_file():
        _error(f"audio file does not exist: {audio_path}")
    if not window_end > window_start:
        _error("invalid FA window")
    if not mora_requests:
        _error("no mora supplied")

    try:
        import torch
        import torchaudio
    except Exception as exc:  # pragma: no cover - environment dependent
        _error(f"torchaudio MMS-FA is unavailable: {exc}")

    try:
        bundle = torchaudio.pipelines.MMS_FA
        waveform, sample_rate = torchaudio.load(str(audio_path))
        waveform = waveform.mean(dim=0, keepdim=True)
        if sample_rate != bundle.sample_rate:
            waveform = torchaudio.functional.resample(
                waveform, sample_rate, bundle.sample_rate
            )
            sample_rate = bundle.sample_rate

        start_sample = max(0, int(window_start * sample_rate))
        end_sample = min(waveform.shape[-1], int(window_end * sample_rate))
        if end_sample <= start_sample:
            _error("FA window has no audio samples")
        cropped = waveform[:, start_sample:end_sample]

        transcripts: list[str] = []
        previous_vowel: str | None = None
        for item in mora_requests:
            text = _normalize_phonemes(item.get("phonemes", []), previous_vowel)
            if not text:
                _error(f"mora has no MMS-compatible phoneme: {item.get('kana', '')}")
            transcripts.append(text)
            for char in reversed(text):
                if char in "aeiou":
                    previous_vowel = char
                    break

        model = bundle.get_model().eval()
        tokenizer = bundle.get_tokenizer()
        aligner = bundle.get_aligner()
        with torch.inference_mode():
            emission, _ = model(cropped)
        # MMS_FA's model output is already log-probability and has a batch dim.
        emission = emission[0]
        token_ids = tokenizer(transcripts)
        spans_by_mora = aligner(emission, token_ids)
        seconds_per_frame = (cropped.shape[-1] / sample_rate) / max(
            1, emission.shape[0]
        )

        aligned = []
        for index, spans in enumerate(spans_by_mora):
            if not spans:
                _error(f"MMS FA could not align mora {index}")
            start = min(span.start for span in spans)
            end = max(span.end for span in spans)
            confidence = sum(float(span.score) for span in spans) / len(spans)
            aligned.append(
                {
                    "mora_index": index,
                    "mora": mora_requests[index].get("kana", ""),
                    "start": window_start + start * seconds_per_frame,
                    "end": window_start + end * seconds_per_frame,
                    "confidence": max(0.0, min(1.0, confidence)),
                }
            )
    except SystemExit:
        raise
    except Exception as exc:  # pragma: no cover - model/runtime dependent
        _error(f"MMS FA inference failed: {exc}")

    print(
        json.dumps(
            {
                "source": "ForcedAlign",
                "model": "torchaudio.MMS_FA",
                "moras": aligned,
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
