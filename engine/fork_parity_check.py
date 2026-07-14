#!/usr/bin/env python3
"""Parity check for engine/omnivoice_fork.py against the stock package.

Run after bumping ``omnivoice`` to prove the fork still produces the same tokens
as stock before trusting it with a render.

Usage:
    python engine/fork_parity_check.py <ref_audio> <ref_text…>
"""

import sys

import torch

from omnivoice import OmniVoice
from omnivoice.models.omnivoice import OmniVoiceGenerationConfig
from omnivoice_fork import PatchedOmniVoice

MODEL_ID = "k2-fsa/OmniVoice"

TEXTS = [
    "Welcome to Athkatla, traveler.",
    "The Flaming Fist has work for you, if you think you can handle yourself in a fight.",
]


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    ref_audio = sys.argv[1]
    ref_text = " ".join(sys.argv[2:])

    print("loading model on CPU (fp32)…", flush=True)
    model = PatchedOmniVoice.from_pretrained(MODEL_ID, device_map="cpu", dtype=torch.float32)

    print("encoding reference prompt…", flush=True)
    prompt = model.create_voice_clone_prompt(
        ref_audio=ref_audio, ref_text=ref_text, preprocess_prompt=True
    )

    cfg = OmniVoiceGenerationConfig(
        num_step=4,
        guidance_scale=2.0,
        position_temperature=0.0,
        class_temperature=0.0,
    )
    task = model._preprocess_all(
        text=TEXTS, language="en", voice_clone_prompt=prompt, preprocess_prompt=True
    )

    print("running fork path…", flush=True)
    torch.manual_seed(42)
    with torch.no_grad():
        fast = model._generate_iterative_fast(task, cfg)

    print("running stock path…", flush=True)
    torch.manual_seed(42)
    with torch.no_grad():
        stock = OmniVoice._generate_iterative(model, task, cfg)

    ok = True
    for i, (f, s) in enumerate(zip(fast, stock)):
        if f.shape != s.shape:
            print(f"item {i}: SHAPE MISMATCH {tuple(f.shape)} vs {tuple(s.shape)}")
            ok = False
            continue
        match = (f == s).float().mean().item()
        print(
            f"item {i}: shape {tuple(f.shape)} exact={torch.equal(f, s)} "
            f"token match={match:.4f}"
        )
        ok &= match >= 0.99

    print("smoke: full generate() through the patched override…", flush=True)
    with torch.no_grad():
        audios = model.generate(
            text=[TEXTS[0]],
            language="en",
            voice_clone_prompt=prompt,
            num_step=2,
            position_temperature=0.0,
        )
    assert len(audios) == 1 and audios[0].ndim == 1 and len(audios[0]) > 0
    phases = model.pop_fork_timings()
    print(f"smoke OK ({len(audios[0])} samples); fork phases: {phases}")
    assert model._fork_disabled is False, "fast path silently fell back to stock"

    print("PARITY OK" if ok else "PARITY FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
