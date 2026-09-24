"""Validate and analyze versioned ink benchmark JSONL records.

The module intentionally only uses the Python standard library. Internal
timestamps measure the app pipeline; pencil-to-pixel is accepted only from an
explicit external camera measurement record.
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


SCHEMA_VERSION = "ink-bench/v1"
VALID_RECORD_TYPES = {"run", "sample", "frame", "memory", "external_latency"}
VALID_REFRESH_HZ = {60, 120}
STABLE_PSS_LIMIT_KB = 250 * 1024
PEAK_PSS_LIMIT_KB = 350 * 1024
THRESHOLDS = {
    "interactive_work_p95_ms": 6.0,
    "frame_p95_ms": 8.33,
    "frame_p99_ms": 16.67,
    "frame_loss_rate_lt": 0.01,
    "stylus_to_frame_p95_ms": 8.33,
    "pencil_to_pixel_p95_ms": 25.0,
    "stable_pss_mb": 250.0,
    "peak_pss_mb": 350.0,
}


class AnalysisError(ValueError):
    """Input is not a complete, valid benchmark JSONL stream."""


def percentile(values: Sequence[float] | Iterable[float], p: float = 50) -> float:
    """Return a deterministic linear-interpolated percentile.

    The definition is the nearest-rank-compatible ``(n - 1) * p`` position,
    with interpolation between adjacent sorted values. Results are rounded to
    six decimals so JSON output is stable across Python versions.
    """

    ordered = sorted(float(value) for value in values)
    if not ordered:
        raise ValueError("percentile requires at least one value")
    if not 0 <= p <= 100:
        raise ValueError("percentile p must be between 0 and 100")
    position = (len(ordered) - 1) * p / 100
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        result = ordered[lower]
    else:
        fraction = position - lower
        result = ordered[lower] + (ordered[upper] - ordered[lower]) * fraction
    return round(result, 6)


def _require(record: Mapping[str, Any], fields: Iterable[str], context: str) -> None:
    missing = [field for field in fields if field not in record]
    if missing:
        raise AnalysisError(f"{context}: missing required fields: {', '.join(missing)}")


def _number(value: Any, field: str, *, integer: bool = False) -> float | int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise AnalysisError(f"{field}: expected a number")
    if not math.isfinite(value):
        raise AnalysisError(f"{field}: expected a finite number")
    if integer and not isinstance(value, int):
        raise AnalysisError(f"{field}: expected an integer")
    return value


def _boolean(value: Any, field: str) -> bool:
    if not isinstance(value, bool):
        raise AnalysisError(f"{field}: expected a boolean")
    return value


def _string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise AnalysisError(f"{field}: expected a non-empty string")
    return value


def _sha256(value: Any, field: str) -> None:
    if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise AnalysisError(f"{field}: expected a lowercase SHA-256 hex digest")


def _validate_run(record: Mapping[str, Any], *, input_sha256: str | None = None, input_name: str | None = None) -> None:
    _require(
        record,
        (
            "schema_version", "record_type", "run_id", "created_at", "device",
            "display", "thermal", "warmup_samples", "engines", "corpus",
            "order", "raw_files",
        ),
        "run",
    )
    if not isinstance(record["run_id"], str) or not record["run_id"]:
        raise AnalysisError("run.run_id: expected a non-empty string")
    if not isinstance(record["device"], Mapping):
        raise AnalysisError("run.device: expected an object")
    _require(record["device"], ("model", "android_version", "build"), "run.device")
    if not isinstance(record["display"], Mapping):
        raise AnalysisError("run.display: expected an object")
    _require(record["display"], ("width_px", "height_px", "requested_hz", "effective_hz"), "run.display")
    for field in ("width_px", "height_px", "requested_hz", "effective_hz"):
        _number(record["display"][field], f"run.display.{field}")
        if record["display"][field] <= 0:
            raise AnalysisError(f"run.display.{field}: expected a positive number")
    if not isinstance(record["thermal"], Mapping):
        raise AnalysisError("run.thermal: expected an object")
    _require(record["thermal"], ("initial_c", "final_c"), "run.thermal")
    _number(record["thermal"]["initial_c"], "run.thermal.initial_c")
    _number(record["thermal"]["final_c"], "run.thermal.final_c")
    warmup = _number(record["warmup_samples"], "run.warmup_samples", integer=True)
    if warmup < 0:
        raise AnalysisError("run.warmup_samples: expected a non-negative integer")
    if not isinstance(record["engines"], list) or len(record["engines"]) != 1:
        raise AnalysisError("run.engines: exactly one engine is required per JSONL run")
    for index, engine in enumerate(record["engines"]):
        if not isinstance(engine, Mapping):
            raise AnalysisError(f"run.engines[{index}]: expected an object")
        _require(engine, ("id", "version"), f"run.engines[{index}]")
    if not isinstance(record["corpus"], list) or len(record["corpus"]) != 1:
        raise AnalysisError("run.corpus: exactly one corpus is required per JSONL run")
    for index, corpus in enumerate(record["corpus"]):
        if not isinstance(corpus, Mapping):
            raise AnalysisError(f"run.corpus[{index}]: expected an object")
        _require(corpus, ("id", "files"), f"run.corpus[{index}]")
        if not isinstance(corpus["files"], list) or not corpus["files"]:
            raise AnalysisError(f"run.corpus[{index}].files: expected a non-empty list")
        for file_index, raw_file in enumerate(corpus["files"]):
            if not isinstance(raw_file, Mapping):
                raise AnalysisError(f"run.corpus[{index}].files[{file_index}]: expected an object")
            _require(raw_file, ("path", "sha256"), f"run.corpus[{index}].files[{file_index}]")
            _sha256(raw_file["sha256"], f"run.corpus[{index}].files[{file_index}].sha256")
    if not isinstance(record["order"], list) or len(record["order"]) != 1:
        raise AnalysisError("run.order: exactly one pre-registered cell is required per JSONL run")
    if not isinstance(record["raw_files"], list):
        raise AnalysisError("run.raw_files: expected a list of external artifacts")
    for index, raw_file in enumerate(record["raw_files"]):
        if not isinstance(raw_file, Mapping):
            raise AnalysisError(f"run.raw_files[{index}]: expected an object")
        _require(raw_file, ("path", "sha256"), f"run.raw_files[{index}]")
        if "kind" in raw_file:
            _string(raw_file["kind"], f"run.raw_files[{index}].kind")
        _sha256(raw_file["sha256"], f"run.raw_files[{index}].sha256")
        if input_sha256 is not None and raw_file["sha256"] == input_sha256:
            raise AnalysisError("run.raw_files: input JSONL cannot hash itself")
        if input_name is not None and Path(str(raw_file["path"])).name == Path(input_name).name:
            raise AnalysisError("run.raw_files: input JSONL must not be listed as an external artifact")


def _validate_records(
    records: Sequence[Mapping[str, Any]], *, input_sha256: str | None = None, input_name: str | None = None
) -> Mapping[str, Any]:
    if not records:
        raise AnalysisError("JSONL stream is empty")
    for index, record in enumerate(records):
        if not isinstance(record, Mapping):
            raise AnalysisError(f"line {index + 1}: expected a JSON object")
        if record.get("schema_version") != SCHEMA_VERSION:
            raise AnalysisError(f"line {index + 1}: unsupported schema_version")
        if record.get("record_type") not in VALID_RECORD_TYPES:
            raise AnalysisError(f"line {index + 1}: unsupported record_type")
    runs = [record for record in records if record["record_type"] == "run"]
    if len(runs) != 1 or records[0] is not runs[0]:
        raise AnalysisError("stream must contain exactly one run record as its first line")
    run = runs[0]
    _validate_run(run, input_sha256=input_sha256, input_name=input_name)
    samples = [record for record in records if record["record_type"] == "sample"]
    frames = [record for record in records if record["record_type"] == "frame"]
    memories = [record for record in records if record["record_type"] == "memory"]
    external = [record for record in records if record["record_type"] == "external_latency"]
    if not samples:
        raise AnalysisError("stream has no sample records")
    if not frames:
        raise AnalysisError("stream has no frame records")
    if not memories:
        raise AnalysisError("stream has no memory records")
    _validate_samples(samples)
    _validate_frames(frames)
    _validate_memory(memories)
    _validate_external(external)
    return run


def _validate_samples(samples: Sequence[Mapping[str, Any]]) -> None:
    ids: set[int] = set()
    fields = (
        "sample_id", "stroke_id", "event_ts_ns", "dispatch_ts_ns", "model_start_ts_ns", "model_end_ts_ns",
        "geometry_start_ts_ns", "geometry_end_ts_ns", "submit_ts_ns", "swap_start_ts_ns",
        "swap_end_ts_ns", "prediction_used",
    )
    for index, sample in enumerate(samples):
        context = f"sample[{index}]"
        _require(sample, fields, context)
        sample_id = _number(sample["sample_id"], f"{context}.sample_id", integer=True)
        if sample_id in ids:
            raise AnalysisError(f"{context}.sample_id: duplicate id")
        ids.add(sample_id)
        _string(sample["stroke_id"], f"{context}.stroke_id")
        timestamps = []
        for field in fields[2:-1]:
            timestamps.append(_number(sample[field], f"{context}.{field}", integer=True))
        if any(left > right for left, right in zip(timestamps, timestamps[1:])):
            raise AnalysisError(f"{context}: timestamps must be non-decreasing")
        _boolean(sample["prediction_used"], f"{context}.prediction_used")


def _validate_frames(frames: Sequence[Mapping[str, Any]]) -> None:
    ids: set[int] = set()
    vsync_slots: set[int] = set()
    fields = ("frame_id", "vsync_ts_ns", "submit_ts_ns", "present_ts_ns", "presented")
    for index, frame in enumerate(frames):
        context = f"frame[{index}]"
        _require(frame, fields, context)
        frame_id = _number(frame["frame_id"], f"{context}.frame_id", integer=True)
        if frame_id in ids:
            raise AnalysisError(f"{context}.frame_id: duplicate id")
        ids.add(frame_id)
        vsync = _number(frame["vsync_ts_ns"], f"{context}.vsync_ts_ns", integer=True)
        if vsync in vsync_slots:
            raise AnalysisError(f"{context}.vsync_ts_ns: duplicate slot")
        vsync_slots.add(vsync)
        submit = _number(frame["submit_ts_ns"], f"{context}.submit_ts_ns", integer=True)
        present = _number(frame["present_ts_ns"], f"{context}.present_ts_ns", integer=True)
        if not vsync <= submit <= present:
            raise AnalysisError(f"{context}: expected vsync <= submit <= present")
        _boolean(frame["presented"], f"{context}.presented")


def _validate_memory(memories: Sequence[Mapping[str, Any]]) -> None:
    fields = ("sample_id", "ts_ns", "pss_kb", "phase")
    for index, memory in enumerate(memories):
        context = f"memory[{index}]"
        _require(memory, fields, context)
        _number(memory["sample_id"], f"{context}.sample_id", integer=True)
        _number(memory["ts_ns"], f"{context}.ts_ns", integer=True)
        pss = _number(memory["pss_kb"], f"{context}.pss_kb")
        if pss < 0:
            raise AnalysisError(f"{context}.pss_kb: expected non-negative value")
        if memory["phase"] not in {"warmup", "stable", "stress"}:
            raise AnalysisError(f"{context}.phase: expected warmup, stable or stress")


def _validate_external(external: Sequence[Mapping[str, Any]]) -> None:
    for index, measurement in enumerate(external):
        context = f"external_latency[{index}]"
        _require(measurement, ("latency_ms", "fps", "uncertainty_ms"), context)
        latency = _number(measurement["latency_ms"], f"{context}.latency_ms")
        fps = _number(measurement["fps"], f"{context}.fps")
        uncertainty = _number(measurement["uncertainty_ms"], f"{context}.uncertainty_ms")
        if latency < 0 or fps <= 0 or uncertainty < 0:
            raise AnalysisError(f"{context}: latency/fps/uncertainty out of range")


def _series(records: Sequence[Mapping[str, Any]], field: str) -> list[float]:
    return [round((float(record[field[1]]) - float(record[field[0]])) / 1_000_000, 6) for record in records]


def _summary(values: Sequence[float]) -> dict[str, Any]:
    return {
        "count": len(values),
        "p50": percentile(values, 50),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
        "min": round(min(values), 6),
        "max": round(max(values), 6),
    }


def _optional_summary(values: Sequence[float]) -> dict[str, Any]:
    if not values:
        return {"count": 0, "p50": None, "p95": None, "p99": None, "min": None, "max": None}
    return _summary(values)


def _frame_intervals(frames: Sequence[Mapping[str, Any]]) -> list[float]:
    ordered = sorted(
        (frame for frame in frames if frame["presented"]),
        key=lambda frame: frame["present_ts_ns"],
    )
    return [
        round((current["present_ts_ns"] - previous["present_ts_ns"]) / 1_000_000, 6)
        for previous, current in zip(ordered, ordered[1:])
    ]


def _frame_loss_rate(frames: Sequence[Mapping[str, Any]], effective_hz: float) -> float:
    period_ns = 1_000_000_000 / effective_hz
    ordered = sorted(frames, key=lambda frame: frame["vsync_ts_ns"])
    holes = 0
    for previous, current in zip(ordered, ordered[1:]):
        gap = current["vsync_ts_ns"] - previous["vsync_ts_ns"]
        slots = int(round(gap / period_ns))
        holes += max(0, slots - 1)
    explicit_lost = sum(not frame["presented"] for frame in frames)
    return round((explicit_lost + holes) / (len(frames) + holes), 6)


def _memory_summary(memories: Sequence[Mapping[str, Any]], warmup_samples: int) -> dict[str, Any]:
    ordered_memories = sorted(memories, key=lambda memory: memory["ts_ns"])
    stable = [float(memory["pss_kb"]) for memory in ordered_memories if memory["phase"] == "stable"]
    stress = [float(memory["pss_kb"]) for memory in memories if memory["phase"] == "stress"]
    eligible = stable or [float(memory["pss_kb"]) for memory in memories if memory["phase"] != "warmup"]
    if not eligible:
        return {
            "stable_status": "missing",
            "stable_pss_kb": None,
            "peak_pss_kb": max(float(m["pss_kb"]) for m in memories),
            "stable_samples": len(stable),
            "stress_samples": len(stress),
            "warmup_samples": warmup_samples,
        }
    growth = len(eligible) >= 3 and all(right > left for left, right in zip(eligible, eligible[1:]))
    stable_status = "growing" if growth else "stable"
    peak_values = stress or [float(memory["pss_kb"]) for memory in memories if memory["phase"] != "warmup"]
    return {
        "stable_status": stable_status,
        "stable_pss_kb": round(percentile(eligible, 50), 6),
        "stable_p95_pss_kb": round(percentile(eligible, 95), 6),
        "peak_pss_kb": round(max(peak_values), 6),
        "warmup_samples": warmup_samples,
        "stable_samples": len(stable),
        "stress_samples": len(stress),
    }


def _external_summary(external: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    if not external:
        return {"status": "no_verificado", "reason": "missing_external_measurement"}
    if len(external) < 30:
        return {
            "status": "no_verificado",
            "reason": "external_observations_below_30",
            "observation_count": len(external),
        }
    fps_values = [float(item["fps"]) for item in external]
    uncertainty_values = [float(item["uncertainty_ms"]) for item in external]
    latencies = [float(item["latency_ms"]) for item in external]
    fps = min(fps_values)
    uncertainty = max(uncertainty_values)
    p95 = percentile(latencies, 95)
    result: dict[str, Any] = {
        "status": "aprobado" if fps >= 240 and p95 + uncertainty <= 25 else "rechazado",
        "fps": fps,
        "uncertainty_ms": uncertainty,
        "latency_ms": _summary(latencies),
        "upper_bound_p95_ms": round(p95 + uncertainty, 6),
        "observation_count": len(external),
    }
    if fps < 240:
        result["status"] = "no_verificado"
        result["reason"] = "camera_fps_below_240"
    return result


def _assessment(run: Mapping[str, Any], metrics: Mapping[str, Any], memory: Mapping[str, Any], quality: Mapping[str, Any], external: Mapping[str, Any]) -> dict[str, Any]:
    reasons: list[str] = []
    failures: list[str] = []
    effective_hz = run["display"]["effective_hz"]
    if effective_hz not in VALID_REFRESH_HZ:
        reasons.append("effective_hz")
    elif effective_hz == 60:
        reasons.append("120_hz_not_measured")
    if quality["prediction_used"]:
        reasons.append("prediction_used")
    if quality["sample_count"] < 1000:
        reasons.append("samples_below_1000")
    if quality["stroke_count"] < 60:
        reasons.append("strokes_below_60")
    if quality["frame_count"] < 120:
        reasons.append("frame_samples_below_120")
    if memory["stable_status"] == "growing":
        reasons.append("memory_growing")
    if memory["stable_status"] == "missing":
        reasons.append("stable_memory_missing")
    if memory["stable_samples"] < 3:
        reasons.append("stable_memory_below_3")
    if memory["stress_samples"] < 1:
        reasons.append("stress_memory_missing")
    if external["status"] == "no_verificado":
        reasons.append(external.get("reason", "external_latency"))
    if external["status"] == "aprobado" and not any(
        artifact.get("kind") == "external_camera" for artifact in run["raw_files"]
    ):
        reasons.append("external_camera_artifact_missing")
    if metrics["app_work_ms"]["p95"] > THRESHOLDS["interactive_work_p95_ms"]:
        failures.append("app_work_p95_ms")
    if effective_hz == 120 and metrics["event_to_submit_ms"]["p95"] > THRESHOLDS["stylus_to_frame_p95_ms"]:
        failures.append("stylus_to_frame_p95_ms")
    if effective_hz == 120 and metrics["frame_interval_ms"]["p95"] is not None and metrics["frame_interval_ms"]["p95"] > THRESHOLDS["frame_p95_ms"]:
        failures.append("frame_p95_ms")
    if effective_hz == 120 and metrics["frame_interval_ms"]["p99"] is not None and metrics["frame_interval_ms"]["p99"] > THRESHOLDS["frame_p99_ms"]:
        failures.append("frame_p99_ms")
    if metrics["frame_loss_rate"] >= THRESHOLDS["frame_loss_rate_lt"]:
        failures.append("frame_loss_rate")
    if memory.get("stable_p95_pss_kb") is not None and memory["stable_p95_pss_kb"] > STABLE_PSS_LIMIT_KB:
        failures.append("stable_pss_kb")
    if memory["peak_pss_kb"] > PEAK_PSS_LIMIT_KB:
        failures.append("peak_pss_kb")
    if external["status"] == "rechazado":
        failures.append("external_latency_p95_ms")
    if failures:
        status = "rechazado"
    elif reasons:
        status = "inconcluso" if "memory_growing" in reasons else "no_verificado"
    else:
        status = "aprobado"
    return {"status": status, "reasons": reasons, "failures": failures}


def analyze_records(
    records: Sequence[Mapping[str, Any]], *, input_sha256: str | None = None, input_name: str | None = None
) -> dict[str, Any]:
    """Validate records and return a stable JSON-compatible analysis report."""

    run = _validate_records(records, input_sha256=input_sha256, input_name=input_name)
    samples = [record for record in records if record["record_type"] == "sample"]
    frames = [record for record in records if record["record_type"] == "frame"]
    memories = [record for record in records if record["record_type"] == "memory"]
    external = [record for record in records if record["record_type"] == "external_latency"]
    metrics = {
        "event_to_dispatch_ms": _summary(_series(samples, ("event_ts_ns", "dispatch_ts_ns"))),
        "model_ms": _summary(_series(samples, ("model_start_ts_ns", "model_end_ts_ns"))),
        "geometry_ms": _summary(_series(samples, ("geometry_start_ts_ns", "geometry_end_ts_ns"))),
        "event_to_submit_ms": _summary(_series(samples, ("event_ts_ns", "submit_ts_ns"))),
        "app_work_ms": _summary(_series(samples, ("dispatch_ts_ns", "submit_ts_ns"))),
        "swap_ms": _summary(_series(samples, ("swap_start_ts_ns", "swap_end_ts_ns"))),
        "presentation_latency_ms": _optional_summary(_series(
            [frame for frame in frames if frame["presented"]],
            ("submit_ts_ns", "present_ts_ns"),
        )),
        "frame_interval_ms": _optional_summary(_frame_intervals(frames)),
        "frame_loss_rate": _frame_loss_rate(frames, float(run["display"]["effective_hz"])),
    }
    quality = {
        "prediction_used": any(sample["prediction_used"] for sample in samples),
        "sample_count": len(samples),
        "stroke_count": len({sample["stroke_id"] for sample in samples}),
        "frame_count": len(frames),
    }
    memory = _memory_summary(memories, run["warmup_samples"])
    external_result = _external_summary(external)
    assessment = _assessment(run, metrics, memory, quality, external_result)
    result = {
        "schema_version": SCHEMA_VERSION,
        "run_id": run["run_id"],
        "comparison_context": {
            "device": run["device"],
            "display": run["display"],
            "corpus": run["corpus"],
        },
        "metrics": metrics,
        "memory": memory,
        "quality": quality,
        "external_latency": external_result,
        "assessment": assessment,
        "thresholds": THRESHOLDS,
        "comparison": {"status": "inconcluso", "reason": "single_run_has_no_comparison_baseline"},
    }
    if input_sha256 is not None:
        result["input_sha256"] = input_sha256
    return result


def analyze_file(path: str | Path) -> dict[str, Any]:
    """Load a UTF-8 JSONL file and analyze it."""

    records: list[Mapping[str, Any]] = []
    input_path = Path(path)
    try:
        raw_bytes = input_path.read_bytes()
        input_sha256 = hashlib.sha256(raw_bytes).hexdigest()
        for line_number, line in enumerate(raw_bytes.decode("utf-8").splitlines(), 1):
            if not line.strip():
                continue
            try:
                value = json.loads(line)
            except json.JSONDecodeError as error:
                raise AnalysisError(f"line {line_number}: invalid JSON: {error.msg}") from error
            if not isinstance(value, Mapping):
                raise AnalysisError(f"line {line_number}: expected a JSON object")
            records.append(value)
    except (OSError, UnicodeDecodeError) as error:
        raise AnalysisError(f"cannot read {path}: {error}") from error
    return analyze_records(records, input_sha256=input_sha256, input_name=input_path.name)


def compare_results(results: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """Compare reports without inventing a winner for incomplete/tied data."""

    if not results:
        return {"status": "inconcluso", "reason": "no_results"}
    approved = [
        result for result in results
        if result.get("assessment", {}).get("status") == "aprobado"
        and result.get("external_latency", {}).get("status") == "aprobado"
    ]
    if len(approved) != len(results):
        return {"status": "inconcluso", "reason": "only_fully_approved_runs_are_comparable"}
    if any(result.get("comparison_context") != approved[0].get("comparison_context") for result in approved[1:]):
        return {"status": "inconcluso", "reason": "comparison_context_mismatch"}
    ordered = sorted(
        approved,
        key=lambda result: (
            result["external_latency"]["latency_ms"]["p95"],
            result["metrics"]["event_to_submit_ms"]["p95"],
            result["metrics"]["app_work_ms"]["p95"],
            result.get("run_id", ""),
        ),
    )
    best = ordered[0]
    second = ordered[1] if len(ordered) > 1 else None
    if second is not None:
        first_delta = second["external_latency"]["latency_ms"]["p95"] - best["external_latency"]["latency_ms"]["p95"]
        if first_delta < 1.0:
            return {"status": "inconcluso", "reason": "external_p95_difference_below_1_ms"}
    return {"status": "ganador", "run_id": best["run_id"]}
