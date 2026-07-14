"""Dependency-free wire tests for the managed OmniVoice server."""

import unittest

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


if __name__ == "__main__":
    unittest.main()
