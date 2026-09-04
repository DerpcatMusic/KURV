import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_performance_matrix as runner


CASE = dict(scenario="solo-saw-1", voices=1, frames=64, sample_rate=48000, oversampling=2)


def output(**overrides):
    values = dict(CASE, callbacks=256, repeats=5, finite="true", peak=0.2,
                  stream_energy=12, stream_sum=0.5, audible_callbacks=1280,
                  median_ns_per_callback=100, p50_ns=95, p95_ns=110, p99_ns=120,
                  p999_ns=150, max_ns=170, deadline_misses=0)
    values.update(overrides)
    return ",".join(f"{key}={value}" for key, value in values.items())+"\n"


class PerformanceMatrixTests(unittest.TestCase):
    def test_full_matrix_covers_required_axes(self):
        cases = list(runner.matrix("full", [64], [48000], [2]))
        self.assertEqual(len(cases), 80)
        self.assertEqual({case["voices"] for case in cases}, {1, 8})
        for lane in [1, 4, 8, 16, 64]:
            for scenario in [f"solo-saw-{lane}", f"xpm-{lane}x{lane}",
                             f"xfm-{lane}x{lane}", f"xdepthpm-{lane}x{lane}", f"xnestedpm-{lane}"]:
                self.assertTrue(any(case["scenario"] == scenario for case in cases))

    def test_valid_result(self):
        parsed = runner.parse_output("diagnostic\n"+output(), CASE)
        self.assertEqual(parsed["p99_ns"], 120)

    def test_reject_nonfinite_silent_and_bad_counts(self):
        for invalid in [dict(finite="false"), dict(peak=0), dict(stream_energy=0),
                        dict(audible_callbacks=0), dict(p99_ns="NaN"), dict(stream_sum="inf"),
                        dict(deadline_misses=-1), dict(deadline_misses=1281),
                        dict(audible_callbacks=1.5), dict(p50_ns=200), dict(callbacks=0),
                        dict(median_ns_per_callback=0), dict(oversampling=4)]:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                runner.parse_output(output(**invalid), CASE)

    def test_reject_missing_new_instrumentation(self):
        with self.assertRaises(ValueError):
            runner.parse_output(output().replace("p999_ns=150,", ""), CASE)

    def test_reject_wrong_scenario_and_duplicate_result(self):
        for bad in [output(scenario="idle"), output()+output(), output().strip()+",peak=1\n"]:
            with self.assertRaises(ValueError):
                runner.parse_output(bad, CASE)

    def test_command_has_independent_notes_lanes_and_oversampling(self):
        self.assertEqual(runner.command(Path("/bin/lab"), CASE, 256, 5),
                         ["/bin/lab", "64", "256", "5", "solo-saw-1", "1", "48000", "2"])

    def test_pair_summary_does_not_hide_tail_or_deadline_regression(self):
        baseline = runner.parse_output(output(), CASE)
        candidate = runner.parse_output(output(median_ns_per_callback=50, p99_ns=130, deadline_misses=2), CASE)
        summary = runner.summarize([dict(baseline=baseline, candidate=candidate)])
        self.assertEqual(summary["median_paired_speedup"], 2)
        self.assertLess(summary["median_p99_speedup"], 1)
        self.assertEqual(summary["deadline_misses"]["candidate"], 2)

    def test_runner_alternates_and_records_binary_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline, candidate = (Path(directory)/name for name in ["baseline", "candidate"])
            baseline.write_bytes(b"baseline fixture")
            candidate.write_bytes(b"candidate fixture")
            destination = Path(directory)/"report.json"
            calls = []
            def fake_run(binary, case, args):
                calls.append(binary.name)
                return runner.parse_output(output(), case)
            args = ["runner", str(baseline), str(candidate), "--rounds", "3", "--output", str(destination)]
            with patch("sys.argv", args), patch.object(runner, "matrix", return_value=iter([CASE])), \
                 patch.object(runner, "run_once", side_effect=fake_run), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(runner.main(), 0)
            self.assertEqual(calls, ["baseline", "candidate", "candidate", "baseline", "baseline", "candidate"])
            report = json.loads(destination.read_text())
            self.assertEqual(len(report["binaries"]["candidate"]["sha256"]), 64)
            self.assertEqual(len(report["results"][0]["pairs"]), 3)

    def test_failed_run_is_saved_and_never_reported_as_speedup(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory)/"fixture"
            binary.write_bytes(b"fixture")
            destination = Path(directory)/"report.json"
            args = ["runner", str(binary), str(binary), "--output", str(destination)]
            with patch("sys.argv", args), patch.object(runner, "matrix", return_value=iter([CASE])), \
                 patch.object(runner, "run_once", side_effect=ValueError("silent workload")), \
                 contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(runner.main(), 1)
            result = json.loads(destination.read_text())["results"][0]
            self.assertIn("error", result)
            self.assertNotIn("summary", result)


if __name__ == "__main__":
    unittest.main()
