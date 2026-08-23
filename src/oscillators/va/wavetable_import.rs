//! Surge `.wt` parsing and offline conversion into editable VA frames.
//!
//! This module is only called by the editor. Parsing, allocation, and spline
//! fitting must never be moved into an audio render path. Imported samples are
//! discarded after fitting: the DSP receives editable procedural `WaveCurveRt`
//! curves, never a sample wavetable or harmonic-partial table.

use std::fmt;

use crate::wave_curve::fit_periodic_samples;

use super::table::{MAX_VA_TABLE_FRAMES, VaTableData};

const HEADER_BYTES: usize = 12;
const MAX_SURGE_SAMPLES_PER_FRAME: usize = 4_096;
const MAX_SURGE_FRAMES: usize = 512;
pub const MAX_WAVETABLE_FILE_BYTES: usize =
    HEADER_BYTES + MAX_SURGE_SAMPLES_PER_FRAME * MAX_SURGE_FRAMES * size_of::<f32>() + 1024 * 1024;

const FLAG_IS_SAMPLE: u16 = 1;
const FLAG_LOOP_SAMPLE: u16 = 2;
const FLAG_INT16: u16 = 4;
const FLAG_INT16_FULL_RANGE: u16 = 8;
const FLAG_HAS_METADATA: u16 = 0x10;
const KNOWN_FLAGS: u16 =
    FLAG_IS_SAMPLE | FLAG_LOOP_SAMPLE | FLAG_INT16 | FLAG_INT16_FULL_RANGE | FLAG_HAS_METADATA;

/// Editor-ready result of importing one Surge wavetable.
pub struct ImportedVaTable {
    pub table: VaTableData,
    pub source_frame_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavetableImportError {
    FileTooLarge { bytes: usize },
    TruncatedHeader,
    InvalidMagic,
    InvalidSampleCount(u32),
    InvalidFrameCount(u16),
    UnsupportedFlags(u16),
    SampleFile,
    InvalidInt16Flags,
    TruncatedData { expected: usize, actual: usize },
    UnexpectedTrailingData,
    InvalidMetadata,
    NonFiniteSample { frame: usize, sample: usize },
}

impl fmt::Display for WavetableImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { bytes } => write!(
                formatter,
                "file is too large ({bytes} bytes; limit is {MAX_WAVETABLE_FILE_BYTES})"
            ),
            Self::TruncatedHeader => {
                formatter.write_str("file is shorter than the 12-byte Surge header")
            }
            Self::InvalidMagic => {
                formatter.write_str("not a Surge .wt file (missing vawt signature)")
            }
            Self::InvalidSampleCount(count) => write!(
                formatter,
                "invalid samples-per-frame count {count} (supported: powers of two in 2..={MAX_SURGE_SAMPLES_PER_FRAME})"
            ),
            Self::InvalidFrameCount(count) => write!(
                formatter,
                "invalid source frame count {count} (supported: 1..={MAX_SURGE_FRAMES})"
            ),
            Self::UnsupportedFlags(flags) => {
                write!(formatter, "unsupported Surge .wt flags 0x{flags:04x}")
            }
            Self::SampleFile => {
                formatter.write_str("Surge sample/one-shot .wt files are not periodic wavetables")
            }
            Self::InvalidInt16Flags => {
                formatter.write_str("16-bit full-range flag is set without the 16-bit data flag")
            }
            Self::TruncatedData { expected, actual } => write!(
                formatter,
                "truncated sample data (expected {expected} bytes, found {actual})"
            ),
            Self::UnexpectedTrailingData => {
                formatter.write_str("unexpected bytes after wavetable sample data")
            }
            Self::InvalidMetadata => formatter
                .write_str("metadata flag is set but metadata is missing or not NUL-terminated"),
            Self::NonFiniteSample { frame, sample } => write!(
                formatter,
                "frame {} contains a non-finite sample at index {}",
                frame + 1,
                sample
            ),
        }
    }
}

impl std::error::Error for WavetableImportError {}

struct SurgeHeader {
    samples_per_frame: usize,
    source_frame_count: usize,
    flags: u16,
    sample_bytes: usize,
    payload_end: usize,
}

fn parse_header(bytes: &[u8]) -> Result<SurgeHeader, WavetableImportError> {
    if bytes.len() > MAX_WAVETABLE_FILE_BYTES {
        return Err(WavetableImportError::FileTooLarge { bytes: bytes.len() });
    }
    if bytes.len() < HEADER_BYTES {
        return Err(WavetableImportError::TruncatedHeader);
    }
    if &bytes[..4] != b"vawt" {
        return Err(WavetableImportError::InvalidMagic);
    }

    let samples_per_frame = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or_default());
    let source_frame_count = u16::from_le_bytes(bytes[8..10].try_into().unwrap_or_default());
    let flags = u16::from_le_bytes(bytes[10..12].try_into().unwrap_or_default());
    if samples_per_frame < 2
        || samples_per_frame as usize > MAX_SURGE_SAMPLES_PER_FRAME
        || !samples_per_frame.is_power_of_two()
    {
        return Err(WavetableImportError::InvalidSampleCount(samples_per_frame));
    }
    if source_frame_count == 0 || source_frame_count as usize > MAX_SURGE_FRAMES {
        return Err(WavetableImportError::InvalidFrameCount(source_frame_count));
    }
    if flags & !KNOWN_FLAGS != 0 {
        return Err(WavetableImportError::UnsupportedFlags(flags & !KNOWN_FLAGS));
    }
    if flags & (FLAG_IS_SAMPLE | FLAG_LOOP_SAMPLE) != 0 {
        return Err(WavetableImportError::SampleFile);
    }
    if flags & FLAG_INT16_FULL_RANGE != 0 && flags & FLAG_INT16 == 0 {
        return Err(WavetableImportError::InvalidInt16Flags);
    }

    let sample_bytes = if flags & FLAG_INT16 != 0 { 2 } else { 4 };
    let payload_bytes = (samples_per_frame as usize)
        .checked_mul(source_frame_count as usize)
        .and_then(|samples| samples.checked_mul(sample_bytes))
        .ok_or(WavetableImportError::FileTooLarge { bytes: bytes.len() })?;
    let payload_end = HEADER_BYTES
        .checked_add(payload_bytes)
        .ok_or(WavetableImportError::FileTooLarge { bytes: bytes.len() })?;
    if bytes.len() < payload_end {
        return Err(WavetableImportError::TruncatedData {
            expected: payload_bytes,
            actual: bytes.len().saturating_sub(HEADER_BYTES),
        });
    }
    Ok(SurgeHeader {
        samples_per_frame: samples_per_frame as usize,
        source_frame_count: source_frame_count as usize,
        flags,
        sample_bytes,
        payload_end,
    })
}

/// Serum/Surge-compatible cycle length used when writing `.wt` files.
pub const EXPORT_SAMPLES_PER_FRAME: usize = 2_048;

/// Encode editable VA frames as a little-endian Surge `.wt` float32 table.
///
/// This is the same `vawt` interchange used by Surge and Vital. Importing the
/// result fits the samples back into procedural curves; it is not a sample
/// playback path.
pub fn encode_surge_wt(table: &VaTableData) -> Result<Vec<u8>, WavetableImportError> {
    if table.frames.is_empty() {
        return Err(WavetableImportError::InvalidFrameCount(0));
    }
    let frame_count = table.frames.len().min(MAX_VA_TABLE_FRAMES);
    let mut bytes = Vec::with_capacity(
        HEADER_BYTES + frame_count * EXPORT_SAMPLES_PER_FRAME * size_of::<f32>(),
    );
    bytes.extend_from_slice(b"vawt");
    bytes.extend_from_slice(&(EXPORT_SAMPLES_PER_FRAME as u32).to_le_bytes());
    bytes.extend_from_slice(&(frame_count as u16).to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    for frame in table.frames.iter().take(frame_count) {
        let curve = frame.compile_rt();
        for index in 0..EXPORT_SAMPLES_PER_FRAME {
            let sample = curve
                .eval(index as f32 / EXPORT_SAMPLES_PER_FRAME as f32)
                .clamp(-1.0, 1.0);
            if !sample.is_finite() {
                return Err(WavetableImportError::NonFiniteSample {
                    frame: bytes.len().saturating_sub(HEADER_BYTES)
                        / (EXPORT_SAMPLES_PER_FRAME * size_of::<f32>()),
                    sample: index,
                });
            }
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    Ok(bytes)
}

/// Parse a standard little-endian Surge wavetable and fit at most 16 editable
/// KURV frames. When reduction is necessary, endpoints and evenly-spaced
/// intermediate source frames are retained.
pub fn parse_surge_wt(bytes: &[u8]) -> Result<ImportedVaTable, WavetableImportError> {
    let header = parse_header(bytes)?;
    let trailing = &bytes[header.payload_end..];
    if header.flags & FLAG_HAS_METADATA != 0 {
        let Some(metadata) = trailing.strip_suffix(&[0]) else {
            return Err(WavetableImportError::InvalidMetadata);
        };
        let Ok(metadata) = std::str::from_utf8(metadata) else {
            return Err(WavetableImportError::InvalidMetadata);
        };
        let Ok(document) = roxmltree::Document::parse(metadata) else {
            return Err(WavetableImportError::InvalidMetadata);
        };
        if document.root_element().tag_name().name() != "wtmeta" {
            return Err(WavetableImportError::InvalidMetadata);
        }
    } else if !trailing.is_empty() {
        return Err(WavetableImportError::UnexpectedTrailingData);
    }

    if header.flags & FLAG_INT16 == 0 {
        let source_samples = header.samples_per_frame * header.source_frame_count;
        for flat_index in 0..source_samples {
            let offset = HEADER_BYTES + flat_index * header.sample_bytes;
            let sample = f32::from_le_bytes(
                bytes[offset..offset + header.sample_bytes]
                    .try_into()
                    .unwrap_or_default(),
            );
            if !sample.is_finite() {
                return Err(WavetableImportError::NonFiniteSample {
                    frame: flat_index / header.samples_per_frame,
                    sample: flat_index % header.samples_per_frame,
                });
            }
        }
    }

    let selected_count = header.source_frame_count.min(MAX_VA_TABLE_FRAMES);
    let mut frames = Vec::with_capacity(selected_count);
    for output_index in 0..selected_count {
        let source_index =
            selected_source_frame(output_index, selected_count, header.source_frame_count);
        let sample_offset =
            HEADER_BYTES + source_index * header.samples_per_frame * header.sample_bytes;
        let mut samples = Vec::with_capacity(header.samples_per_frame);
        for sample_index in 0..header.samples_per_frame {
            let offset = sample_offset + sample_index * header.sample_bytes;
            let sample = decode_sample(bytes, offset, header.flags);
            samples.push(sample);
        }
        frames.push(fit_periodic_samples(&samples));
    }

    let frame_count = frames.len();
    let positions = (0..frame_count)
        .map(|index| (index as f32 + 0.5) / frame_count as f32)
        .collect();
    Ok(ImportedVaTable {
        table: VaTableData { frames, positions },
        source_frame_count: header.source_frame_count,
    })
}

fn decode_sample(bytes: &[u8], offset: usize, flags: u16) -> f32 {
    if flags & FLAG_INT16 != 0 {
        let raw = i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap_or_default());
        let scale = if flags & FLAG_INT16_FULL_RANGE != 0 {
            1.0 / 32_768.0
        } else {
            1.0 / 16_384.0
        };
        f32::from(raw) * scale
    } else {
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default())
    }
}

const fn selected_source_frame(output: usize, selected: usize, source: usize) -> usize {
    if selected <= 1 || source <= 1 {
        0
    } else {
        // Integer rounding avoids accumulated floating-point error and always
        // preserves the first and last source frame.
        (output * (source - 1) + (selected - 1) / 2) / (selected - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float_fixture(frames: &[Vec<f32>], flags: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"vawt");
        bytes.extend_from_slice(&(frames[0].len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(frames.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&flags.to_le_bytes());
        for frame in frames {
            for &sample in frame {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        bytes
    }

    fn int16_fixture(samples: &[i16], flags: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"vawt");
        bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&flags.to_le_bytes());
        for &sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn encode_round_trips_through_parse() {
        let sine = (0..EXPORT_SAMPLES_PER_FRAME)
            .map(|index| {
                (std::f32::consts::TAU * index as f32 / EXPORT_SAMPLES_PER_FRAME as f32).sin()
            })
            .collect::<Vec<_>>();
        let imported = parse_surge_wt(&float_fixture(&[sine], 0)).expect("fixture should parse");
        let encoded = encode_surge_wt(&imported.table).expect("export should encode");
        let again = parse_surge_wt(&encoded).expect("exported table should parse");
        assert_eq!(again.table.frames.len(), 1);
        assert_eq!(&encoded[..4], b"vawt");
        assert_eq!(
            u32::from_le_bytes(encoded[4..8].try_into().expect("sample count")),
            EXPORT_SAMPLES_PER_FRAME as u32
        );
    }

    #[test]
    fn parses_and_fits_generated_float_frames() {
        let sine = (0..128)
            .map(|index| (std::f32::consts::TAU * index as f32 / 128.0).sin())
            .collect::<Vec<_>>();
        let triangle = (0..128)
            .map(|index| 1.0 - 4.0 * ((index as f32 / 128.0) - 0.5).abs())
            .collect::<Vec<_>>();
        let parsed = parse_surge_wt(&float_fixture(&[sine.clone(), triangle], 0))
            .expect("generated wavetable should parse");

        assert_eq!(parsed.source_frame_count, 2);
        assert_eq!(parsed.table.frames.len(), 2);
        assert_eq!(parsed.table.positions, vec![0.25, 0.75]);
        let fitted = parsed.table.frames[0].compile_rt();
        let rms = (0..128)
            .map(|index| {
                let error = fitted.eval(index as f32 / 128.0) - sine[index];
                error * error
            })
            .sum::<f32>()
            / 128.0;
        assert!(rms.sqrt() < 0.035, "sine fitting RMS was {}", rms.sqrt());
    }

    #[test]
    fn imports_minimum_size_constant_cycle() {
        let parsed = parse_surge_wt(&float_fixture(&[vec![0.25; 2]], 0))
            .expect("two-sample constant cycle should parse");
        let curve = parsed.table.frames[0].compile_rt();

        assert!((curve.eval(0.73) - 0.25).abs() < 0.02);
    }

    #[test]
    fn decodes_both_surge_int16_ranges() {
        let int15 = parse_surge_wt(&int16_fixture(&[0, 16_384, 0, -16_384], FLAG_INT16))
            .expect("15-bit fixture should parse");
        let full = parse_surge_wt(&int16_fixture(
            &[0, 16_384, 0, -16_384],
            FLAG_INT16 | FLAG_INT16_FULL_RANGE,
        ))
        .expect("16-bit fixture should parse");

        let int15_peak = int15.table.frames[0].compile_rt().eval(0.25);
        let full_peak = full.table.frames[0].compile_rt().eval(0.25);
        assert!((int15_peak - 1.0).abs() < 0.08);
        assert!((full_peak - 0.5).abs() < 0.05);
    }

    #[test]
    fn reduces_large_tables_evenly_and_preserves_endpoints() {
        let frames = (0..31)
            .map(|index| vec![index as f32 / 30.0; 8])
            .collect::<Vec<_>>();
        let parsed = parse_surge_wt(&float_fixture(&frames, 0))
            .expect("generated many-frame table should parse");

        assert_eq!(parsed.source_frame_count, 31);
        assert_eq!(parsed.table.frames.len(), MAX_VA_TABLE_FRAMES);
        assert_eq!(parsed.table.positions[0], 0.5 / MAX_VA_TABLE_FRAMES as f32);
        assert_eq!(
            parsed.table.positions[MAX_VA_TABLE_FRAMES - 1],
            (MAX_VA_TABLE_FRAMES as f32 - 0.5) / MAX_VA_TABLE_FRAMES as f32
        );
        let first = parsed.table.frames[0].compile_rt().eval(0.37);
        let last = parsed.table.frames[MAX_VA_TABLE_FRAMES - 1]
            .compile_rt()
            .eval(0.37);
        assert!(first.abs() < 0.02);
        assert!((last - 1.0).abs() < 0.02);
    }

    #[test]
    fn rejects_malformed_and_sample_files() {
        assert_eq!(
            parse_surge_wt(b"vawt").err(),
            Some(WavetableImportError::TruncatedHeader)
        );
        let mut truncated = float_fixture(&[vec![0.0; 8]], 0);
        truncated.pop();
        assert!(matches!(
            parse_surge_wt(&truncated),
            Err(WavetableImportError::TruncatedData { .. })
        ));
        assert_eq!(
            parse_surge_wt(&float_fixture(&[vec![0.0; 8]], FLAG_IS_SAMPLE)).err(),
            Some(WavetableImportError::SampleFile)
        );
    }

    #[test]
    fn rejects_invalid_magic_counts_flags_and_oversized_files() {
        let mut invalid_magic = float_fixture(&[vec![0.0; 8]], 0);
        invalid_magic[..4].copy_from_slice(b"RIFF");
        assert_eq!(
            parse_surge_wt(&invalid_magic).err(),
            Some(WavetableImportError::InvalidMagic)
        );

        let mut too_many_samples = float_fixture(&[vec![0.0; 8]], 0);
        too_many_samples[4..8]
            .copy_from_slice(&((MAX_SURGE_SAMPLES_PER_FRAME + 1) as u32).to_le_bytes());
        assert!(matches!(
            parse_surge_wt(&too_many_samples),
            Err(WavetableImportError::InvalidSampleCount(_))
        ));
        let non_power_of_two = float_fixture(&[vec![0.0; 3]], 0);
        assert_eq!(
            parse_surge_wt(&non_power_of_two).err(),
            Some(WavetableImportError::InvalidSampleCount(3))
        );

        let unknown_flags = float_fixture(&[vec![0.0; 8]], 0x8000);
        assert!(matches!(
            parse_surge_wt(&unknown_flags),
            Err(WavetableImportError::UnsupportedFlags(0x8000))
        ));

        let oversized = vec![0; MAX_WAVETABLE_FILE_BYTES + 1];
        assert!(matches!(
            parse_surge_wt(&oversized),
            Err(WavetableImportError::FileTooLarge { .. })
        ));
    }

    #[test]
    fn validates_non_finite_samples_and_metadata() {
        let nan = float_fixture(&[vec![0.0, f32::NAN]], 0);
        assert!(matches!(
            parse_surge_wt(&nan),
            Err(WavetableImportError::NonFiniteSample { .. })
        ));
        let missing_metadata = float_fixture(&[vec![0.0; 8]], FLAG_HAS_METADATA);
        assert_eq!(
            parse_surge_wt(&missing_metadata).err(),
            Some(WavetableImportError::InvalidMetadata)
        );
        let mut metadata = missing_metadata.clone();
        metadata.extend_from_slice(b"<wtmeta/>\0");
        assert!(parse_surge_wt(&metadata).is_ok());

        let mut wrong_root = missing_metadata.clone();
        wrong_root.extend_from_slice(b"<other/>\0");
        assert_eq!(
            parse_surge_wt(&wrong_root).err(),
            Some(WavetableImportError::InvalidMetadata)
        );

        let mut trailing_garbage = missing_metadata;
        trailing_garbage.extend_from_slice(b"<wtmeta/>\0garbage\0");
        assert_eq!(
            parse_surge_wt(&trailing_garbage).err(),
            Some(WavetableImportError::InvalidMetadata)
        );
    }
}
