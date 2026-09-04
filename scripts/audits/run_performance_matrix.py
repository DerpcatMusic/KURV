#!/usr/bin/env python3
"""Compare two process_lab builds, sequentially, with alternating AB/BA pairs.

Both binaries must include this revision's instrumentation/scenarios. Timings are
machine-local evidence, never proof of universal synth superiority. Stdlib only.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
from datetime import datetime, timezone


def matrix(mode, frames, rates, factors):
    lanes = [1, 4, 8, 16, 64] if mode == "full" else [1, 8, 64]
    waves = ["sine", "triangle", "saw", "pulse"] if mode == "full" else ["sine", "saw"]
    for lane in lanes:
        scenarios = [f"solo-{wave}-{lane}" for wave in waves]
        scenarios += [f"{kind}-{lane}x{lane}" for kind in ["xpm", "xfm", "xdepthpm"]]
        scenarios += [f"xnestedpm-{lane}"]
        for scenario in scenarios:
            for voices in [1, 8]:
                for frame in frames:
                    for rate in rates:
                        for factor in factors:
                            yield dict(scenario=scenario, voices=voices, frames=frame,
                                       sample_rate=rate, oversampling=factor)


def parse_output(output, expected):
    lines = [line for line in output.splitlines() if line.startswith("scenario=")]
    if len(lines) != 1:
        raise ValueError("expected exactly one process_lab result")
    pairs = [part.split("=", 1) for part in lines[0].split(",")]
    if any(len(pair) != 2 for pair in pairs):
        raise ValueError("malformed process_lab result")
    values = dict(pairs)
    if len(values) != len(pairs):
        raise ValueError("duplicate result keys")
    if values.get("finite") != "true":
        raise ValueError("nonfinite stream or missing whole-stream instrumentation")
    for key, value in expected.items():
        if key == "scenario":
            if values.get(key) != value:
                raise ValueError("wrong scenario")
        elif float(values.get(key, "nan")) != value:
            raise ValueError(f"wrong or missing {key}")
    required = ["median_ns_per_callback", "p50_ns", "p95_ns", "p99_ns", "p999_ns",
                "max_ns", "peak", "stream_energy", "stream_sum", "audible_callbacks",
                "deadline_misses", "callbacks", "repeats"]
    for key in required:
        number = float(values.get(key, "nan"))
        if not math.isfinite(number):
            raise ValueError(f"invalid or missing {key}")
        values[key] = number
    for key in ["median_ns_per_callback", "p50_ns", "p95_ns", "p99_ns", "p999_ns", "max_ns"]:
        if values[key] <= 0:
            raise ValueError(f"nonpositive timing {key}")
    total = values["callbacks"] * values["repeats"]
    if total <= 0 or any(values[key] != int(values[key]) or values[key] <= 0
                         for key in ["callbacks", "repeats"]):
        raise ValueError("invalid callback/repeat count")
    if values["peak"] <= 0 or values["stream_energy"] <= 0 or values["audible_callbacks"] <= 0:
        raise ValueError("silent workload")
    for key in ["deadline_misses", "audible_callbacks"]:
        if values[key] != int(values[key]) or not 0 <= values[key] <= total:
            raise ValueError(f"invalid {key}")
    if not values["p50_ns"] <= values["p95_ns"] <= values["p99_ns"] <= values["p999_ns"] <= values["max_ns"]:
        raise ValueError("unordered percentiles")
    return values


def command(binary, case, callbacks, repeats):
    return [str(binary), str(case["frames"]), str(callbacks), str(repeats),
            case["scenario"], str(case["voices"]), str(case["sample_rate"]),
            str(case["oversampling"])]


def run_once(binary, case, args):
    result = subprocess.run(command(binary, case, args.callbacks, args.repeats),
                            capture_output=True, text=True, timeout=args.timeout, check=True)
    return parse_output(result.stdout, dict(case, callbacks=args.callbacks, repeats=args.repeats))


def summarize(pairs):
    ratios = [pair["baseline"]["median_ns_per_callback"] /
              pair["candidate"]["median_ns_per_callback"] for pair in pairs]
    return dict(median_paired_speedup=statistics.median(ratios),
                min_paired_speedup=min(ratios), max_paired_speedup=max(ratios),
                median_p99_speedup=statistics.median(
                    pair["baseline"]["p99_ns"] / pair["candidate"]["p99_ns"] for pair in pairs),
                deadline_misses={side: sum(pair[side]["deadline_misses"] for pair in pairs)
                                 for side in ["baseline", "candidate"]})


def positive(value):
    value = int(value)
    if value <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--mode", choices=["quick", "full"], default="quick")
    parser.add_argument("--frames", type=positive, nargs="+", default=[64])
    parser.add_argument("--sample-rates", type=positive, nargs="+", default=[48000])
    parser.add_argument("--oversampling", type=int, choices=[1, 2, 3, 4], nargs="+", default=[2])
    parser.add_argument("--rounds", type=positive, default=3)
    parser.add_argument("--callbacks", type=positive, default=256)
    parser.add_argument("--repeats", type=positive, default=5)
    parser.add_argument("--timeout", type=positive, default=120)
    parser.add_argument("--cpu", default=platform.processor() or "unspecified")
    parser.add_argument("--baseline-build", default="unspecified: supply compiler, flags, commit")
    parser.add_argument("--candidate-build", default="unspecified: supply compiler, flags, commit")
    parser.add_argument("--notes", default="")
    parser.add_argument("--output", type=Path, default=Path("performance-matrix.json"))
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    cases = list(matrix(args.mode, args.frames, args.sample_rates, args.oversampling))
    if args.dry_run:
        print(json.dumps(dict(cases=cases, process_runs=len(cases)*args.rounds*2), indent=2))
        return 0
    binaries = {side: getattr(args, side).resolve(strict=True) for side in ["baseline", "candidate"]}
    report = dict(schema_version=1, started_utc=datetime.now(timezone.utc).isoformat(),
                  platform=platform.platform(), cpu=args.cpu,
                  affinity=sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
                  settings={key: str(value) if isinstance(value, Path) else value
                            for key, value in vars(args).items()},
                  binaries={side: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                                       build=getattr(args, side+"_build")) for side, path in binaries.items()},
                  caveat="Machine-local timings; no universal superiority or audio-equivalence claim.",
                  results=[])
    def save():
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(args.output.suffix+".tmp")
        temporary.write_text(json.dumps(report, indent=2, allow_nan=False)+"\n")
        temporary.replace(args.output)
    for index, case in enumerate(cases):
        entry = dict(case=case, pairs=[])
        report["results"].append(entry)
        try:
            for round_index in range(args.rounds):
                order = ["baseline", "candidate"] if (index+round_index) % 2 == 0 else ["candidate", "baseline"]
                pair = dict(order=order)
                for side in order:  # Never launch competing benchmarks concurrently.
                    pair[side] = run_once(binaries[side], case, args)
                entry["pairs"].append(pair)
            entry["summary"] = summarize(entry["pairs"])
        except (ValueError, subprocess.SubprocessError, OSError) as error:
            entry["error"] = str(error)
            save()
            print(f"Rejected {case}: {error}", file=sys.stderr)
            return 1
        save()
        print(f"{index+1}/{len(cases)} {case}: {entry['summary']['median_paired_speedup']:.3f}x", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
