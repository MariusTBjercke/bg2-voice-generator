# OmniVoice quality benchmark

Use `engine/quality_benchmark.py` to compare OmniVoice render settings without changing
application defaults. It renders a controlled matrix, records timing and WAV/VAD metrics,
and stages opaque A/B pairs for human listening.

## Safety and prerequisites

- Run this only on a machine with the local OmniVoice model and a supported GPU. A dry run
  validates the experiment design but is not quality evidence.
- Do not commit or transfer references, generated clips, or benchmark output. They may be
  derived from game audio. The checked-in example contains invented text and no audio.
- Make a private copy of `engine/quality-corpus.example.json`. Replace all four reference
  paths and every `reference_text` value with the exact, transcript-aligned contents of the
  corresponding WAV. Use the same references for the entire run.
- Use the same model version, inference fork, server process, corpus, and WAV format for all
  matrix cells. The runner hashes the corpus, settings, reference WAVs, and texts, and saves
  the engine health response in `run-metadata.json`.

The manifest is versioned and requires at least four voices and twenty lines spanning
`short`, `medium`, `long`, `emotional`, and `punctuation_heavy`. Environment variables in
reference paths are expanded, so the example's `BG2_BENCH_VOICE_A` through
`BG2_BENCH_VOICE_D` placeholders may be set instead of replacing the paths.

## Plan and run

First inspect the exact matrix without loading a model:

```powershell
python engine\quality_benchmark.py plan C:\private\bg2-quality-corpus.json --out C:\private\quality-run\plan.json
```

The full matrix keeps production settings fixed for the `auto_pace` baseline, compares
speeds `0.9`, `1.0`, `1.15`, and `1.25`, and changes every other experimental render field
one at a time. It is intentionally expensive. A focused pacing pass can be selected first:

```powershell
python engine\quality_benchmark.py run C:\private\bg2-quality-corpus.json `
  --output C:\private\quality-run `
  --experiment auto_pace `
  --experiment speed_0_90 `
  --experiment speed_1_00 `
  --experiment speed_1_15 `
  --experiment speed_1_25
```

The managed server defaults to `http://127.0.0.1:8140`; pass `--base-url` if it is running
elsewhere. Start it through the app's Generation screen so the same pinned engine and fork
used by production are exercised. Use a fresh output directory for each run. `--overwrite`
is explicit and replaces result records for that directory, but does not erase unrelated
files.

Each JSONL result records generation time, engine-reported duration, measured duration,
silence ratio, clipping samples/ratio, peak level, VAD speech ratio, settings and input
hashes, and any synthesis, WAV, or VAD failure. Empty audio is a failed cell. Objective
metrics help identify suspicious takes; they do not establish audible quality.

## Blind listening

Stage baseline-versus-variant pairs under opaque filenames and deterministic randomized
sides:

```powershell
python engine\quality_benchmark.py blind --output C:\private\quality-run --seed 42
```

Give the listener `blind\trials.json` and `blind\audio\`. Keep `blind\key.json` hidden until
all ratings are complete. Record `a`, `b`, or `tie` for each trial:

```powershell
python engine\quality_benchmark.py record --output C:\private\quality-run --trial <trial-id> --winner a
python engine\quality_benchmark.py summarize --output C:\private\quality-run
```

The summary decodes sides only during aggregation and reports baseline wins, variant wins,
ties, and the decisive variant preference rate per isolated experiment. Review failures and
missing VAD results alongside preferences; a preference rate alone is not sufficient when a
variant increases empty or failed renders.

## Interpretation

Keep automatic pacing and 32 diffusion steps as production defaults unless repeated,
matched, blind comparisons show an audible improvement worth the measured cost. Composite
references must remain opt-in unless their separate matched trials reach at least 60%
preference without increasing empty or failed output.

No real-GPU benchmark was executed while adding this harness. Its presence is not evidence
for a setting change, and the current BG2 defaults remain unchanged.
