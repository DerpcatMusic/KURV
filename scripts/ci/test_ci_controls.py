#!/usr/bin/env python3
import copy
from pathlib import Path
import tempfile
import unittest
from check_build_inputs import missing_inputs
from require_success import EXPECTED, check

class Controls(unittest.TestCase):
    def test_required_jobs_cannot_disappear_or_skip(self):
        good = {name: {'result': 'success'} for name in EXPECTED}
        self.assertTrue(check(good))
        for name in EXPECTED:
            for status in ('failure', 'cancelled', 'skipped', None):
                bad = copy.deepcopy(good)
                bad[name]['result'] = status
                self.assertFalse(check(bad), (name, status))
            bad = copy.deepcopy(good)
            del bad[name]
            self.assertFalse(check(bad))
        self.assertFalse(check({}))

    def test_optional_and_transitive_paths_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'Cargo.toml').write_text('[dependencies]\nprivate = {path="private", optional=true}\n[patch.crates-io]\npatched = {path="vendor/patched"}\n')
            missing = missing_inputs(root)
            self.assertIn(root/'private/Cargo.toml', missing)
            self.assertIn(root/'vendor/patched/Cargo.toml', missing)
            self.assertIn(root/'src/licensing/mod.rs', missing)
            (root/'private').mkdir()
            (root/'private/Cargo.toml').write_text('[dependencies]\nchild = {path="../child"}\n')
            self.assertIn(root/'child/Cargo.toml', missing_inputs(root))

if __name__ == '__main__':
    unittest.main()
