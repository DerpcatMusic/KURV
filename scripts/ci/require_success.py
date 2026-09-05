#!/usr/bin/env python3
"""Reject failed, cancelled, missing or skipped required jobs."""
import json
import os
import sys

EXPECTED = {'format', 'quality', 'rt-safety', 'dsp-integration'}

def check(needs):
    return (isinstance(needs, dict) and set(needs) == EXPECTED
            and all(isinstance(value, dict) and value.get('result') == 'success'
                    for value in needs.values()))

if __name__ == '__main__':
    needs = json.loads(os.environ['NEEDS_JSON'])
    print(json.dumps(needs, indent=2))
    sys.exit(0 if check(needs) else 'Required CI jobs did not all succeed')
