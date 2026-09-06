"""Persistent legacy MMS-FA worker (Round4 任务书 P0-4 / §10.4).

Long-lived JSONL worker: the model is loaded ONCE at init and the audio is
decoded/resampled ONCE per song. Each subsequent alignment is a single
request/response line pair — the old spawn-per-line helper
(forced_align_mms.py) is kept only as reference for the one-shot protocol.

Protocol (one JSON object per line):

  -> {"type": "init", "audio_path": "..."}
  <- {"type": "ready", "model": "torchaudio.MMS_FA"}
  <- {"type": "error", "code": "model_load_failed", "message": "..."}

  -> {"type": "align", "segments": [
         {"id": 0, "window_start": 1.0, "window_end": 4.0,
          "units": [{"id": 0, "symbols": ["k", "y", "o"]}, ...]}, ...]}
  <- {"type": "result", "segments": [
         {"segment_id": 0, "units": [{"unit_id": 0, "start": .., "end": ..,
                                      "raw_score": ..}, ...]}, ...]}

  -> {"type": "shutdown"}  (worker exits)
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

MODEL_NAME = "torchaudio.MMS_FA"


def _error_line(code: str, message: str) -> None:
    print(
        json.dumps({"type": "error", "code": code, "message": message}, ensure_ascii=False),
        flush=True,
    )


def _normalize_phonemes(values: list[str], previous_vowel: str | None) -> tuple[str, str | None]:
    """Convert the project's Japanese/Chinese phoneme labels to MMS uroman chars.

    Legacy compatibility mapping (Round4 §31): cl -> t, ':' -> previous vowel,
    N -> n. This is a hack retained for the LEGACY backend only; the singing
    FA backend (planned) receives real phoneme labels.
    """
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
            out.append(previous_vowel or "a")
            continue
        if lower == "n" or p == "N":
            lower = "n"
        elif lower == "cl":
            lower = "t"
        for ch in lower:
            if ch in labels:
                out.append(ch)
    vowel: str | None = None
    for char in reversed(out):
        if char in "aeiou":
            vowel = char
            break
    return "".join(out), vowel


class MmsState:
    """Model + audio loaded once for the whole song."""

    def __init__(self, audio_path: str) -> None:
        import torch
        import torchaudio

        self.torch = torch
        bundle = torchaudio.pipelines.MMS_FA
        self.sample_rate = bundle.sample_rate
        self.model = bundle.get_model().eval()
        self.tokenizer = bundle.get_tokenizer()
        self.aligner = bundle.get_aligner()

        path = Path(audio_path)
        if not path.is_file():
            raise FileNotFoundError(f"audio file does not exist: {path}")
        try:
            waveform, sample_rate = torchaudio.load(str(path))
        except Exception:
            # torchaudio >= 2.9 delegates file IO to torchcodec which is often
            # unavailable on CPU-only envs; soundfile reads the same formats.
            import soundfile as sf_module

            data, sample_rate = sf_module.read(str(path), dtype="float32", always_2d=True)
            waveform = torch.from_numpy(data.T.copy())
        waveform = waveform.mean(dim=0, keepdim=True)
        if sample_rate != self.sample_rate:
            waveform = torchaudio.functional.resample(waveform, sample_rate, self.sample_rate)
        self.waveform = waveform

    def align_segment(self, segment: dict) -> dict:
        torch = self.torch
        window_start = float(segment["window_start"])
        window_end = float(segment["window_end"])
        if not window_end > window_start:
            raise ValueError("invalid window")
        units = segment.get("units", [])
        if not units:
            raise ValueError("segment has no units")

        start_sample = max(0, int(window_start * self.sample_rate))
        end_sample = min(self.waveform.shape[-1], int(window_end * self.sample_rate))
        if end_sample <= start_sample:
            raise ValueError("window has no audio samples")
        cropped = self.waveform[:, start_sample:end_sample]

        transcripts: list[str] = []
        previous_vowel: str | None = None
        for unit in units:
            text, previous_vowel = _normalize_phonemes(unit.get("symbols", []), previous_vowel)
            if not text:
                raise ValueError(f"unit {unit.get('id')} has no MMS-compatible phoneme")
            transcripts.append(text)

        with torch.inference_mode():
            emission, _ = self.model(cropped)
        emission = emission[0]
        token_ids = self.tokenizer(transcripts)
        spans_by_unit = self.aligner(emission, token_ids)
        seconds_per_frame = (cropped.shape[-1] / self.sample_rate) / max(1, emission.shape[0])

        aligned_units = []
        for index, spans in enumerate(spans_by_unit):
            if not spans:
                raise ValueError(f"could not align unit {index}")
            start = min(span.start for span in spans)
            end = max(span.end for span in spans)
            score = sum(float(span.score) for span in spans) / len(spans)
            aligned_units.append(
                {
                    "unit_id": units[index]["id"],
                    "start": window_start + start * seconds_per_frame,
                    "end": window_start + end * seconds_per_frame,
                    "raw_score": max(0.0, min(1.0, score)),
                }
            )
        return {"segment_id": segment["id"], "units": aligned_units}


def main() -> None:
    state: MmsState | None = None
    for raw_line in sys.stdin:
        raw_line = raw_line.strip()
        if not raw_line:
            continue
        try:
            request = json.loads(raw_line)
        except Exception as exc:
            _error_line("invalid_request", str(exc))
            continue

        kind = request.get("type")
        if kind == "shutdown":
            return
        if kind == "init":
            try:
                state = MmsState(request["audio_path"])
            except ImportError as exc:
                _error_line("dependency_missing", str(exc))
                return
            except FileNotFoundError as exc:
                _error_line("audio_invalid", str(exc))
                return
            except Exception as exc:
                _error_line("model_load_failed", str(exc))
                return
            print(
                json.dumps({"type": "ready", "model": MODEL_NAME}, ensure_ascii=False),
                flush=True,
            )
            continue
        if kind == "align":
            if state is None:
                _error_line("not_ready", "worker received align before init")
                continue
            try:
                segments = [state.align_segment(seg) for seg in request.get("segments", [])]
            except ValueError as exc:
                _error_line("invalid_request", str(exc))
                continue
            except Exception as exc:
                _error_line("inference_failed", str(exc))
                continue
            print(
                json.dumps({"type": "result", "segments": segments}, ensure_ascii=False),
                flush=True,
            )
            continue
        _error_line("invalid_request", f"unknown request type: {kind}")


if __name__ == "__main__":
    main()
