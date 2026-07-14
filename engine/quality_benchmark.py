"""Reproducible local A/B harness for BG2's managed OmniVoice server.

The harness never bundles reference audio. A corpus manifest points at four or more
local WAV files and supplies their exact transcripts. Every experiment resolves from
one shared BG2 default settings object and changes at most one field.
"""

from __future__ import annotations

import argparse
import array
import hashlib
import json
import math
import os
import random
import re
import shutil
import sys
import time
import urllib.error
import urllib.request
import wave
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

HARNESS_VERSION = 1
CORPUS_VERSION = 1
BASELINE_EXPERIMENT = "auto_pace"
TARGET_SAMPLE_RATE = 22_050
SILENCE_WINDOW_MS = 20
SILENCE_THRESHOLD_DBFS = -45.0
CLIPPING_ABS_SAMPLE = 32_760
REQUIRED_CATEGORIES = {
    "short",
    "medium",
    "long",
    "emotional",
    "punctuation_heavy",
}

# Field order matches Rust's OmniVoiceRenderSettings contract. These are the
# production defaults and remain unchanged by this benchmark tooling.
DEFAULT_SETTINGS: dict[str, Any] = {
    "speed": None,
    "num_steps": 32,
    "guidance_scale": 2.0,
    "t_shift": 0.1,
    "layer_penalty_factor": 5.0,
    "position_temperature": 5.0,
    "class_temperature": 0.0,
    "prompt_denoise": True,
    "preprocess_prompt": True,
    "postprocess_output": True,
    "audio_chunk_duration": 10.0,
    "audio_chunk_threshold": 30.0,
    "seed": 42,
    "peak_normalize_dbfs": -1.0,
}


@dataclass(frozen=True)
class Experiment:
    experiment_id: str
    label: str
    override: dict[str, Any]

    @property
    def changed_field(self) -> str | None:
        return next(iter(self.override), None)

    def settings(self) -> dict[str, Any]:
        resolved = dict(DEFAULT_SETTINGS)
        resolved.update(self.override)
        return resolved


# Automatic pacing is the baseline. Every other entry differs by exactly one
# field, so the harness cannot accidentally compare uncontrolled bundles.
EXPERIMENTS: tuple[Experiment, ...] = (
    Experiment("auto_pace", "Automatic model pacing", {}),
    Experiment("speed_0_90", "Fixed speed 0.90x", {"speed": 0.9}),
    Experiment("speed_1_00", "Fixed speed 1.00x", {"speed": 1.0}),
    Experiment("speed_1_15", "Fixed speed 1.15x", {"speed": 1.15}),
    Experiment("speed_1_25", "Fixed speed 1.25x (Morrowind)", {"speed": 1.25}),
    Experiment("steps_48", "48 diffusion steps", {"num_steps": 48}),
    Experiment("guidance_2_50", "Guidance 2.50", {"guidance_scale": 2.5}),
    Experiment("t_shift_0_20", "Time-step shift 0.20", {"t_shift": 0.2}),
    Experiment("layer_penalty_4", "Layer penalty 4.0", {"layer_penalty_factor": 4.0}),
    Experiment("position_temp_3", "Position temperature 3.0", {"position_temperature": 3.0}),
    Experiment("class_temp_0_20", "Class temperature 0.20", {"class_temperature": 0.2}),
    Experiment("prompt_denoise_off", "Prompt denoise disabled", {"prompt_denoise": False}),
    Experiment("preprocess_off", "Reference preprocessing disabled", {"preprocess_prompt": False}),
    Experiment("postprocess_off", "Output postprocessing disabled", {"postprocess_output": False}),
    Experiment("chunk_duration_15", "Chunk duration 15s", {"audio_chunk_duration": 15.0}),
    Experiment("chunk_threshold_45", "Chunk threshold 45s", {"audio_chunk_threshold": 45.0}),
    Experiment("seed_7", "Fixed seed 7", {"seed": 7}),
    Experiment("peak_minus_2", "Peak normalize -2 dBFS", {"peak_normalize_dbfs": -2.0}),
)
EXPERIMENT_BY_ID = {experiment.experiment_id: experiment for experiment in EXPERIMENTS}


class BenchmarkError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_json(value: Any) -> str:
    return sha256_bytes(canonical_json(value).encode("utf-8"))


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_id(value: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("._")
    if not cleaned:
        raise BenchmarkError(f"identifier has no safe characters: {value!r}")
    return cleaned


def load_corpus(path: Path) -> dict[str, Any]:
    try:
        corpus = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise BenchmarkError(f"could not read corpus {path}: {exc}") from exc
    validate_corpus(corpus)
    return corpus


def validate_corpus(corpus: dict[str, Any]) -> None:
    if not isinstance(corpus, dict) or corpus.get("version") != CORPUS_VERSION:
        raise BenchmarkError(f"corpus version must be {CORPUS_VERSION}")
    voices = corpus.get("voices")
    lines = corpus.get("lines")
    if not isinstance(voices, list) or len(voices) < 4:
        raise BenchmarkError("corpus needs at least four voices")
    if not isinstance(lines, list) or len(lines) < 20:
        raise BenchmarkError("corpus needs at least twenty lines")

    voice_ids: set[str] = set()
    for voice in voices:
        if not isinstance(voice, dict):
            raise BenchmarkError("every voice must be an object")
        voice_id = voice.get("id")
        if not isinstance(voice_id, str) or not voice_id.strip():
            raise BenchmarkError("every voice needs a non-empty id")
        safe_id(voice_id)
        if voice_id in voice_ids:
            raise BenchmarkError(f"duplicate voice id: {voice_id}")
        voice_ids.add(voice_id)
        for field in ("reference_audio", "reference_text"):
            if not isinstance(voice.get(field), str) or not voice[field].strip():
                raise BenchmarkError(f"voice {voice_id} needs non-empty {field}")

    line_ids: set[str] = set()
    used_voices: set[str] = set()
    categories: set[str] = set()
    for line in lines:
        if not isinstance(line, dict):
            raise BenchmarkError("every line must be an object")
        line_id = line.get("id")
        if not isinstance(line_id, str) or not line_id.strip():
            raise BenchmarkError("every line needs a non-empty id")
        safe_id(line_id)
        if line_id in line_ids:
            raise BenchmarkError(f"duplicate line id: {line_id}")
        line_ids.add(line_id)
        voice_id = line.get("voice_id")
        if voice_id not in voice_ids:
            raise BenchmarkError(f"line {line_id} references unknown voice {voice_id!r}")
        used_voices.add(voice_id)
        category = line.get("category")
        if category not in REQUIRED_CATEGORIES:
            raise BenchmarkError(
                f"line {line_id} category must be one of {sorted(REQUIRED_CATEGORIES)}"
            )
        categories.add(category)
        if not isinstance(line.get("text"), str) or not line["text"].strip():
            raise BenchmarkError(f"line {line_id} needs non-empty text")
    if len(used_voices) < 4:
        raise BenchmarkError("at least four voices must be represented by lines")
    missing = REQUIRED_CATEGORIES - categories
    if missing:
        raise BenchmarkError(f"corpus is missing categories: {sorted(missing)}")


def validate_experiments() -> None:
    ids: set[str] = set()
    for experiment in EXPERIMENTS:
        if experiment.experiment_id in ids:
            raise BenchmarkError(f"duplicate experiment id: {experiment.experiment_id}")
        ids.add(experiment.experiment_id)
        if experiment.experiment_id == BASELINE_EXPERIMENT:
            if experiment.override:
                raise BenchmarkError("baseline must not override production defaults")
            continue
        if len(experiment.override) != 1:
            raise BenchmarkError(
                f"experiment {experiment.experiment_id} must change exactly one setting"
            )
        field = experiment.changed_field
        if field not in DEFAULT_SETTINGS:
            raise BenchmarkError(f"experiment changes unknown setting: {field}")
        if experiment.override[field] == DEFAULT_SETTINGS[field]:
            raise BenchmarkError(f"experiment {experiment.experiment_id} does not change {field}")
    expected_speeds = {None, 0.9, 1.0, 1.15, 1.25}
    actual_speeds = {
        experiment.settings()["speed"]
        for experiment in EXPERIMENTS
        if experiment.experiment_id == BASELINE_EXPERIMENT
        or experiment.changed_field == "speed"
    }
    if actual_speeds != expected_speeds:
        raise BenchmarkError(f"speed matrix drifted: {actual_speeds!r}")


def select_experiments(ids: Iterable[str] | None) -> list[Experiment]:
    validate_experiments()
    selected_ids = list(ids or [])
    if not selected_ids:
        return list(EXPERIMENTS)
    unknown = [experiment_id for experiment_id in selected_ids if experiment_id not in EXPERIMENT_BY_ID]
    if unknown:
        raise BenchmarkError(f"unknown experiments: {unknown}")
    return [EXPERIMENT_BY_ID[experiment_id] for experiment_id in selected_ids]


def build_plan(corpus: dict[str, Any], experiments: list[Experiment]) -> dict[str, Any]:
    cases = []
    for line in corpus["lines"]:
        for experiment in experiments:
            settings = experiment.settings()
            cases.append(
                {
                    "case_id": f"{line['voice_id']}::{line['id']}::{experiment.experiment_id}",
                    "voice_id": line["voice_id"],
                    "line_id": line["id"],
                    "category": line["category"],
                    "experiment_id": experiment.experiment_id,
                    "changed_field": experiment.changed_field,
                    "settings_sha256": sha256_json(settings),
                }
            )
    return {
        "harness_version": HARNESS_VERSION,
        "corpus_version": corpus["version"],
        "corpus_sha256": sha256_json(corpus),
        "voice_count": len(corpus["voices"]),
        "line_count": len(corpus["lines"]),
        "categories": sorted({line["category"] for line in corpus["lines"]}),
        "experiments": [
            {
                "experiment_id": experiment.experiment_id,
                "label": experiment.label,
                "changed_field": experiment.changed_field,
                "override": experiment.override,
                "settings": experiment.settings(),
                "settings_sha256": sha256_json(experiment.settings()),
            }
            for experiment in experiments
        ],
        "case_count": len(cases),
        "cases": cases,
    }


def http_json(
    method: str,
    url: str,
    payload: dict[str, Any] | None = None,
    timeout: float = 600.0,
) -> dict[str, Any]:
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        method=method,
        headers={"Content-Type": "application/json"} if body is not None else {},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise BenchmarkError(f"{method} {url} failed ({exc.code}): {detail}") from exc
    except (OSError, TimeoutError, json.JSONDecodeError) as exc:
        raise BenchmarkError(f"{method} {url} failed: {exc}") from exc


def measure_wav(path: Path) -> dict[str, Any]:
    try:
        with wave.open(str(path), "rb") as source:
            channels = source.getnchannels()
            sample_width = source.getsampwidth()
            sample_rate = source.getframerate()
            frame_count = source.getnframes()
            raw = source.readframes(frame_count)
    except (OSError, wave.Error) as exc:
        raise BenchmarkError(f"could not measure WAV {path}: {exc}") from exc
    if channels != 1 or sample_width != 2:
        raise BenchmarkError(
            f"benchmark WAV must be mono 16-bit PCM, got channels={channels} width={sample_width}"
        )
    samples = array.array("h")
    samples.frombytes(raw)
    if sys.byteorder != "little":
        samples.byteswap()
    if not samples:
        raise BenchmarkError("model produced empty audio (0 samples)")

    window_samples = max(1, round(sample_rate * SILENCE_WINDOW_MS / 1000.0))
    silence_threshold = 32_767.0 * (10.0 ** (SILENCE_THRESHOLD_DBFS / 20.0))
    silent_samples = 0
    for start in range(0, len(samples), window_samples):
        window = samples[start : start + window_samples]
        rms = math.sqrt(sum(float(sample) ** 2 for sample in window) / len(window))
        if rms <= silence_threshold:
            silent_samples += len(window)
    clipping_samples = sum(1 for sample in samples if abs(sample) >= CLIPPING_ABS_SAMPLE)
    peak = max(abs(sample) for sample in samples)
    peak_dbfs = 20.0 * math.log10(peak / 32_767.0) if peak else None
    duration = len(samples) / float(sample_rate)
    return {
        "sample_rate": sample_rate,
        "samples": len(samples),
        "duration_seconds": round(duration, 6),
        "silence_seconds": round(silent_samples / float(sample_rate), 6),
        "silence_ratio": round(silent_samples / float(len(samples)), 6),
        "clipping_samples": clipping_samples,
        "clipping_ratio": round(clipping_samples / float(len(samples)), 8),
        "peak_dbfs": None if peak_dbfs is None else round(peak_dbfs, 4),
    }


def atomic_write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".part")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def atomic_write_jsonl(path: Path, values: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".part")
    with temporary.open("w", encoding="utf-8", newline="\n") as output:
        for value in values:
            output.write(json.dumps(value, ensure_ascii=False, sort_keys=True) + "\n")
    os.replace(temporary, path)


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    try:
        with path.open("r", encoding="utf-8") as source:
            return [json.loads(line) for line in source if line.strip()]
    except (OSError, json.JSONDecodeError) as exc:
        raise BenchmarkError(f"could not read results {path}: {exc}") from exc


def _expanded_reference_path(raw: str, corpus_path: Path) -> Path:
    expanded = Path(os.path.expandvars(os.path.expanduser(raw)))
    if not expanded.is_absolute():
        expanded = corpus_path.parent / expanded
    return expanded.resolve()


def run_benchmark(
    corpus_path: Path,
    output_dir: Path,
    base_url: str,
    experiments: list[Experiment],
    overwrite: bool = False,
) -> list[dict[str, Any]]:
    corpus = load_corpus(corpus_path)
    plan = build_plan(corpus, experiments)
    output_dir = output_dir.resolve()
    results_path = output_dir / "results.jsonl"
    if results_path.exists() and not overwrite:
        raise BenchmarkError(f"{results_path} already exists; use --overwrite or a new output dir")
    output_dir.mkdir(parents=True, exist_ok=True)

    health = http_json("GET", f"{base_url.rstrip('/')}/health", timeout=30.0)
    if health.get("status") != "ok":
        raise BenchmarkError(f"engine health is not ok: {health}")
    metadata = {
        "harness_version": HARNESS_VERSION,
        "started_at": utc_now(),
        "base_url": base_url,
        "engine_health": health,
        "output_format": {"container": "wav", "pcm_bits": 16, "channels": 1, "sample_rate": TARGET_SAMPLE_RATE},
        "plan": plan,
    }
    atomic_write_json(output_dir / "run-metadata.json", metadata)

    voices = {voice["id"]: voice for voice in corpus["voices"]}
    reference_cache: dict[str, tuple[Path, str]] = {}
    for voice_id, voice in voices.items():
        reference_path = _expanded_reference_path(voice["reference_audio"], corpus_path)
        if not reference_path.is_file():
            raise BenchmarkError(f"voice {voice_id} reference does not exist: {reference_path}")
        reference_cache[voice_id] = (reference_path, sha256_file(reference_path))

    records: list[dict[str, Any]] = []
    for line in corpus["lines"]:
        voice = voices[line["voice_id"]]
        reference_path, reference_sha = reference_cache[line["voice_id"]]
        for experiment in experiments:
            settings = experiment.settings()
            relative_output = Path("audio") / safe_id(experiment.experiment_id) / (
                f"{safe_id(line['voice_id'])}__{safe_id(line['id'])}.wav"
            )
            output_path = output_dir / relative_output
            output_path.parent.mkdir(parents=True, exist_ok=True)
            payload = {
                "text": line["text"],
                "ref_audio": str(reference_path),
                "ref_text": voice["reference_text"],
                "out_path": str(output_path),
                "target_sample_rate": TARGET_SAMPLE_RATE,
                **settings,
            }
            case_id = f"{line['voice_id']}::{line['id']}::{experiment.experiment_id}"
            started = time.perf_counter()
            response: dict[str, Any] | None = None
            metrics: dict[str, Any] | None = None
            error: str | None = None
            try:
                response = http_json(
                    "POST", f"{base_url.rstrip('/')}/synthesize", payload, timeout=600.0
                )
                metrics = measure_wav(output_path)
            except Exception as exc:  # keep every failed matrix cell in the evidence
                error = f"{type(exc).__name__}: {exc}"
            elapsed = time.perf_counter() - started
            records.append(
                {
                    "harness_version": HARNESS_VERSION,
                    "case_id": case_id,
                    "voice_id": line["voice_id"],
                    "line_id": line["id"],
                    "category": line["category"],
                    "text": line["text"],
                    "text_sha256": sha256_bytes(line["text"].encode("utf-8")),
                    "reference_audio_sha256": reference_sha,
                    "reference_text_sha256": sha256_bytes(voice["reference_text"].encode("utf-8")),
                    "experiment_id": experiment.experiment_id,
                    "changed_field": experiment.changed_field,
                    "settings": settings,
                    "settings_sha256": sha256_json(settings),
                    "output_path": relative_output.as_posix() if error is None else None,
                    "status": "done" if error is None else "failed",
                    "generation_seconds": round(elapsed, 6),
                    "engine_duration_seconds": None if response is None else response.get("duration"),
                    "metrics": metrics,
                    "speech_ratio": None,
                    "vad_duration_seconds": None,
                    "vad_error": None,
                    "error": error,
                }
            )

    successful = [record for record in records if record["status"] == "done"]
    for start in range(0, len(successful), 64):
        chunk = successful[start : start + 64]
        paths = [str(output_dir / record["output_path"]) for record in chunk]
        try:
            response = http_json(
                "POST", f"{base_url.rstrip('/')}/vad_batch", {"paths": paths}, timeout=300.0
            )
            by_path = {item.get("path"): item for item in response.get("items", [])}
            for record, path in zip(chunk, paths):
                item = by_path.get(path)
                if item is None:
                    record["vad_error"] = "VAD response omitted path"
                else:
                    record["speech_ratio"] = item.get("speech_ratio")
                    record["vad_duration_seconds"] = item.get("duration")
                    record["vad_error"] = item.get("error")
        except Exception as exc:
            for record in chunk:
                record["vad_error"] = f"{type(exc).__name__}: {exc}"

    atomic_write_jsonl(results_path, records)
    metadata["finished_at"] = utc_now()
    metadata["result_counts"] = {
        "total": len(records),
        "done": sum(record["status"] == "done" for record in records),
        "failed": sum(record["status"] == "failed" for record in records),
        "vad_missing": sum(
            record["status"] == "done" and record["speech_ratio"] is None for record in records
        ),
    }
    atomic_write_json(output_dir / "run-metadata.json", metadata)
    return records


def stage_blind_trials(output_dir: Path, seed: int = 42) -> dict[str, Any]:
    output_dir = output_dir.resolve()
    records = read_jsonl(output_dir / "results.jsonl")
    successful = {
        (record["voice_id"], record["line_id"], record["experiment_id"]): record
        for record in records
        if record.get("status") == "done" and record.get("output_path")
    }
    randomizer = random.Random(seed)
    blind_audio = output_dir / "blind" / "audio"
    blind_audio.mkdir(parents=True, exist_ok=True)
    trials: list[dict[str, Any]] = []
    key_entries: list[dict[str, Any]] = []
    for (voice_id, line_id, experiment_id), variant in sorted(successful.items()):
        if experiment_id == BASELINE_EXPERIMENT:
            continue
        baseline = successful.get((voice_id, line_id, BASELINE_EXPERIMENT))
        if baseline is None:
            continue
        trial_id = sha256_bytes(f"{seed}:{voice_id}:{line_id}:{experiment_id}".encode())[:20]
        baseline_side = "a" if randomizer.randrange(2) == 0 else "b"
        variant_side = "b" if baseline_side == "a" else "a"
        staged: dict[str, str] = {}
        for side, source_record in ((baseline_side, baseline), (variant_side, variant)):
            opaque = sha256_bytes(f"{seed}:{trial_id}:{side}".encode())[:24] + ".wav"
            source_path = output_dir / source_record["output_path"]
            if not source_path.is_file():
                raise BenchmarkError(f"blind source missing: {source_path}")
            destination = blind_audio / opaque
            shutil.copy2(source_path, destination)
            staged[side] = (Path("audio") / opaque).as_posix()
        trials.append(
            {
                "trial_id": trial_id,
                "category": variant["category"],
                "text": variant["text"],
                "audio_a": staged["a"],
                "audio_b": staged["b"],
            }
        )
        key_entries.append(
            {
                "trial_id": trial_id,
                "voice_id": voice_id,
                "line_id": line_id,
                "experiment_id": experiment_id,
                "baseline_side": baseline_side,
                "variant_side": variant_side,
            }
        )
    if not trials:
        raise BenchmarkError(
            "no matched blind trials; render auto_pace and at least one variant successfully"
        )
    trials_payload = {
        "harness_version": HARNESS_VERSION,
        "seed": seed,
        "created_at": utc_now(),
        "trial_count": len(trials),
        "trials": trials,
    }
    key_payload = {
        "harness_version": HARNESS_VERSION,
        "seed": seed,
        "created_at": utc_now(),
        "entries": key_entries,
    }
    atomic_write_json(output_dir / "blind" / "trials.json", trials_payload)
    atomic_write_json(output_dir / "blind" / "key.json", key_payload)
    return trials_payload


def record_preference(
    output_dir: Path,
    trial_id: str,
    winner: str,
    notes: str | None = None,
) -> dict[str, Any]:
    if winner not in {"a", "b", "tie"}:
        raise BenchmarkError("winner must be a, b, or tie")
    trials_path = output_dir / "blind" / "trials.json"
    try:
        trials = json.loads(trials_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise BenchmarkError(f"could not read blind trials: {exc}") from exc
    if trial_id not in {trial["trial_id"] for trial in trials.get("trials", [])}:
        raise BenchmarkError(f"unknown trial id: {trial_id}")
    preferences_path = output_dir / "blind" / "preferences.json"
    if preferences_path.exists():
        preferences = json.loads(preferences_path.read_text(encoding="utf-8"))
    else:
        preferences = {"harness_version": HARNESS_VERSION, "ratings": []}
    rating = {
        "trial_id": trial_id,
        "winner": winner,
        "notes": notes or "",
        "rated_at": utc_now(),
    }
    ratings = [item for item in preferences.get("ratings", []) if item.get("trial_id") != trial_id]
    ratings.append(rating)
    ratings.sort(key=lambda item: item["trial_id"])
    preferences["ratings"] = ratings
    atomic_write_json(preferences_path, preferences)
    return rating


def summarize_preferences(output_dir: Path) -> dict[str, Any]:
    blind_dir = output_dir / "blind"
    try:
        key = json.loads((blind_dir / "key.json").read_text(encoding="utf-8"))
        preferences = json.loads((blind_dir / "preferences.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise BenchmarkError(f"could not read blind key/preferences: {exc}") from exc
    by_trial = {entry["trial_id"]: entry for entry in key.get("entries", [])}
    summaries: dict[str, dict[str, Any]] = {}
    ignored = 0
    for rating in preferences.get("ratings", []):
        entry = by_trial.get(rating.get("trial_id"))
        if entry is None:
            ignored += 1
            continue
        experiment_id = entry["experiment_id"]
        summary = summaries.setdefault(
            experiment_id,
            {
                "experiment_id": experiment_id,
                "rated_trials": 0,
                "baseline_wins": 0,
                "variant_wins": 0,
                "ties": 0,
            },
        )
        summary["rated_trials"] += 1
        winner = rating.get("winner")
        if winner == "tie":
            summary["ties"] += 1
        elif winner == entry["baseline_side"]:
            summary["baseline_wins"] += 1
        elif winner == entry["variant_side"]:
            summary["variant_wins"] += 1
        else:
            ignored += 1
            summary["rated_trials"] -= 1
    for summary in summaries.values():
        decisive = summary["baseline_wins"] + summary["variant_wins"]
        summary["variant_preference_rate"] = (
            None if decisive == 0 else round(summary["variant_wins"] / decisive, 6)
        )
    payload = {
        "harness_version": HARNESS_VERSION,
        "created_at": utc_now(),
        "rated_trials": sum(summary["rated_trials"] for summary in summaries.values()),
        "ignored_ratings": ignored,
        "experiments": [summaries[key] for key in sorted(summaries)],
    }
    atomic_write_json(blind_dir / "preference-summary.json", payload)
    return payload


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    plan = subparsers.add_parser("plan", help="validate a corpus and print the exact matrix")
    plan.add_argument("corpus", type=Path)
    plan.add_argument("--experiment", action="append", dest="experiments")
    plan.add_argument("--out", type=Path, help="also write the plan JSON")

    run = subparsers.add_parser("run", help="render the controlled matrix against a local server")
    run.add_argument("corpus", type=Path)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--base-url", default="http://127.0.0.1:8140")
    run.add_argument("--experiment", action="append", dest="experiments")
    run.add_argument("--overwrite", action="store_true")

    blind = subparsers.add_parser("blind", help="stage opaque baseline-vs-variant WAV pairs")
    blind.add_argument("--output", type=Path, required=True)
    blind.add_argument("--seed", type=int, default=42)

    record = subparsers.add_parser("record", help="record one blind preference")
    record.add_argument("--output", type=Path, required=True)
    record.add_argument("--trial", required=True)
    record.add_argument("--winner", choices=("a", "b", "tie"), required=True)
    record.add_argument("--notes")

    summarize = subparsers.add_parser("summarize", help="aggregate recorded blind preferences")
    summarize.add_argument("--output", type=Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "plan":
            corpus = load_corpus(args.corpus)
            payload = build_plan(corpus, select_experiments(args.experiments))
            if args.out:
                atomic_write_json(args.out, payload)
        elif args.command == "run":
            records = run_benchmark(
                args.corpus,
                args.output,
                args.base_url,
                select_experiments(args.experiments),
                args.overwrite,
            )
            payload = {
                "total": len(records),
                "done": sum(record["status"] == "done" for record in records),
                "failed": sum(record["status"] == "failed" for record in records),
                "results": str((args.output / "results.jsonl").resolve()),
            }
        elif args.command == "blind":
            payload = stage_blind_trials(args.output, args.seed)
        elif args.command == "record":
            payload = record_preference(args.output, args.trial, args.winner, args.notes)
        else:
            payload = summarize_preferences(args.output)
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return 0
    except BenchmarkError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
