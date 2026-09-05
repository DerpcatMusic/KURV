#!/usr/bin/env python3
"""Standard-library-only coefficient/stream-model audit; not a plugin benchmark.

Loads f32-rounded production coefficients. The streaming model uses Python f64
accumulation (not SIMD/f32/FMA) and mirrors push-all-internal-samples then output.
Run from any directory; --check asserts stable diagnostic invariants.
"""
import argparse
import ast
import cmath
import json
import math
from pathlib import Path
import re
import struct

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / "src/oversampling.rs").read_text()


def f32(value):
    return struct.unpack("f", struct.pack("f", value))[0]


def scalar(name):
    text = re.search(rf"const {name}: f32 = (.*?);", SOURCE).group(1)
    for dependency in re.findall(r"\b[A-Z][A-Z_]+\b", text):
        text = text.replace(dependency, repr(scalar(dependency)))
    # Only arithmetic literals from this repository; no eval or shell execution.
    def resolve(node):
        if isinstance(node, ast.Constant):
            return node.value
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            return -resolve(node.operand)
        if isinstance(node, ast.BinOp):
            a, b = resolve(node.left), resolve(node.right)
            if isinstance(node.op, ast.Sub):
                return a - b
            if isinstance(node.op, ast.Mult):
                return a * b
        raise ValueError(ast.dump(node))
    return f32(resolve(ast.parse(text.replace("_", ""), mode="eval").body))


def coefficients(factor):
    text = re.search(rf"const EQUIRIPPLE_{factor}X_HALF:.*?= \[(.*?)\];", SOURCE, re.S).group(1)
    half = [f32(float(value.replace("_", ""))) for value in text.split(",") if value.strip()]
    return half + half[-2::-1]


def response(taps, frequency, sample_rate):
    return sum(c * cmath.exp(-2j * math.pi * frequency * i / sample_rate) for i, c in enumerate(taps))


def db(value):
    return 20 * math.log10(max(abs(value), 1e-150))


def eq_taps(spline):
    if spline:
        outer, side, center = [scalar("SPLINE_EQ_" + key) for key in ("OUTER", "SIDE", "CENTER")]
    else:
        outer, side, center = 0, scalar("PASSBAND_EQ_SIDE"), scalar("PASSBAND_EQ_CENTER")
    return [outer, side, center, side, outer]


def stream_model(factor, spline, frames=1024):
    taps, eq = coefficients(factor), eq_taps(spline)
    frequency = 1 / 64
    history, eq_history, outputs = [], [], []
    for frame in range(frames):
        for sub in range(factor):
            history.append(math.sin(2 * math.pi * frequency * (frame + sub / factor)))
        value = sum(c * history[-1 - i] for i, c in enumerate(taps) if i < len(history))
        eq_history.append(value)
        index = frame - 7  # production post-EQ delay
        outputs.append(sum(c * eq_history[index - i] for i, c in enumerate(eq) if 0 <= index - i < len(eq_history)))
    start = 256
    output_bin = sum(outputs[i] * cmath.exp(-2j * math.pi * frequency * i) for i in range(start, frames))
    input_bin = sum(math.sin(2 * math.pi * frequency * i) * cmath.exp(-2j * math.pi * frequency * i) for i in range(start, frames))
    lag = (-cmath.phase(output_bin / input_bin) / (2 * math.pi * frequency)) % 64
    return lag


def run(check):
    rows = []
    for host in (44100, 48000, 96000):
        for factor in (2, 3, 4):
            fir = response(coefficients(factor), 20000, host * factor)
            for spline in ((False, True) if factor == 2 else (False,)):
                eq = response(eq_taps(spline), 20000, host)
                rows.append(dict(host_hz=host, factor=factor, spline=spline, fir_db=round(db(fir), 6), full_filter_db=round(db(fir * eq), 6)))
    delays = [dict(factor=factor, spline=spline, measured_host_samples=round(stream_model(factor, spline), 9), nominal_host_samples=33) for factor in (2, 3, 4) for spline in ((False, True) if factor == 2 else (False,))]
    if check:
        for row in delays:
            expected = 33 - (row["factor"] - 1) / row["factor"]
            assert abs(row["measured_host_samples"] - expected) < 1e-7, row
        for row in rows:
            if row["host_hz"] == 44100:
                assert row["fir_db"] < -5.4, row
            if row["host_hz"] == 48000:
                assert abs(row["fir_db"]) < 0.052, row
    print(json.dumps(dict(model="f32 production coefficients, f64 response/stream accumulation; no oscillator or plugin execution", response_at_20khz=rows, stream_delays=delays), indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    run(parser.parse_args().check)
