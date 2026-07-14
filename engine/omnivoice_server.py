"""OmniVoice engine server (item-08, performance stack).

The Rust backend (`src-tauri/src/tts`) launches this as a managed subprocess and
talks to it over HTTP on a fixed local port:

  * ``GET  /health``           - liveness + readiness + device/fork diagnostics.
  * ``POST /synthesize``       - zero-shot voice clone: {text, ref_audio, ref_text, out_path}
                                 -> writes a mono 16-bit PCM WAV at ``out_path``.
  * ``POST /synthesize_batch`` - batched clone against ONE shared reference.
  * ``POST /vad_batch``        - neural speech verification (Silero VAD).

The heavy OmniVoice model is imported LAZILY on the first synthesis call so this
script still starts and answers ``/health`` on a machine without deps installed.

Run standalone for a smoke test:

    python engine/omnivoice_server.py --port 8140

Env vars:
    OMNIVOICE_TTS_PORT       - listen port (default 8140)
    OMNIVOICE_DEVICE         - "cuda:0" (default) or "cpu"
    OMNIVOICE_MODEL_ID       - HF repo id (default k2-fsa/OmniVoice)
    OMNIVOICE_STOCK_GENERATE - "1" forces stock generation (no fork)
"""

from __future__ import annotations

import argparse
import gc
import json
import os
import sys
import threading
import wave
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from time import perf_counter

ENGINE = "omnivoice"
VERSION = "0.1.0"
MODEL_ID = os.environ.get("OMNIVOICE_MODEL_ID", "k2-fsa/OmniVoice")
DEVICE = os.environ.get("OMNIVOICE_DEVICE", "cuda:0").strip()
LANGUAGE = os.environ.get("OMNIVOICE_LANGUAGE", "en").strip() or None
TARGET_SAMPLE_RATE = 22050
NATIVE_SAMPLE_RATE = 24000

GEN_NUM_STEP = 32
GEN_GUIDANCE_SCALE = 2.0
GEN_T_SHIFT = 0.1
GEN_LAYER_PENALTY_FACTOR = 5.0
GEN_POSITION_TEMPERATURE = 5.0
GEN_CLASS_TEMPERATURE = 0.0
GEN_DENOISE = True
GEN_PREPROCESS_PROMPT = True
GEN_POSTPROCESS_OUTPUT = True
GEN_CHUNK_DURATION = 10.0
GEN_CHUNK_THRESHOLD = 30.0
GEN_PEAK_NORMALIZE_DBFS = -1.0
GEN_SEED = 42


def _generate_kwargs(req: dict) -> dict:
    """Resolve every render knob from one validated Rust request.

    Defaults keep direct/manual callers compatible, but normal app traffic sends
    the full shape so single and batch requests are identical.
    """
    return {
        "speed": req.get("speed"),
        "num_step": int(req.get("num_steps", GEN_NUM_STEP)),
        "guidance_scale": float(req.get("guidance_scale", GEN_GUIDANCE_SCALE)),
        "t_shift": float(req.get("t_shift", GEN_T_SHIFT)),
        "layer_penalty_factor": float(
            req.get("layer_penalty_factor", GEN_LAYER_PENALTY_FACTOR)
        ),
        "position_temperature": float(
            req.get("position_temperature", GEN_POSITION_TEMPERATURE)
        ),
        "class_temperature": float(
            req.get("class_temperature", GEN_CLASS_TEMPERATURE)
        ),
        "denoise": bool(req.get("prompt_denoise", GEN_DENOISE)),
        "preprocess_prompt": bool(
            req.get("preprocess_prompt", GEN_PREPROCESS_PROMPT)
        ),
        "postprocess_output": bool(
            req.get("postprocess_output", GEN_POSTPROCESS_OUTPUT)
        ),
        "audio_chunk_duration": float(
            req.get("audio_chunk_duration", GEN_CHUNK_DURATION)
        ),
        "audio_chunk_threshold": float(
            req.get("audio_chunk_threshold", GEN_CHUNK_THRESHOLD)
        ),
    }


def _resolve_seed(req: dict) -> int:
    seed = req.get("seed")
    if seed is None:
        return GEN_SEED
    seed = int(seed)
    return int.from_bytes(os.urandom(4), "big") if seed < 0 else seed


def _seed_rng(seed: int) -> None:
    try:
        import torch

        torch.manual_seed(seed)
        if torch.cuda.is_available():
            torch.cuda.manual_seed_all(seed)
    except Exception:  # noqa: BLE001
        pass


_LOCK = threading.Lock()
_MODEL = None
_MODEL_IS_FORK = False
_LOAD_ERROR: str | None = None

_PROMPT_CACHE: OrderedDict[tuple, object] = OrderedDict()
_PROMPT_CACHE_MAX = 32

_VAD_LOCK = threading.Lock()
_VAD_MODEL = None
VAD_SAMPLE_RATE = 16000


def _load_model():
    """Import + load the OmniVoice model on first use."""
    global _MODEL, _MODEL_IS_FORK, _LOAD_ERROR
    if _MODEL is not None:
        return _MODEL
    try:
        import torch

        from omnivoice_fork import load_model

        _MODEL, _MODEL_IS_FORK = load_model(MODEL_ID, DEVICE, torch.float16)
        _LOAD_ERROR = None
        return _MODEL
    except Exception as exc:  # noqa: BLE001
        _LOAD_ERROR = f"{type(exc).__name__}: {exc}"
        raise


def _device_str() -> str:
    try:
        import torch

        if torch.cuda.is_available():
            return DEVICE
    except Exception:  # noqa: BLE001
        pass
    return "cpu"


def _cuda_name() -> str | None:
    try:
        import torch

        if torch.cuda.is_available():
            return torch.cuda.get_device_name(0)
    except Exception:  # noqa: BLE001
        pass
    return None


def _get_clone_prompt(ref_audio: str, ref_text: str, preprocess: bool):
    """Return a cached VoiceClonePrompt. MUST be called with ``_LOCK`` held."""
    path = os.path.abspath(ref_audio)
    st = os.stat(path)
    key = (path, st.st_mtime_ns, st.st_size, ref_text, bool(preprocess))
    cached = _PROMPT_CACHE.get(key)
    if cached is not None:
        _PROMPT_CACHE.move_to_end(key)
        return cached, True
    prompt = _MODEL.create_voice_clone_prompt(
        ref_audio=path,
        ref_text=ref_text,
        preprocess_prompt=preprocess,
    )
    _PROMPT_CACHE[key] = prompt
    while len(_PROMPT_CACHE) > _PROMPT_CACHE_MAX:
        _PROMPT_CACHE.popitem(last=False)
    return prompt, False


def _to_wave(obj):
    import numpy as np

    if isinstance(obj, (list, tuple)):
        arrs = [np.asarray(x, dtype=np.float32).reshape(-1) for x in obj]
        if not arrs:
            return np.zeros(0, dtype=np.float32)
        return np.concatenate(arrs) if len(arrs) > 1 else arrs[0]
    try:
        import torch

        if isinstance(obj, torch.Tensor):
            obj = obj.detach().cpu().float().numpy()
    except Exception:  # noqa: BLE001
        pass
    return np.asarray(obj, dtype=np.float32).reshape(-1)


def _peak_normalize(arr, dbfs: float | None):
    import numpy as np

    if dbfs is None or arr.size == 0:
        return arr
    peak = float(np.max(np.abs(arr))) or 1.0
    return arr * (10 ** (dbfs / 20.0) / peak)


_warned_no_resampler = False


def _resample(arr, src_sr: int, dst_sr: int):
    global _warned_no_resampler
    import numpy as np

    if not dst_sr or src_sr == dst_sr:
        return arr, src_sr
    try:
        from math import gcd

        from scipy.signal import resample_poly

        g = gcd(int(dst_sr), int(src_sr))
        up, down = int(dst_sr) // g, int(src_sr) // g
        return resample_poly(arr, up, down).astype(np.float32), dst_sr
    except Exception:  # noqa: BLE001
        pass
    try:
        import librosa

        out = librosa.resample(arr.astype(np.float32), orig_sr=src_sr, target_sr=dst_sr)
        return out.astype(np.float32), dst_sr
    except Exception:  # noqa: BLE001
        if not _warned_no_resampler:
            print(f"WARNING: no resampler (scipy/librosa); emitting at native {src_sr} Hz")
            _warned_no_resampler = True
        return arr, src_sr


def _native_rate(model) -> int:
    return int(getattr(model, "sampling_rate", None) or NATIVE_SAMPLE_RATE)


def _write_wav(
    out_path: str,
    samples,
    native_rate: int,
    target_rate: int,
    peak_normalize_dbfs: float | None = GEN_PEAK_NORMALIZE_DBFS,
) -> float:
    import numpy as np

    arr = _to_wave(samples)
    if arr.size == 0:
        raise ValueError("model produced empty audio (0 samples)")
    arr = _peak_normalize(arr, peak_normalize_dbfs)
    arr, out_sr = _resample(arr, native_rate, target_rate)
    pcm = (np.clip(arr, -1.0, 1.0) * 32767.0).astype("<i2").tobytes()
    os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
    tmp = out_path + ".part"
    with wave.open(tmp, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(out_sr)
        w.writeframes(pcm)
    os.replace(tmp, out_path)
    return len(arr) / float(out_sr) if out_sr else 0.0


def _run_generate(texts, ref_audio: str, ref_text: str, req: dict) -> list:
    """Run model.generate for one or more texts; return CPU waveforms."""
    import torch

    texts = list(texts)
    with _LOCK:
        model = _load_model()
        _seed_rng(_resolve_seed(req))
        print(f"[omnivoice] synthesizing batch of {len(texts)}…", file=sys.stderr, flush=True)

        t0 = perf_counter()
        preprocess_prompt = bool(req.get("preprocess_prompt", GEN_PREPROCESS_PROMPT))
        prompt, cache_hit = _get_clone_prompt(ref_audio, ref_text, preprocess_prompt)
        t_prompt = perf_counter() - t0

        if torch.cuda.is_available():
            torch.cuda.reset_peak_memory_stats(0)

        t0 = perf_counter()
        with torch.no_grad():
            audios = model.generate(
                text=texts,
                language=LANGUAGE,
                voice_clone_prompt=prompt,
                **_generate_kwargs(req),
            )
        t_generate = perf_counter() - t0

        inner = ""
        pop_timings = getattr(model, "pop_fork_timings", None)
        if pop_timings is not None:
            phases = pop_timings()
            if phases:
                inner = " [" + " ".join(
                    f"{k}={v:.2f}s" for k, v in sorted(phases.items())
                ) + "]"
        print(
            f"[omnivoice] batch of {len(texts)} timings: "
            f"prompt={t_prompt:.2f}s ({'hit' if cache_hit else 'miss'}) "
            f"generate={t_generate:.2f}s{inner}",
            file=sys.stderr,
            flush=True,
        )

        waves = [_to_wave(a) for a in audios]
        del audios
        if torch.cuda.is_available():
            try:
                peak = torch.cuda.max_memory_allocated(0) / 1e9
                peak_resv = torch.cuda.max_memory_reserved(0) / 1e9
                gc.collect()
                torch.cuda.empty_cache()
                alloc = torch.cuda.memory_allocated(0) / 1e9
                resv = torch.cuda.memory_reserved(0) / 1e9
                free, total = torch.cuda.mem_get_info()
                print(
                    f"[omnivoice] VRAM after batch of {len(waves)}: "
                    f"peak={peak:.2f}G (reserved {peak_resv:.2f}G) "
                    f"floor allocated={alloc:.2f}G reserved={resv:.2f}G "
                    f"gpu_free={free / 1e9:.2f}G/{total / 1e9:.2f}G",
                    file=sys.stderr,
                    flush=True,
                )
            except Exception:  # noqa: BLE001
                pass
    return waves


def _synthesize(req: dict) -> dict:
    text = req["text"]
    ref_audio = os.path.abspath(req["ref_audio"])
    ref_text = req.get("ref_text") or ""
    out_path = req["out_path"]
    sample_rate = int(req.get("target_sample_rate") or TARGET_SAMPLE_RATE)
    waves = _run_generate([text], ref_audio, ref_text, req)
    native_rate = _native_rate(_MODEL)
    duration = _write_wav(
        out_path,
        waves[0],
        native_rate,
        sample_rate,
        req.get("peak_normalize_dbfs", GEN_PEAK_NORMALIZE_DBFS),
    )
    return {"sample_rate": sample_rate, "duration": duration, "written": True}


def _synthesize_batch(req: dict) -> dict:
    ref_audio = os.path.abspath(req["ref_audio"])
    ref_text = req.get("ref_text") or ""
    sample_rate = int(req.get("target_sample_rate") or TARGET_SAMPLE_RATE)
    items = req.get("items") or []
    if not items:
        return {"sample_rate": sample_rate, "items": []}
    texts = [it["text"] for it in items]
    waves = _run_generate(texts, ref_audio, ref_text, req)
    if len(waves) != len(items):
        raise ValueError(
            f"batch mismatch: {len(items)} texts in, {len(waves)} waves out"
        )
    native_rate = _native_rate(_MODEL)

    def _write_one(pair):
        it, wave_f32 = pair
        duration = _write_wav(
            it["out_path"],
            wave_f32,
            native_rate,
            sample_rate,
            req.get("peak_normalize_dbfs", GEN_PEAK_NORMALIZE_DBFS),
        )
        return {"out_path": it["out_path"], "duration": duration, "written": True}

    t0 = perf_counter()
    with ThreadPoolExecutor(max_workers=min(4, max(1, len(waves)))) as pool:
        results = list(pool.map(_write_one, zip(items, waves)))
    print(
        f"[omnivoice] batch of {len(waves)}: encode+write={perf_counter() - t0:.2f}s",
        file=sys.stderr,
        flush=True,
    )
    return {"sample_rate": sample_rate, "items": results}


def _load_vad():
    global _VAD_MODEL
    if _VAD_MODEL is None:
        from silero_vad import load_silero_vad  # type: ignore

        _VAD_MODEL = load_silero_vad()
    return _VAD_MODEL


def _vad_batch(req: dict) -> dict:
    paths = req.get("paths") or []
    with _VAD_LOCK:
        model = _load_vad()
        from silero_vad import get_speech_timestamps, read_audio  # type: ignore

        results = []
        for path in paths:
            try:
                wav = read_audio(os.path.abspath(path), sampling_rate=VAD_SAMPLE_RATE)
                total = len(wav) / float(VAD_SAMPLE_RATE)
                stamps = get_speech_timestamps(
                    wav, model, sampling_rate=VAD_SAMPLE_RATE, return_seconds=True
                )
                speech = sum(s["end"] - s["start"] for s in stamps)
                ratio = min(1.0, speech / total) if total > 0 else 0.0
                results.append(
                    {"path": path, "speech_ratio": ratio, "duration": total, "error": None}
                )
            except Exception as exc:  # noqa: BLE001
                results.append(
                    {
                        "path": path,
                        "speech_ratio": None,
                        "duration": None,
                        "error": f"{type(exc).__name__}: {exc}",
                    }
                )
    return {"sample_rate": VAD_SAMPLE_RATE, "items": results}


class Handler(BaseHTTPRequestHandler):
    def _send_json(self, code: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/health":
            self._send_json(
                200,
                {
                    "engine": ENGINE,
                    "version": VERSION,
                    "status": "ok",
                    "ready": _MODEL is not None,
                    "model_id": MODEL_ID,
                    "device": _device_str(),
                    "cuda_name": _cuda_name(),
                    "fork": _MODEL_IS_FORK,
                    "load_error": _LOAD_ERROR,
                },
            )
        else:
            self._send_json(404, {"error": f"unknown path: {self.path}"})

    def do_POST(self) -> None:  # noqa: N802
        handler = {
            "/synthesize": _synthesize,
            "/synthesize_batch": _synthesize_batch,
            "/vad_batch": _vad_batch,
        }.get(self.path)
        if handler is None:
            self._send_json(404, {"error": f"unknown path: {self.path}"})
            return
        length = int(self.headers.get("Content-Length") or 0)
        try:
            req = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError as exc:
            self._send_json(400, {"error": f"invalid JSON body: {exc}"})
            return
        try:
            self._send_json(200, handler(req))
        except ImportError as exc:
            self._send_json(503, {"error": f"omnivoice not available: {exc}"})
        except Exception as exc:  # noqa: BLE001
            self._send_json(500, {"error": f"{type(exc).__name__}: {exc}"})

    def log_message(self, *args) -> None:
        pass


def main() -> None:
    parser = argparse.ArgumentParser(description="OmniVoice engine server")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=int(os.environ.get("OMNIVOICE_TTS_PORT", "8140")))
    args = parser.parse_args()

    print(
        f"{ENGINE} {VERSION} listening on http://{args.host}:{args.port} "
        f"(device={DEVICE})",
        flush=True,
    )
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
