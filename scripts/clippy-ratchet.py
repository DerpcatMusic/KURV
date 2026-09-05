#!/usr/bin/env python3
"""Count clippy/rustc warnings, deduplicated, and compare against a baseline.

The crate enables all of clippy's pedantic and nursery groups. That is
deliberate -- they surface real findings -- but the backlog is far too large to
gate on `-D warnings`, and silencing whole groups to make a green gate possible
would throw away exactly the signal the groups exist to provide. So the gate is
a ratchet instead: the deny-level lints (correctness, suspicious,
undocumented_unsafe_blocks, ...) are hard errors and must stay at zero, while
the warn-level backlog is only allowed to shrink.

Reads `cargo ... --message-format json` on stdin. Warnings are deduplicated by
(lint code, file, line, column) because the crate builds as cdylib, staticlib
and rlib, so rustc lints every source file three times over.
"""

import json
import sys
from pathlib import Path

BASELINE = Path(__file__).with_name("clippy-baseline.txt")


def main() -> int:
    seen = set()
    finished = False
    failed = False
    for line in sys.stdin:
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            print("malformed Cargo diagnostics", file=sys.stderr)
            return 1
        if not isinstance(msg, dict):
            return 1
        if msg.get("reason") == "build-finished":
            finished = True
            failed |= msg.get("success") is not True
            continue
        if msg.get("reason") != "compiler-message":
            continue
        diag = msg["message"]
        failed |= diag.get("level") in {"error", "failure-note"}
        if diag.get("level") != "warning":
            continue
        code = (diag.get("code") or {}).get("code")
        if code is None:
            # Summary lines ("generated N warnings") carry no code.
            continue
        primary = next(
            (s for s in diag.get("spans", []) if s.get("is_primary")),
            None,
        )
        where = (
            (primary["file_name"], primary["line_start"], primary["column_start"])
            if primary
            else (diag.get("message"), 0, 0)
        )
        seen.add((code, *where))

    if not finished or failed:
        print("Cargo did not finish successfully; refusing incomplete diagnostics", file=sys.stderr)
        return 1
    count = len(seen)
    baseline = int(BASELINE.read_text().split("#", 1)[0].strip())

    if count > baseline:
        print(
            f"clippy ratchet: {count} warnings, baseline {baseline}. "
            f"{count - baseline} new warning(s) -- fix them or, if they are "
            f"unavoidable, raise the baseline in {BASELINE.name} in the same "
            f"commit with a note saying why.",
            file=sys.stderr,
        )
        return 1

    if count < baseline:
        # Not a failure: a green build should never depend on remembering to
        # edit a counter. But say it loudly, because the gain is not locked in
        # until the baseline moves.
        print(
            f"clippy ratchet: {count} warnings, down from {baseline}. "
            f"Lower the baseline in {BASELINE.name} to lock the gain in."
        )
        return 0

    print(f"clippy ratchet: {count} warnings, matching the baseline.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
