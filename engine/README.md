# engine/

The local TTS engine that BG2 Voice Generator drives as a managed subprocess.

This is a **generation-time** dependency only. The voice packs the app exports are
native WeiDU installs and do **not** require this engine, Python, or the app itself to
play (see `docs/adr/0001-native-weidu-export.md`).

## Files

- `omnivoice_server.py` - the HTTP server the Rust `generator`/`audio` modules talk to.
  Includes fp16 loading, voice-clone prompt caching, VRAM reclaim, and the performance
  fork (`omnivoice_fork.py`).
- `omnivoice_fork.py` - math-preserving override of OmniVoice generation internals
  (~1.6× faster diffusion on GPU). Set `OMNIVOICE_STOCK_GENERATE=1` to disable.
- `fork_parity_check.py` - verifies fork output matches stock (run after bumping
  `omnivoice` in `requirements-omnivoice.txt`).
- `requirements-omnivoice.txt` - the pinned Python dependencies installed into the
  `engine-runtime/venv` on first run.

## Performance tuning

- **Batch size / char budget** — Generation screen settings (defaults: 8 lines, 800
  chars). Do not raise batch size without watching engine stderr `peak=` VRAM logs.
- **Reference clips** — 5–8 s is ideal; clips over 8 s slow every batch for that
  speaker (Binding warns when a clip exceeds 8 s).
- **Diagnostics** — Engine stderr logs per-batch `prompt=` / `generate=` / `encode+write=`
  timings and VRAM peaks. `/health` reports `device`, `cuda_name`, and `fork`.
- **Escape hatch** — `OMNIVOICE_STOCK_GENERATE=1` forces stock OmniVoice generation.

## Layout

In a portable build these files sit next to the exe under `engine/`, and the venv +
models are created on first run in a writable `engine-runtime/` sibling. In dev the
app uses your app-data dir as the runtime root. Resolution lives in `src-tauri/src/paths.rs`.

## Install (the in-app installer)

The engine is provisioned from inside the app: the Generation screen's **Install engine**
button (the `install_engine` command) creates a virtual environment at
`engine-runtime/venv`, installs `requirements-omnivoice.txt` (plus the pinned torch build)
**into that venv** — never into the vendored `tools/python` — downloads the model weights,
and writes an `engine-runtime/venv/.installed` marker on success. On the next engine start
the supervisor spawns the venv interpreter automatically (keyed off that marker); until it
exists the app offers Install rather than Start. Re-running Install on an already-marked
venv is a no-op; delete `engine-runtime/` to force a clean reinstall.
