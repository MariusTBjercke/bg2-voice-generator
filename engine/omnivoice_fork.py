#!/usr/bin/env python3
"""Performance fork of ``omnivoice.OmniVoice`` for BG2 Voice Generator.

A subclass override of the installed package's generation internals (v0.1.5) —
the package itself is never modified. See docs/OMNIVOICE-PERF.md for the audit.

Safety: every entry point degrades to stock, never to a crash:

- ``OMNIVOICE_STOCK_GENERATE=1`` forces the stock class outright.
- An import failure here (upstream renamed a helper) makes the server loader
  fall back to stock.
- A runtime exception inside the fast path disables the fork for the rest of the
  process and re-runs that batch through stock.
"""

import math
import os
import sys
import traceback
from time import perf_counter

import torch

from omnivoice import OmniVoice
from omnivoice.models.omnivoice import (
    _filter_top_k,
    _get_time_steps,
    _gumbel_sample,
)


def stock_requested() -> bool:
    val = os.environ.get("OMNIVOICE_STOCK_GENERATE", "").strip()
    return val not in ("", "0")


def load_model(model_id: str, device: str, dtype):
    """Load the patched model, or stock when forced/failed. Returns
    ``(model, fork_active)``."""
    if stock_requested():
        print(
            "[omnivoice-fork] OMNIVOICE_STOCK_GENERATE set — using stock generation",
            file=sys.stderr,
            flush=True,
        )
        return OmniVoice.from_pretrained(model_id, device_map=device, dtype=dtype), False
    try:
        model = PatchedOmniVoice.from_pretrained(model_id, device_map=device, dtype=dtype)
        print("[omnivoice-fork] patched generation active", file=sys.stderr, flush=True)
        return model, True
    except Exception:  # noqa: BLE001 — any load failure degrades to stock
        traceback.print_exc(file=sys.stderr)
        print(
            "[omnivoice-fork] failed to load patched model; using stock",
            file=sys.stderr,
            flush=True,
        )
        return OmniVoice.from_pretrained(model_id, device_map=device, dtype=dtype), False


class PatchedOmniVoice(OmniVoice):
    """OmniVoice with a faster ``_generate_iterative`` and per-phase timing."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._fork_disabled = False
        self._fork_phase = {}

    def _accumulate(self, phase: str, seconds: float) -> None:
        self._fork_phase[phase] = self._fork_phase.get(phase, 0.0) + seconds

    def pop_fork_timings(self) -> dict:
        t, self._fork_phase = self._fork_phase, {}
        return t

    def _preprocess_all(self, *args, **kwargs):
        t0 = perf_counter()
        try:
            return super()._preprocess_all(*args, **kwargs)
        finally:
            self._accumulate("prep", perf_counter() - t0)

    def _decode_and_post_process(self, *args, **kwargs):
        t0 = perf_counter()
        try:
            return super()._decode_and_post_process(*args, **kwargs)
        finally:
            self._accumulate("decode", perf_counter() - t0)

    def _generate_iterative(self, task, gen_config):
        t0 = perf_counter()
        try:
            if self._fork_disabled:
                return super()._generate_iterative(task, gen_config)
            try:
                return self._generate_iterative_fast(task, gen_config)
            except Exception:  # noqa: BLE001 — degrade, never crash a render
                traceback.print_exc(file=sys.stderr)
                print(
                    "[omnivoice-fork] patched generation failed — falling back to "
                    "stock for the rest of this process",
                    file=sys.stderr,
                    flush=True,
                )
                self._fork_disabled = True
                return super()._generate_iterative(task, gen_config)
        finally:
            if torch.cuda.is_available():
                torch.cuda.synchronize()
            self._accumulate("diffusion", perf_counter() - t0)

    def _fork_forward(self, input_ids, audio_mask, attention_mask):
        inputs_embeds = self._prepare_embed_inputs(input_ids, audio_mask)
        out = self.llm(
            inputs_embeds=inputs_embeds,
            attention_mask=attention_mask,
            return_dict=True,
        )
        return out[0]

    def _fork_head(self, hidden):
        b, t, _ = hidden.shape
        logits = self.audio_heads(hidden)
        return (
            logits.view(b, t, self.config.num_audio_codebook, self.config.audio_vocab_size)
            .permute(0, 2, 1, 3)
            .to(torch.float32)
        )

    def _generate_iterative_fast(self, task, gen_config):
        B = task.batch_size
        C = self.config.num_audio_codebook
        mask_id = self.config.audio_mask_id
        device = self.device

        inputs_list = [
            self._prepare_inference_inputs(
                task.texts[i],
                task.target_lens[i],
                task.ref_texts[i],
                task.ref_audio_tokens[i],
                task.langs[i],
                task.instructs[i],
                gen_config.denoise,
            )
            for i in range(B)
        ]

        c_lens = [inp["input_ids"].size(2) for inp in inputs_list]
        t_lens = list(task.target_lens)
        max_c_len = max(c_lens)
        max_t = max(t_lens)

        cond_ids = torch.full((B, C, max_c_len), mask_id, dtype=torch.long, device=device)
        cond_amask = torch.zeros((B, max_c_len), dtype=torch.bool, device=device)
        cond_attn = torch.zeros((B, 1, max_c_len, max_c_len), dtype=torch.bool, device=device)
        unc_ids = torch.full((B, C, max_t), mask_id, dtype=torch.long, device=device)
        unc_amask = torch.zeros((B, max_t), dtype=torch.bool, device=device)
        unc_attn = torch.zeros((B, 1, max_t, max_t), dtype=torch.bool, device=device)

        for i, inp in enumerate(inputs_list):
            c_len, t_len = c_lens[i], t_lens[i]
            cond_ids[i, :, :c_len] = inp["input_ids"]
            cond_amask[i, :c_len] = inp["audio_mask"]
            cond_attn[i, :, :c_len, :c_len] = True
            if max_c_len > c_len:
                pad_diag = torch.arange(c_len, max_c_len, device=device)
                cond_attn[i, :, pad_diag, pad_diag] = True
            unc_ids[i, :, :t_len] = inp["input_ids"][..., -t_len:]
            unc_amask[i, :t_len] = inp["audio_mask"][..., -t_len:]
            unc_attn[i, :, :t_len, :t_len] = True
            if max_t > t_len:
                pad_diag = torch.arange(t_len, max_t, device=device)
                unc_attn[i, :, pad_diag, pad_diag] = True

        cond_tgt_idx = torch.zeros((B, max_t), dtype=torch.long, device=device)
        for i in range(B):
            cond_tgt_idx[i, : t_lens[i]] = torch.arange(
                c_lens[i] - t_lens[i], c_lens[i], device=device
            )

        valid_pos = (
            torch.arange(max_t, device=device)[None, :]
            < torch.tensor(t_lens, device=device)[:, None]
        ).unsqueeze(1)

        tokens = torch.full((B, C, max_t), mask_id, dtype=torch.long, device=device)

        timesteps = _get_time_steps(
            t_start=0.0,
            t_end=1.0,
            num_step=gen_config.num_step,
            t_shift=gen_config.t_shift,
        ).tolist()
        schedules = []
        for t_len in t_lens:
            total_mask = t_len * C
            rem = total_mask
            sched = []
            for step in range(gen_config.num_step):
                num = (
                    rem
                    if step == gen_config.num_step - 1
                    else min(
                        math.ceil(total_mask * (timesteps[step + 1] - timesteps[step])),
                        rem,
                    )
                )
                sched.append(int(num))
                rem -= int(num)
            schedules.append(sched)

        layer_ids = torch.arange(C, device=device).view(1, -1, 1)
        hidden_size = self.config.llm_config.hidden_size
        gather_idx = cond_tgt_idx.unsqueeze(-1).expand(B, max_t, hidden_size)
        use_cfg = gen_config.guidance_scale != 0

        for step in range(gen_config.num_step):
            c_hidden = self._fork_forward(cond_ids, cond_amask, cond_attn)
            c_tgt = torch.gather(c_hidden, 1, gather_idx)
            c_logits = self._fork_head(c_tgt)

            if use_cfg:
                u_hidden = self._fork_forward(unc_ids, unc_amask, unc_attn)
                u_logits = self._fork_head(u_hidden)
                c_lp = torch.nn.functional.log_softmax(c_logits, dim=-1)
                u_lp = torch.nn.functional.log_softmax(u_logits, dim=-1)
                log_probs = torch.nn.functional.log_softmax(
                    c_lp + gen_config.guidance_scale * (c_lp - u_lp), dim=-1
                )
            else:
                log_probs = torch.nn.functional.log_softmax(c_logits, dim=-1)

            log_probs[..., mask_id] = -float("inf")

            if gen_config.class_temperature > 0.0:
                filtered = _filter_top_k(log_probs, ratio=0.1)
                pred_tokens = _gumbel_sample(filtered, gen_config.class_temperature).argmax(
                    dim=-1
                )
            else:
                pred_tokens = log_probs.argmax(dim=-1)

            scores = log_probs.max(dim=-1)[0]
            scores = scores - (layer_ids * gen_config.layer_penalty_factor)
            if gen_config.position_temperature > 0.0:
                scores = _gumbel_sample(scores, gen_config.position_temperature)
            scores = scores.masked_fill(~valid_pos, -float("inf"))
            scores = scores.masked_fill(tokens != mask_id, -float("inf"))

            for i in range(B):
                k = schedules[i][step]
                if k <= 0:
                    continue
                t_len = t_lens[i]
                _, topk_idx = torch.topk(scores[i, :, :t_len].flatten(), k)
                flat_tokens = tokens[i, :, :t_len].flatten()
                flat_tokens[topk_idx] = pred_tokens[i, :, :t_len].flatten()[topk_idx]
                tokens[i, :, :t_len] = flat_tokens.view(C, t_len)
                cond_ids[i, :, c_lens[i] - t_len : c_lens[i]] = tokens[i, :, :t_len]
            unc_ids.copy_(tokens)

        return [tokens[i, :, : t_lens[i]] for i in range(B)]
