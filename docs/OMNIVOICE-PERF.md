# OmniVoice generation performance

BG2 Voice Generator uses an optimized OmniVoice stack (pinned `omnivoice==0.1.5` recipe).

## Engine wins (Python)

| Feature | Effect |
|---------|--------|
| fp16 + explicit CUDA device | ~1.5–2× vs stock fp32 |
| `omnivoice_fork.py` | ~1.6× on diffusion (split CFG, vectorized scoring) |
| Voice-clone prompt LRU (32) | Skips re-tokenizing the same reference each batch |
| `gc` + `empty_cache` after each batch | Prevents VRAM fragmentation on long runs |
| Parallel WAV write pool | Overlaps post-GPU encode/write |

Set `OMNIVOICE_STOCK_GENERATE=1` to force stock generation. The fork auto-falls back
to stock on any runtime error.

## Rust orchestrator wins

| Feature | Effect |
|---------|--------|
| Length-sorted batch packing | Less padding waste in batched diffusion |
| Pipeline depth 2 | Next batch starts while the previous finalizes |
| Text dedup + fan-out | One synthesis per identical `(text, clone)` |
| Partial batch recovery | Adopts on-disk clips before per-line fallback |

## Tuning

- Defaults: **8 lines**, **800 chars** per batch (Generation screen).
- Watch stderr `peak=` after raising char budget — long lines spike VRAM super-linearly.
- Reference clips: **5–8 s ideal**; Binding warns above **8 s**.

## Verification

```powershell
python engine/fork_parity_check.py <ref.wav> "reference transcript text"
```

Re-run after any `omnivoice` package bump.

For controlled setting comparisons, objective audio metrics, and blind A/B preference
records, follow the [OmniVoice quality benchmark](testing/omnivoice-quality-benchmark.md).
The checked-in corpus manifest is audio-free; references and generated results stay local.

## Validation record (2026-07-14)

- The harness plan was exercised on an NVIDIA GeForce RTX 5070 Ti (16,303 MiB,
  driver 610.47): the checked-in audio-free manifest expands to 4 voices, 20 lines,
  and 360 isolated render cases. The seven harness/server unit tests passed.
- This was a plan and code-path validation only. No generated audio was rendered or
  auditioned because the manifest deliberately contains placeholder reference paths;
  no private local reference corpus or blinded listener ratings were supplied.
- Therefore automatic pacing, the 32-step default, and the opt-in composite-reference
  policy remain unchanged. Objective metrics and diagnostic flags are useful review
  signals, not evidence of audible preference.

To complete the empirical pass locally, prepare the private corpus described in
[`testing/omnivoice-quality-benchmark.md`](testing/omnivoice-quality-benchmark.md), then run:

```powershell
python engine/quality_benchmark.py run <private-corpus.json> --output <local-results-dir>
python engine/quality_benchmark.py blind --output <local-results-dir> --seed 42
python engine/quality_benchmark.py record --output <local-results-dir> --trial <trial-id> --winner a
python engine/quality_benchmark.py summarize --output <local-results-dir>
```

Keep the resulting references, generated files, and ratings local. A default or automatic
composite change needs at least 60% matched blind preference and no failure increase.
