#!/usr/bin/env python3

import math
from pathlib import Path

import numpy as np


TABLE_SIZE = 4096
SAW_HARMONICS = list(range(129))
TRIANGLE_HARMONICS = list(range(129))


def write_bank(output: Path, waveform: str, stored_harmonics: list[int]) -> None:
    phase = np.arange(TABLE_SIZE, dtype=np.float64) / TABLE_SIZE
    samples = np.zeros(TABLE_SIZE, dtype=np.float64)
    wanted = set(stored_harmonics)
    with output.open("wb") as file:
        file.write(np.zeros(TABLE_SIZE, dtype="<f4").tobytes())
        for harmonic in range(1, stored_harmonics[-1] + 1):
            if waveform == "saw":
                samples += -2.0 * np.sin(math.tau * harmonic * phase) / (math.pi * harmonic)
            elif harmonic % 2 == 1:
                samples += (
                    -8.0
                    * np.cos(math.tau * harmonic * phase)
                    / (math.pi * math.pi * harmonic * harmonic)
                )
            if harmonic in wanted:
                file.write(samples.astype("<f4").tobytes())


def main() -> None:
    source = Path(__file__).resolve().parents[1] / "src"
    write_bank(source / "spectral-saw-f32le.bin", "saw", SAW_HARMONICS)
    write_bank(
        source / "spectral-triangle-f32le.bin", "triangle", TRIANGLE_HARMONICS
    )


if __name__ == "__main__":
    main()
