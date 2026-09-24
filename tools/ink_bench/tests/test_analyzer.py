import json
import hashlib
import tempfile
import unittest
from pathlib import Path

from ink_bench.analyzer import AnalysisError, analyze_file, compare_results, percentile


def record_run(*, effective_hz=120, warmup_samples=0):
    return {
        "schema_version": "ink-bench/v1",
        "record_type": "run",
        "run_id": "fixture-run",
        "created_at": "2026-09-23T10:00:00Z",
        "device": {"model": "fixture", "android_version": "fixture", "build": "fixture"},
        "display": {
            "width_px": 1440,
            "height_px": 2200,
            "requested_hz": 120,
            "effective_hz": effective_hz,
        },
        "thermal": {"initial_c": 30.0, "final_c": 31.0},
        "warmup_samples": warmup_samples,
        "engines": [{"id": "engine-a", "version": "fixture"}],
        "corpus": [{"id": "corpus-a", "files": [{"path": "fixture.pdf", "sha256": "a" * 64}]}],
        "order": ["engine-a/corpus-a"],
        "raw_files": [{"path": "fixture.jsonl", "sha256": "b" * 64}],
    }


def sample(sample_id, *, duration_ms=4.0, predicted=False):
    event = sample_id * 1_000_000
    dispatch = event + 1_000_000
    model_start = dispatch
    model_end = model_start
    geometry_start = model_end
    geometry_end = geometry_start
    submit = event + int(duration_ms * 1_000_000)
    swap_start = submit
    swap_end = swap_start + 500_000
    return {
        "schema_version": "ink-bench/v1",
        "record_type": "sample",
        "sample_id": sample_id,
        "stroke_id": f"stroke-{(sample_id - 1) % 60}",
        "event_ts_ns": event,
        "dispatch_ts_ns": dispatch,
        "model_start_ts_ns": model_start,
        "model_end_ts_ns": model_end,
        "geometry_start_ts_ns": geometry_start,
        "geometry_end_ts_ns": geometry_end,
        "submit_ts_ns": submit,
        "swap_start_ts_ns": swap_start,
        "swap_end_ts_ns": swap_end,
        "prediction_used": predicted,
    }


def frame(frame_id, *, presented=True, duration_ms=4.0, interval_ms=8.3):
    vsync = int(frame_id * interval_ms * 1_000_000)
    return {
        "schema_version": "ink-bench/v1",
        "record_type": "frame",
        "frame_id": frame_id,
        "vsync_ts_ns": vsync,
        "submit_ts_ns": vsync,
        "present_ts_ns": vsync + int(duration_ms * 1_000_000),
        "presented": presented,
    }


def memory(sample_id, pss_kb, phase="stable"):
    return {
        "schema_version": "ink-bench/v1",
        "record_type": "memory",
        "sample_id": sample_id,
        "ts_ns": sample_id * 10_000_000,
        "pss_kb": pss_kb,
        "phase": phase,
    }


def write_jsonl(records):
    temp = tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False)
    with temp:
        for record in records:
            temp.write(json.dumps(record) + "\n")
    return Path(temp.name)


def external_measurements(count=30, *, latency_ms=20.0, fps=240, uncertainty_ms=1.0):
    return [{
        "schema_version": "ink-bench/v1",
        "record_type": "external_latency",
        "observation_id": index,
        "latency_ms": latency_ms,
        "fps": fps,
        "uncertainty_ms": uncertainty_ms,
    } for index in range(count)]


def complete_records(*, effective_hz=120, sample_duration_ms=4.0, frame_duration_ms=4.0):
    records = [record_run(effective_hz=effective_hz)]
    records.extend(sample(i, duration_ms=sample_duration_ms) for i in range(1, 3))
    records.extend(frame(i, duration_ms=frame_duration_ms, interval_ms=8.3) for i in range(1, 3))
    records.extend(memory(i, 100_000) for i in range(1, 3))
    records.extend(external_measurements())
    return records


def approval_records():
    records = [record_run()]
    records[0]["raw_files"] = [{"kind": "external_camera", "path": "camera.mp4", "sha256": "c" * 64}]
    records.extend(sample(i) for i in range(1, 1001))
    records.extend(frame(i, interval_ms=8.3) for i in range(1, 121))
    records.extend(memory(i, 100_000, phase="stable") for i in range(1, 4))
    records.append(memory(4, 120_000, phase="stress"))
    records.extend(external_measurements())
    return records


class AnalyzerTests(unittest.TestCase):
    def test_percentile_is_deterministic_and_interpolated(self):
        self.assertEqual(percentile([1, 2, 3, 4]), 2.5)
        self.assertEqual(percentile([1, 2, 3, 4], 95), 3.85)
        self.assertEqual(percentile([1, 2, 3, 4], 99), 3.97)

    def test_analyzes_latency_components_and_frame_loss(self):
        records = [record_run()]
        records.extend(sample(i, duration_ms=float(i)) for i in range(1, 5))
        records.extend(frame(i, presented=i != 4) for i in range(1, 5))
        records.extend(memory(i, 200_000) for i in range(1, 4))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["metrics"]["event_to_dispatch_ms"]["p50"], 1.0)
        self.assertEqual(result["metrics"]["event_to_submit_ms"]["p95"], 3.85)
        self.assertEqual(result["metrics"]["frame_loss_rate"], 0.25)
        self.assertEqual(result["memory"]["stable_pss_kb"], 200_000)
        self.assertEqual(result["memory"]["peak_pss_kb"], 200_000)

    def test_incorrect_effective_refresh_is_not_approved(self):
        records = [record_run(effective_hz=90)]
        records.extend(sample(i) for i in range(1, 3))
        records.extend(frame(i) for i in range(1, 3))
        records.extend(memory(i, 100_000) for i in range(1, 3))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["assessment"]["status"], "no_verificado")
        self.assertIn("effective_hz", result["assessment"]["reasons"])

    def test_zero_effective_refresh_is_rejected_as_invalid_input(self):
        records = complete_records(effective_hz=0)
        with self.assertRaises(AnalysisError):
            analyze_file(write_jsonl(records))

    def test_unpresented_frame_is_not_a_presentation_interval_or_double_counted(self):
        records = [record_run()]
        records.extend(sample(i) for i in range(1, 4))
        records.extend(frame(i, presented=i != 2, interval_ms=1000 / 120) for i in range(1, 4))
        records.extend(memory(i, 100_000) for i in range(1, 4))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["metrics"]["frame_interval_ms"]["count"], 1)
        self.assertEqual(result["metrics"]["presentation_latency_ms"]["count"], 2)
        self.assertAlmostEqual(result["metrics"]["frame_interval_ms"]["p95"], 1000 / 60, places=3)
        self.assertAlmostEqual(result["metrics"]["frame_loss_rate"], 1 / 3, places=5)

    def test_duplicate_vsync_slot_is_invalid(self):
        records = complete_records()
        records[4]["vsync_ts_ns"] = records[3]["vsync_ts_ns"]
        with self.assertRaises(AnalysisError):
            analyze_file(write_jsonl(records))

    def test_memory_growth_uses_sample_timestamps(self):
        records = approval_records()
        records[0]["raw_files"] = [{"kind": "external_camera", "path": "camera.mp4", "sha256": "c" * 64}]
        memory_records = [record for record in records if record["record_type"] == "memory"]
        for index, record in enumerate(memory_records[:3]):
            record["pss_kb"] = 100_000 + index * 1_000
        records = [record for record in records if record["record_type"] != "memory"]
        records.extend(reversed(memory_records[:3]))
        records.append(memory_records[3])
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["memory"]["stable_status"], "growing")

    def test_prediction_is_reported_and_prevents_approval(self):
        records = [record_run()]
        records.extend(sample(i, predicted=i == 2) for i in range(1, 4))
        records.extend(frame(i) for i in range(1, 4))
        records.extend(memory(i, 100_000) for i in range(1, 4))
        result = analyze_file(write_jsonl(records))
        self.assertTrue(result["quality"]["prediction_used"])
        self.assertEqual(result["assessment"]["status"], "no_verificado")
        self.assertIn("prediction_used", result["assessment"]["reasons"])

    def test_missing_required_data_is_rejected(self):
        with self.assertRaises(AnalysisError):
            analyze_file(write_jsonl([record_run()]))

    def test_growing_memory_is_inconclusive(self):
        records = [record_run()]
        records.extend(sample(i) for i in range(1, 5))
        records.extend(frame(i) for i in range(1, 5))
        records.extend(memory(i, 100_000 + i * 10_000) for i in range(1, 5))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["memory"]["stable_status"], "growing")
        self.assertEqual(result["assessment"]["status"], "inconcluso")

    def test_external_latency_is_required_for_approval(self):
        records = [record_run()]
        records.extend(sample(i) for i in range(1, 3))
        records.extend(frame(i) for i in range(1, 3))
        records.extend(memory(i, 100_000) for i in range(1, 3))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["external_latency"]["status"], "no_verificado")
        self.assertNotIn("pencil_to_pixel_ms", result["metrics"])
        self.assertNotEqual(result["assessment"]["status"], "aprobado")

    def test_external_latency_requires_fps_and_uncertainty(self):
        records = [record_run()]
        records.extend(sample(i) for i in range(1, 3))
        records.extend(frame(i) for i in range(1, 3))
        records.extend(memory(i, 100_000) for i in range(1, 3))
        records.append({
            "schema_version": "ink-bench/v1",
            "record_type": "external_latency",
            "latency_ms": 20.0,
        })
        with self.assertRaises(AnalysisError):
            analyze_file(write_jsonl(records))

    def test_complete_external_measurement_can_be_approved(self):
        result = analyze_file(write_jsonl(approval_records()))
        self.assertEqual(result["external_latency"]["status"], "aprobado")
        self.assertEqual(result["assessment"]["status"], "aprobado")

    def test_external_latency_needs_identified_camera_artifact(self):
        records = approval_records()
        records[0]["raw_files"] = []
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["assessment"]["status"], "no_verificado")
        self.assertIn("external_camera_artifact_missing", result["assessment"]["reasons"])

    def test_comparison_requires_same_device_corpus_and_refresh(self):
        left = analyze_file(write_jsonl(approval_records()))
        differing = approval_records()
        differing[0]["display"]["effective_hz"] = 60
        right = analyze_file(write_jsonl(differing))
        right["assessment"]["status"] = "aprobado"
        self.assertEqual(compare_results([left, right])["reason"], "comparison_context_mismatch")

    def test_app_work_excludes_input_queue(self):
        result = analyze_file(write_jsonl(complete_records(sample_duration_ms=8.0)))
        self.assertEqual(result["metrics"]["app_work_ms"]["p95"], 7.0)
        self.assertEqual(result["assessment"]["status"], "rechazado")
        self.assertIn("app_work_p95_ms", result["assessment"]["failures"])

    def test_sixty_hz_is_diagnostic_but_not_a_120_hz_approval(self):
        result = analyze_file(write_jsonl(complete_records(effective_hz=60, sample_duration_ms=7.0, frame_duration_ms=20.0)))
        self.assertEqual(result["assessment"]["status"], "no_verificado")
        self.assertIn("120_hz_not_measured", result["assessment"]["reasons"])
        self.assertNotIn("frame_p95_ms", result["assessment"]["failures"])

    def test_single_external_observation_is_not_enough(self):
        records = complete_records()[:-30]
        records.append(external_measurements(1)[0])
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["external_latency"]["status"], "no_verificado")
        self.assertIn("external_observations_below_30", result["assessment"]["reasons"])

    def test_input_sha256_is_calculated_and_raw_files_can_be_empty(self):
        records = complete_records()
        records[0]["raw_files"] = []
        path = write_jsonl(records)
        result = analyze_file(path)
        expected = hashlib.sha256(path.read_bytes()).hexdigest()
        self.assertEqual(result["input_sha256"], expected)

    def test_raw_files_cannot_claim_the_input_jsonl_hash(self):
        records = complete_records()
        records[0]["raw_files"] = [{"path": "placeholder.jsonl", "sha256": "b" * 64}]
        path = write_jsonl(records)
        records[0]["raw_files"][0]["path"] = path.name
        path.write_text("\n".join(json.dumps(record) for record in records) + "\n", encoding="utf-8")
        with self.assertRaises(AnalysisError):
            analyze_file(path)

    def test_comparison_prioritizes_external_latency_then_event_and_app_work(self):
        fast_external = analyze_file(write_jsonl(approval_records()))
        slow_external = analyze_file(write_jsonl(approval_records()))
        fast_external["run_id"] = "fast-external"
        slow_external["run_id"] = "slow-external"
        fast_external["external_latency"]["latency_ms"]["p95"] = 10.0
        slow_external["external_latency"]["latency_ms"]["p95"] = 15.0
        self.assertEqual(compare_results([slow_external, fast_external]), {"status": "ganador", "run_id": "fast-external"})

    def test_external_difference_below_one_ms_is_tie(self):
        left = analyze_file(write_jsonl(approval_records()))
        right = analyze_file(write_jsonl(approval_records()))
        left["run_id"] = "left"
        right["run_id"] = "right"
        left["external_latency"]["latency_ms"]["p95"] = 10.0
        right["external_latency"]["latency_ms"]["p95"] = 10.5
        self.assertIn(compare_results([left, right])["status"], {"empate", "inconcluso"})

    def test_jsonl_must_describe_one_engine_one_corpus_and_one_frequency(self):
        records = complete_records()
        records[0]["engines"] = [{"id": "engine-a", "version": "fixture"}, {"id": "engine-b", "version": "fixture"}]
        with self.assertRaises(AnalysisError):
            analyze_file(write_jsonl(records))

    def test_sample_requires_stroke_id(self):
        records = complete_records()
        del records[1]["stroke_id"]
        with self.assertRaises(AnalysisError):
            analyze_file(write_jsonl(records))

    def test_approval_requires_sixty_strokes_thousand_samples_memory_and_frames(self):
        result = analyze_file(write_jsonl(complete_records()))
        self.assertEqual(result["assessment"]["status"], "no_verificado")
        self.assertIn("samples_below_1000", result["assessment"]["reasons"])
        self.assertIn("strokes_below_60", result["assessment"]["reasons"])
        self.assertIn("stable_memory_below_3", result["assessment"]["reasons"])
        self.assertIn("stress_memory_missing", result["assessment"]["reasons"])
        self.assertIn("frame_samples_below_120", result["assessment"]["reasons"])

    def test_full_sample_gate_can_approve(self):
        result = analyze_file(write_jsonl(approval_records()))
        self.assertEqual(result["assessment"]["status"], "aprobado")
        self.assertEqual(result["quality"]["sample_count"], 1000)
        self.assertEqual(result["quality"]["stroke_count"], 60)

    def test_presentation_latency_and_frame_interval_are_separate(self):
        records = complete_records()
        records = [record for record in records if record["record_type"] != "frame"]
        records.append(frame(1, interval_ms=8.333333, duration_ms=2.0))
        records.append(frame(2, interval_ms=8.333333, duration_ms=3.0))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["metrics"]["presentation_latency_ms"]["p50"], 2.5)
        self.assertAlmostEqual(result["metrics"]["frame_interval_ms"]["p50"], 9.333333, places=5)

    def test_frame_loss_counts_missing_cadence_slots(self):
        records = complete_records()
        records = [record for record in records if record["record_type"] != "frame"]
        records.append(frame(1, interval_ms=8.333333))
        records.append(frame(3, interval_ms=8.333333))
        result = analyze_file(write_jsonl(records))
        self.assertGreater(result["metrics"]["frame_loss_rate"], 0.0)

    def test_threshold_failure_is_rejected(self):
        records = [record_run()]
        records.extend(sample(i, duration_ms=10.0) for i in range(1, 3))
        records.extend(frame(i) for i in range(1, 3))
        records.extend(memory(i, 100_000) for i in range(1, 3))
        records.extend(external_measurements())
        result = analyze_file(write_jsonl(records))
        self.assertEqual(result["assessment"]["status"], "rechazado")
        self.assertIn("app_work_p95_ms", result["assessment"]["failures"])

    def test_tie_comparison_is_inconclusive(self):
        records = [record_run()]
        records.extend(sample(i) for i in range(1, 3))
        records.extend(frame(i) for i in range(1, 3))
        records.extend(memory(i, 100_000) for i in range(1, 3))
        records.extend(external_measurements())
        result = analyze_file(write_jsonl(records))
        self.assertIn(compare_results([result, result])["status"], {"empate", "inconcluso"})

    def test_comparison_with_unverified_run_is_inconclusive(self):
        records = [record_run()]
        records.extend(sample(i) for i in range(1, 3))
        records.extend(frame(i) for i in range(1, 3))
        records.extend(memory(i, 100_000) for i in range(1, 3))
        result = analyze_file(write_jsonl(records))
        self.assertEqual(compare_results([result])["status"], "inconcluso")


if __name__ == "__main__":
    unittest.main()
