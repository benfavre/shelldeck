#!/usr/bin/env python3
"""SDTEST-2876: native performance measurements must use production GPUI semantics."""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]

class NativeBenchmarkGraph(unittest.TestCase):
    def test_sdtest_2876_preview_uses_production_gpui_without_test_support(self):
        result = subprocess.run([
            "cargo", "tree", "--locked", "--offline", "-p", "shelldeck",
            "--features", "agent-preview", "-e", "features", "-i", "adabraka-gpui",
        ], cwd=ROOT, text=True, capture_output=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("adabraka-gpui v", result.stdout)
        self.assertNotIn('adabraka-gpui feature "test-support"', result.stdout,
                         "Test-support forces eager redraws and invalidates native FPS measurements")

if __name__ == "__main__":
    unittest.main()
