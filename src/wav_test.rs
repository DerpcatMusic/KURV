//! Shared WAV fixture builders for tests.

use std::io::Cursor;

pub(crate) fn wav_i16(
    channels: u16,
    sample_rate: u32,
    samples: impl IntoIterator<Item = i16>,
) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("writer");
        for sample in samples {
            writer.write_sample(sample).expect("sample");
        }
        writer.finalize().expect("finalize");
    }
    cursor.into_inner()
}

pub(crate) fn wav_f32(
    channels: u16,
    sample_rate: u32,
    samples: impl IntoIterator<Item = f32>,
) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("writer");
        for sample in samples {
            writer.write_sample(sample).expect("sample");
        }
        writer.finalize().expect("finalize");
    }
    cursor.into_inner()
}

pub(crate) fn wav_sine(frequency: f32, seconds: f32) -> Vec<u8> {
    let frames = (48_000.0 * seconds) as usize;
    wav_i16(
        1,
        48_000,
        (0..frames).map(|index| {
            let phase = std::f32::consts::TAU * frequency * index as f32 / 48_000.0;
            (phase.sin() * 20_000.0) as i16
        }),
    )
}
