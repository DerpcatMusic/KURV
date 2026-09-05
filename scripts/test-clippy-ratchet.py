#!/usr/bin/env python3
"""Negative controls for the actual command-line CI gate; no Cargo needed."""
import json
from pathlib import Path
import subprocess
import sys
import unittest

GATE = Path(__file__).with_name("clippy-ratchet.py")


class GateTests(unittest.TestCase):
    def run_gate(self, messages):
        return subprocess.run(
            [sys.executable, str(GATE)],
            input="".join(json.dumps(message) + "\n" for message in messages),
            text=True, capture_output=True, check=False,
        ).returncode

    def test_successful_empty_warning_set(self):
        self.assertEqual(self.run_gate([{"reason": "build-finished", "success": True}]), 0)

    def test_failed_or_truncated_builds_never_pass(self):
        for messages in [[], [{"reason": "compiler-artifact"}],
                         [{"reason": "build-finished", "success": False}],
                         [{"reason": "build-finished"}],
                         [[], {"reason": "build-finished", "success": True}],
                         [{"reason": "compiler-message", "message": {"level": "error"}},
                          {"reason": "build-finished", "success": True}]]:
            with self.subTest(messages=messages):
                self.assertNotEqual(self.run_gate(messages), 0)

    def test_warning_budget_and_duplicate_messages(self):
        baseline = int(GATE.with_name("clippy-baseline.txt").read_text().split("#", 1)[0].strip())
        def warning(line):
            return {"reason": "compiler-message", "message": {
                "level": "warning", "code": {"code": "clippy::test_control"},
                "spans": [{"is_primary": True, "file_name": "src/control.rs",
                           "line_start": line, "column_start": 1}]}}
        end = {"reason": "build-finished", "success": True}
        self.assertEqual(self.run_gate([warning(1)] * (baseline + 1) + [end]), 0)
        self.assertNotEqual(self.run_gate([warning(n) for n in range(baseline + 1)] + [end]), 0)


if __name__ == "__main__":
    unittest.main()
