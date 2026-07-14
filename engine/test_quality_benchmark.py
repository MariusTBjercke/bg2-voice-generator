import json
import tempfile
import unittest
import wave
from array import array
from pathlib import Path
from unittest import mock

from engine import quality_benchmark as benchmark


HERE = Path(__file__).resolve().parent
EXAMPLE_CORPUS = HERE / "quality-corpus.example.json"


class QualityBenchmarkTests(unittest.TestCase):
    def test_example_corpus_and_matrix_cover_required_cases(self) -> None:
        corpus = benchmark.load_corpus(EXAMPLE_CORPUS)
        benchmark.validate_experiments()
        plan = benchmark.build_plan(corpus, list(benchmark.EXPERIMENTS))

        self.assertEqual(plan["voice_count"], 4)
        self.assertEqual(plan["line_count"], 20)
        self.assertEqual(set(plan["categories"]), benchmark.REQUIRED_CATEGORIES)
        self.assertEqual(plan["case_count"], 20 * len(benchmark.EXPERIMENTS))
        self.assertEqual(
            {
                experiment.settings()["speed"]
                for experiment in benchmark.EXPERIMENTS
                if experiment.experiment_id == benchmark.BASELINE_EXPERIMENT
                or experiment.changed_field == "speed"
            },
            {None, 0.9, 1.0, 1.15, 1.25},
        )
        for experiment in benchmark.EXPERIMENTS[1:]:
            self.assertEqual(len(experiment.override), 1)
        for case in plan["cases"]:
            self.assertEqual(len(case["settings_sha256"]), 64)

    def test_wav_metrics_measure_silence_and_clipping(self) -> None:
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            path = Path(temp) / "metrics.wav"
            samples = array("h", [0] * 1_000 + [10_000] * 998 + [32_767, -32_768])
            with wave.open(str(path), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(1_000)
                output.writeframes(samples.tobytes())

            metrics = benchmark.measure_wav(path)

        self.assertEqual(metrics["duration_seconds"], 2.0)
        self.assertAlmostEqual(metrics["silence_ratio"], 0.5)
        self.assertEqual(metrics["clipping_samples"], 2)
        self.assertAlmostEqual(metrics["clipping_ratio"], 0.001)

    def test_blind_staging_hides_identity_and_aggregation_decodes_winner(self) -> None:
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            output = Path(temp)
            audio = output / "audio"
            audio.mkdir()
            records = []
            for line_id in ("line_1", "line_2"):
                for experiment_id in (benchmark.BASELINE_EXPERIMENT, "speed_1_00"):
                    relative = Path("audio") / f"{line_id}-{experiment_id}.wav"
                    (output / relative).write_bytes(b"fixture audio")
                    records.append(
                        {
                            "voice_id": "voice_a",
                            "line_id": line_id,
                            "category": "short",
                            "text": f"Fixture {line_id}",
                            "experiment_id": experiment_id,
                            "output_path": relative.as_posix(),
                            "status": "done",
                        }
                    )
            benchmark.atomic_write_jsonl(output / "results.jsonl", records)

            trials = benchmark.stage_blind_trials(output, seed=7)
            public_payload = json.loads((output / "blind" / "trials.json").read_text())
            key_payload = json.loads((output / "blind" / "key.json").read_text())

            self.assertEqual(trials["trial_count"], 2)
            public_text = json.dumps(public_payload)
            self.assertNotIn("speed_1_00", public_text)
            self.assertNotIn("baseline_side", public_text)
            self.assertNotIn("output_path", public_text)
            self.assertTrue(all((output / "blind" / trial["audio_a"]).is_file() for trial in trials["trials"]))
            self.assertEqual(
                {entry["baseline_side"] for entry in key_payload["entries"]}, {"a", "b"}
            )

            first_key = key_payload["entries"][0]
            benchmark.record_preference(
                output, first_key["trial_id"], first_key["variant_side"], "preferred variant"
            )
            summary = benchmark.summarize_preferences(output)

        experiment = summary["experiments"][0]
        self.assertEqual(experiment["variant_wins"], 1)
        self.assertEqual(experiment["baseline_wins"], 0)
        self.assertEqual(experiment["variant_preference_rate"], 1.0)

    def test_invalid_corpus_is_rejected(self) -> None:
        corpus = benchmark.load_corpus(EXAMPLE_CORPUS)
        corpus["lines"] = corpus["lines"][:19]
        with self.assertRaisesRegex(benchmark.BenchmarkError, "twenty lines"):
            benchmark.validate_corpus(corpus)

    def test_runner_records_each_synthesis_failure(self) -> None:
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            root = Path(temp)
            reference = root / "reference.wav"
            reference.write_bytes(b"local reference fixture")
            corpus = benchmark.load_corpus(EXAMPLE_CORPUS)
            for voice in corpus["voices"]:
                voice["reference_audio"] = str(reference)
                voice["reference_text"] = "Exact local fixture transcript."
            corpus_path = root / "corpus.json"
            corpus_path.write_text(json.dumps(corpus), encoding="utf-8")

            def fake_http(method, url, payload=None, timeout=600.0):
                if method == "GET":
                    return {"status": "ok", "ready": True, "model_id": "fixture"}
                raise benchmark.BenchmarkError("fixture synthesis failure")

            with mock.patch.object(benchmark, "http_json", side_effect=fake_http):
                records = benchmark.run_benchmark(
                    corpus_path,
                    root / "results",
                    "http://fixture",
                    [benchmark.EXPERIMENT_BY_ID[benchmark.BASELINE_EXPERIMENT]],
                )
            metadata = json.loads((root / "results" / "run-metadata.json").read_text())

        self.assertEqual(len(records), 20)
        self.assertTrue(all(record["status"] == "failed" for record in records))
        self.assertTrue(all(record["metrics"] is None for record in records))
        self.assertTrue(all("fixture synthesis failure" in record["error"] for record in records))
        self.assertEqual(metadata["result_counts"], {"total": 20, "done": 0, "failed": 20, "vad_missing": 0})


if __name__ == "__main__":
    unittest.main()
