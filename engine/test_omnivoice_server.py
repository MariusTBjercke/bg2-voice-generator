"""Dependency-free wire tests for the managed OmniVoice server."""

import unittest
import sys
import types
from unittest import mock

from engine import omnivoice_server as server


class RenderSettingsWireTests(unittest.TestCase):
    def test_every_request_setting_reaches_generate_kwargs(self):
        request = {
            "speed": 1.15,
            "num_steps": 48,
            "guidance_scale": 2.5,
            "t_shift": 0.2,
            "layer_penalty_factor": 4.0,
            "position_temperature": 6.0,
            "class_temperature": 0.3,
            "prompt_denoise": False,
            "preprocess_prompt": False,
            "postprocess_output": False,
            "audio_chunk_duration": 12.0,
            "audio_chunk_threshold": 25.0,
        }
        self.assertEqual(
            server._generate_kwargs(request),
            {
                "speed": 1.15,
                "num_step": 48,
                "guidance_scale": 2.5,
                "t_shift": 0.2,
                "layer_penalty_factor": 4.0,
                "position_temperature": 6.0,
                "class_temperature": 0.3,
                "denoise": False,
                "preprocess_prompt": False,
                "postprocess_output": False,
                "audio_chunk_duration": 12.0,
                "audio_chunk_threshold": 25.0,
            },
        )

    def test_absent_values_preserve_bg2_defaults(self):
        kwargs = server._generate_kwargs({})
        self.assertIsNone(kwargs["speed"])
        self.assertEqual(kwargs["num_step"], 32)
        self.assertEqual(kwargs["guidance_scale"], 2.0)
        self.assertEqual(kwargs["t_shift"], 0.1)
        self.assertTrue(kwargs["denoise"])
        self.assertTrue(kwargs["preprocess_prompt"])
        self.assertTrue(kwargs["postprocess_output"])


class VoiceDesignWireTests(unittest.TestCase):
    def test_design_forwards_text_instruct_and_explicit_seed(self):
        calls = []

        class Model:
            sampling_rate = 24000

            def generate(self, text, instruct):
                calls.append((text, instruct))
                return [0.25]

        fake_torch = types.SimpleNamespace(no_grad=lambda: mock.MagicMock(__enter__=lambda self: None, __exit__=lambda self, *args: None))
        with mock.patch.dict(sys.modules, {"torch": fake_torch}), \
             mock.patch.object(server, "_load_model", return_value=Model()), \
             mock.patch.object(server, "_seed_rng") as seeded, \
             mock.patch.object(server, "_write_wav", return_value=6.2) as write:
            result = server._design_voice({
                "text": "A new road awaits.",
                "instruct": "female, young adult, high pitch, british accent",
                "out_path": "candidate.wav",
                "target_sample_rate": 22050,
                "seed": 137,
            })

        self.assertEqual(calls, [("A new road awaits.", "female, young adult, high pitch, british accent")])
        seeded.assert_called_once_with(137)
        write.assert_called_once()
        self.assertEqual(result["duration"], 6.2)

    def test_design_rejects_an_engine_without_instruct_support(self):
        class LegacyModel:
            def generate(self, text):
                return [0.25]

        fake_torch = types.SimpleNamespace(no_grad=mock.MagicMock())
        with mock.patch.dict(sys.modules, {"torch": fake_torch}), \
             mock.patch.object(server, "_load_model", return_value=LegacyModel()):
            with self.assertRaisesRegex(RuntimeError, "does not support voice design"):
                server._design_voice({"text": "Hello", "instruct": "male", "out_path": "x.wav"})


if __name__ == "__main__":
    unittest.main()
