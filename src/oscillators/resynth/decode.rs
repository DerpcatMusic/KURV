//! Bounded Source Master decode for WAV plus common audio containers.

use std::io::Cursor;

use super::{
    ImportError, MAX_RESYNTH_DECODED_FRAMES, MAX_RESYNTH_SOURCE_BYTES,
    artifact::MAX_SOURCE_ABS_SAMPLE,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodedSourceSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Exact mono/stereo playback projection plus the strongest channel used only
/// for pitch and visual analysis. Stereo remains losslessly recoverable as
/// `left = mid + side`, `right = mid - side`.
pub(crate) struct DecodedSourcePcm {
    mid: Vec<f32>,
    side: Option<Vec<f32>>,
    analysis_is_side: bool,
}

impl DecodedSourcePcm {
    fn new(mid: Vec<f32>, side: Option<Vec<f32>>) -> Self {
        let analysis_is_side = side.as_ref().is_some_and(|side| {
            let power = |samples: &[f32]| {
                samples
                    .iter()
                    .map(|sample| f64::from(*sample) * f64::from(*sample))
                    .sum::<f64>()
            };
            power(side) > power(&mid) * 1.01
        });
        Self {
            mid,
            side,
            analysis_is_side,
        }
    }

    pub(crate) fn analysis(&self) -> &[f32] {
        if self.analysis_is_side {
            self.side.as_deref().unwrap_or(&self.mid)
        } else {
            &self.mid
        }
    }

    pub(crate) fn analysis_mut(&mut self) -> &mut [f32] {
        if self.analysis_is_side {
            self.side.as_deref_mut().unwrap_or(&mut self.mid)
        } else {
            &mut self.mid
        }
    }

    pub(crate) fn mid(&self) -> &[f32] {
        &self.mid
    }

    pub(crate) fn side(&self) -> Option<&[f32]> {
        self.side.as_deref()
    }
}

pub(crate) const AUDIO_IMPORT_EXTENSIONS: &[&str] =
    &["wav", "wave", "flac", "aif", "aiff", "ogg", "oga", "mp3"];

#[must_use]
pub(crate) fn is_supported_audio_import_name(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return looks_like_supported_audio(name.as_bytes());
    };
    is_supported_audio_import_extension(extension)
}

#[must_use]
pub(crate) fn is_supported_audio_import_extension(extension: &str) -> bool {
    AUDIO_IMPORT_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

#[must_use]
pub(crate) fn looks_like_supported_audio(bytes: &[u8]) -> bool {
    is_wave_container(bytes)
        || bytes.starts_with(b"fLaC")
        || is_aiff_container(bytes)
        || bytes.starts_with(b"OggS")
        || is_mpeg_container(bytes)
}

fn is_wave_container(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE"
}

fn is_aiff_container(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && bytes.starts_with(b"FORM")
        && (&bytes[8..12] == b"AIFF" || &bytes[8..12] == b"AIFC")
}

fn is_mpeg_container(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3")
        || matches!(
            bytes,
            [0xff, second, ..] if second & 0xe0 == 0xe0
        )
}

pub(crate) fn decode_source_with_cancel(
    bytes: &[u8],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(DecodedSourcePcm, DecodedSourceSpec, usize), ImportError> {
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    if bytes.is_empty() {
        return Err(ImportError::Empty);
    }
    if bytes.len() > MAX_RESYNTH_SOURCE_BYTES {
        return Err(ImportError::Oversize {
            bytes: bytes.len(),
            limit: MAX_RESYNTH_SOURCE_BYTES,
        });
    }
    if is_wave_container(bytes) {
        return decode_wav_with_cancel(bytes, should_cancel);
    }
    decode_compressed_with_cancel(bytes, should_cancel)
}

fn decode_wav_with_cancel(
    bytes: &[u8],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(DecodedSourcePcm, DecodedSourceSpec, usize), ImportError> {
    let mut reader =
        hound::WavReader::new(Cursor::new(bytes)).map_err(|_| ImportError::UnsupportedWav)?;
    let spec = reader.spec();
    let source = DecodedSourceSpec {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    };
    validate_spec(source)?;
    let frames = usize::try_from(reader.duration()).map_err(|_| ImportError::TooManyFrames)?;
    if frames == 0 || frames > MAX_RESYNTH_DECODED_FRAMES {
        return Err(ImportError::TooManyFrames);
    }
    let mut mid = Vec::with_capacity(frames);
    let mut side = (spec.channels == 2).then(|| Vec::with_capacity(frames));
    let mut frame = [0.0_f32; 2];
    let mut channel = 0_usize;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (sample_index, value) in reader.samples::<f32>().enumerate() {
                if sample_index & 4_095 == 0 && should_cancel() {
                    return Err(ImportError::Cancelled);
                }
                let value = value.map_err(|_| ImportError::UnsupportedWav)?;
                if !value.is_finite() || value.abs() > MAX_SOURCE_ABS_SAMPLE {
                    return Err(ImportError::UnsupportedWav);
                }
                frame[channel] = value;
                channel += 1;
                if channel == usize::from(spec.channels) {
                    push_frame(&mut mid, side.as_mut(), &frame, spec.channels);
                    channel = 0;
                }
            }
        }
        hound::SampleFormat::Int => {
            let scale = 2.0_f32.powi(1 - i32::from(spec.bits_per_sample));
            for (sample_index, value) in reader.samples::<i32>().enumerate() {
                if sample_index & 4_095 == 0 && should_cancel() {
                    return Err(ImportError::Cancelled);
                }
                frame[channel] = value.map_err(|_| ImportError::UnsupportedWav)? as f32 * scale;
                channel += 1;
                if channel == usize::from(spec.channels) {
                    push_frame(&mut mid, side.as_mut(), &frame, spec.channels);
                    channel = 0;
                }
            }
        }
    }
    if channel != 0 || mid.len() != frames {
        return Err(ImportError::UnsupportedWav);
    }
    Ok((DecodedSourcePcm::new(mid, side), source, frames))
}

fn decode_compressed_with_cancel(
    bytes: &[u8],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(DecodedSourcePcm, DecodedSourceSpec, usize), ImportError> {
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    if !looks_like_supported_audio(bytes) {
        return Err(ImportError::UnsupportedWav);
    }

    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| ImportError::UnsupportedWav)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(ImportError::UnsupportedWav)?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or(ImportError::UnsupportedWav)?;
    let channels = track
        .codec_params
        .channels
        .map_or(0, |layout| layout.count())
        .try_into()
        .unwrap_or(0);
    let spec = DecodedSourceSpec {
        sample_rate,
        channels,
    };
    validate_spec(spec)?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| ImportError::UnsupportedWav)?;

    let mut mid = Vec::new();
    let mut side = (spec.channels == 2).then(Vec::new);
    loop {
        if should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => return Err(ImportError::UnsupportedWav),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => return Err(ImportError::UnsupportedWav),
        };
        append_decoded_buffer(decoded, spec.channels, &mut mid, side.as_mut())?;
        if mid.len() > MAX_RESYNTH_DECODED_FRAMES {
            return Err(ImportError::TooManyFrames);
        }
    }
    if mid.is_empty() {
        return Err(ImportError::Empty);
    }
    let frames = mid.len();
    Ok((DecodedSourcePcm::new(mid, side), spec, frames))
}

fn append_decoded_buffer(
    decoded: symphonia::core::audio::AudioBufferRef<'_>,
    channels: u16,
    mid: &mut Vec<f32>,
    side: Option<&mut Vec<f32>>,
) -> Result<(), ImportError> {
    use symphonia::core::audio::AudioBufferRef;

    match decoded {
        AudioBufferRef::F32(buffer) => append_planar(buffer.planes().planes(), channels, mid, side),
        AudioBufferRef::F64(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                *sample as f32
            })
        }
        AudioBufferRef::S32(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                *sample as f32 / 2_147_483_648.0
            })
        }
        AudioBufferRef::S24(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                sample.inner() as f32 / 8_388_608.0
            })
        }
        AudioBufferRef::S16(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                f32::from(*sample) / 32_768.0
            })
        }
        AudioBufferRef::S8(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                f32::from(*sample) / 128.0
            })
        }
        AudioBufferRef::U8(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                (f32::from(*sample) - 128.0) / 128.0
            })
        }
        AudioBufferRef::U16(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                (f32::from(*sample) - 32_768.0) / 32_768.0
            })
        }
        AudioBufferRef::U24(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                (sample.inner() as f32 - 8_388_608.0) / 8_388_608.0
            })
        }
        AudioBufferRef::U32(buffer) => {
            append_converted(buffer.planes().planes(), channels, mid, side, |sample| {
                (*sample as f32 - 2_147_483_648.0) / 2_147_483_648.0
            })
        }
    }
}

fn append_planar(
    planes: &[&[f32]],
    channels: u16,
    mid: &mut Vec<f32>,
    side: Option<&mut Vec<f32>>,
) -> Result<(), ImportError> {
    append_converted(planes, channels, mid, side, |sample| *sample)
}

fn append_converted<T>(
    planes: &[&[T]],
    channels: u16,
    mid: &mut Vec<f32>,
    mut side: Option<&mut Vec<f32>>,
    convert: impl Fn(&T) -> f32,
) -> Result<(), ImportError> {
    let channel_count = usize::from(channels);
    if planes.len() < channel_count {
        return Err(ImportError::UnsupportedWav);
    }
    let frames = planes[0].len();
    if planes
        .iter()
        .take(channel_count)
        .any(|plane| plane.len() != frames)
    {
        return Err(ImportError::UnsupportedWav);
    }
    mid.reserve(frames);
    if let Some(side) = side.as_mut() {
        side.reserve(frames);
    }
    for index in 0..frames {
        let left = convert(&planes[0][index]);
        if !left.is_finite() || left.abs() > MAX_SOURCE_ABS_SAMPLE {
            return Err(ImportError::UnsupportedWav);
        }
        if channel_count == 1 {
            mid.push(left);
            continue;
        }
        let right = convert(&planes[1][index]);
        if !right.is_finite() || right.abs() > MAX_SOURCE_ABS_SAMPLE {
            return Err(ImportError::UnsupportedWav);
        }
        mid.push((left + right) * 0.5);
        if let Some(side) = side.as_mut() {
            side.push((left - right) * 0.5);
        }
    }
    Ok(())
}

fn push_frame(mid: &mut Vec<f32>, side: Option<&mut Vec<f32>>, frame: &[f32; 2], channels: u16) {
    if channels == 1 {
        mid.push(frame[0]);
        return;
    }
    mid.push((frame[0] + frame[1]) * 0.5);
    if let Some(side) = side {
        side.push((frame[0] - frame[1]) * 0.5);
    }
}

fn validate_spec(spec: DecodedSourceSpec) -> Result<(), ImportError> {
    if !(1..=2).contains(&spec.channels) {
        return Err(ImportError::UnsupportedChannels(spec.channels));
    }
    if !(8_000..=384_000).contains(&spec.sample_rate) {
        return Err(ImportError::UnsupportedSampleRate(spec.sample_rate));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_supported_audio_import_name, looks_like_supported_audio};

    #[test]
    fn import_names_accept_common_audio_extensions() {
        assert!(is_supported_audio_import_name("pad.FLAC"));
        assert!(is_supported_audio_import_name("loop.wav"));
        assert!(is_supported_audio_import_name("choir.aiff"));
        assert!(is_supported_audio_import_name("noise.ogg"));
        assert!(is_supported_audio_import_name("hit.mp3"));
        assert!(!is_supported_audio_import_name("table.wt"));
        assert!(!is_supported_audio_import_name("readme.txt"));
    }

    #[test]
    fn magic_bytes_identify_wave_and_flac() {
        let mut wave = b"RIFFxxxxWAVE".to_vec();
        wave.extend_from_slice(&[0; 8]);
        assert!(looks_like_supported_audio(&wave));
        assert!(looks_like_supported_audio(b"fLaC...."));
        assert!(looks_like_supported_audio(b"OggS...."));
        assert!(!looks_like_supported_audio(b"vawt...."));
    }
}
