"""Deterministic mock worker speaking the Round4 legacy-FA JSONL protocol.

Used by Tier-B protocol tests (tests/forced_align_v2_test.rs) so the Rust
worker lifecycle can be verified without torch/torchaudio. It mimics the
real worker's line protocol and failure modes:

  - init counts are observable via the "inits" field of ready
  - symbol "BAD" forces a non-monotonic output -> InvalidModelOutput path
  - symbol "OOR" forces an out-of-window result -> InvalidModelOutput path
  - window_start < 0 forces an init-time error path when audio_path=="FAIL"
"""

from __future__ import annotations

import json
import sys

AUDIO_LOADS: list[str] = []


def respond(obj: dict) -> None:
    print(json.dumps(obj, ensure_ascii=False), flush=True)


def align_segment(segment: dict) -> dict:
    start = float(segment["window_start"])
    end = float(segment["window_end"])
    out_units = []
    units = segment.get("units", [])
    n = max(1, len(units))
    span = (end - start) / n
    for index, unit in enumerate(units):
        symbols = [str(s).lower() for s in unit.get("symbols", [])]
        u_start = start + span * index
        u_end = u_start + span
        if "bad" in symbols:
            # deliberate overlap with the previous unit (non-monotonic)
            u_start = u_start - span
        if "oor" in symbols:
            u_end = end + 5.0
        out_units.append(
            {
                "unit_id": unit["id"],
                "start": u_start,
                "end": u_end,
                "raw_score": 0.9,
            }
        )
    return {"segment_id": segment["id"], "units": out_units}


def main() -> None:
    state = {"audio": None, "inits": 0}
    for raw_line in sys.stdin:
        raw_line = raw_line.strip()
        if not raw_line:
            continue
        request = json.loads(raw_line)
        kind = request.get("type")
        if kind == "shutdown":
            return
        if kind == "init":
            if request.get("audio_path") == "FAIL":
                respond({"type": "error", "code": "audio_invalid", "message": "no such file"})
                continue
            state["audio"] = request.get("audio_path")
            state["inits"] += 1
            AUDIO_LOADS.append(state["audio"] or "")
            respond({"type": "ready", "model": "mock", "inits": state["inits"]})
            continue
        if kind == "align":
            if state["audio"] is None:
                respond({"type": "error", "code": "not_ready", "message": "align before init"})
                continue
            segments = [align_segment(seg) for seg in request.get("segments", [])]
            respond({"type": "result", "segments": segments})
            continue
        respond({"type": "error", "code": "invalid_request", "message": f"unknown {kind}"})


if __name__ == "__main__":
    main()
