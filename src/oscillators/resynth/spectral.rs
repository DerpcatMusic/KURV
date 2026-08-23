//! Shared spectral voice renderer for free shift and absolute targeting.
//!
//! Analysis prepares target pitches off-thread. The callback only advances a
//! bounded set of harmonic partials and mixes the prepared residual/onset
//! path. Target modes therefore share one engine instead of duplicating DSP.

use std::f32::consts::TAU;

use super::analysis::{MAX_PITCH_FAMILIES, TargetedPitchFrame};

pub const MAX_SPECTRAL_PARTIALS: usize = MAX_PITCH_FAMILIES;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpectralPartial {
    pub source_midi: f32,
    pub target_midi: f32,
    pub frequency_hz: f32,
    pub amplitude: f32,
    pub confidence: f32,
    pub phase: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpectralFrame {
    pub frame_index: u32,
    pub onset: f32,
    pub residual_gain: f32,
    /// Sum normalization keeps aligned partials bounded before residual mix.
    pub harmonic_gain: f32,
    pub partials: [SpectralPartial; MAX_SPECTRAL_PARTIALS],
    pub partial_count: u8,
}

impl SpectralFrame {
    #[must_use]
    pub fn from_targeted(frame: &TargetedPitchFrame, sample_rate: f32, residual_gain: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.max(1.0)
        } else {
            1.0
        };
        let mut output = Self {
            frame_index: frame.frame_index,
            onset: frame.onset.clamp(0.0, 1.0),
            residual_gain: residual_gain.clamp(0.0, 1.0),
            harmonic_gain: 1.0,
            ..Self::default()
        };
        let mut weight_sum = 0.0_f32;
        for pitch in frame
            .families
            .iter()
            .copied()
            .take(usize::from(frame.family_count))
        {
            let index = usize::from(output.partial_count);
            if index >= MAX_SPECTRAL_PARTIALS {
                break;
            }
            let frequency_hz = midi_to_hz(pitch.target_midi).min(sample_rate * 0.49);
            output.partials[index] = SpectralPartial {
                source_midi: pitch.source_midi,
                target_midi: pitch.target_midi,
                frequency_hz,
                amplitude: pitch.strength.max(0.0),
                confidence: pitch.confidence.clamp(0.0, 1.0),
                phase: 0.0,
            };
            weight_sum += output.partials[index].amplitude * output.partials[index].confidence;
            output.partial_count += 1;
        }
        output.harmonic_gain = weight_sum.max(1.0).recip();
        output
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SpectralRenderer {
    frame: SpectralFrame,
    sample_rate: f32,
}

impl SpectralRenderer {
    #[must_use]
    pub const fn new(sample_rate: f32) -> Self {
        Self {
            frame: SpectralFrame {
                frame_index: 0,
                onset: 0.0,
                residual_gain: 0.0,
                harmonic_gain: 1.0,
                partials: [SpectralPartial {
                    source_midi: 0.0,
                    target_midi: 0.0,
                    frequency_hz: 0.0,
                    amplitude: 0.0,
                    confidence: 0.0,
                    phase: 0.0,
                }; MAX_SPECTRAL_PARTIALS],
                partial_count: 0,
            },
            sample_rate,
        }
    }

    pub fn reset(&mut self) {
        self.frame = SpectralFrame::default();
    }

    pub fn set_frame(&mut self, mut frame: SpectralFrame) {
        let count = usize::from(frame.partial_count).min(MAX_SPECTRAL_PARTIALS);
        // Preserve phase by source-family identity when a worker frame arrives.
        for next in frame.partials.iter_mut().take(count) {
            if let Some(previous) = self
                .frame
                .partials
                .iter()
                .take(usize::from(self.frame.partial_count))
                .find(|previous| (previous.source_midi - next.source_midi).abs() < 0.01)
            {
                next.phase = previous.phase;
            }
        }
        frame.partial_count = count as u8;
        self.frame = frame;
    }

    #[must_use]
    pub const fn frame(&self) -> SpectralFrame {
        self.frame
    }

    /// Render one sample of harmonics plus prepared residual/onset material.
    /// Residual and onset samples are deliberately not pitch-retuned.
    pub fn render_sample(&mut self, residual: f32, onset: f32) -> f32 {
        let mut harmonic = 0.0_f32;
        let sample_rate = self.sample_rate.max(1.0);
        for partial in self
            .frame
            .partials
            .iter_mut()
            .take(usize::from(self.frame.partial_count))
        {
            harmonic += partial.phase.sin() * partial.amplitude * partial.confidence;
            partial.phase =
                (partial.phase + TAU * partial.frequency_hz / sample_rate).rem_euclid(TAU);
        }
        let onset_mix = self.frame.onset;
        (harmonic.mul_add(
            self.frame.harmonic_gain,
            residual * self.frame.residual_gain,
        ) + onset * onset_mix)
            .clamp(-1.0, 1.0)
    }
}

#[must_use]
pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscillators::resynth::analysis::{PitchCandidate, PitchFrame};
    use crate::oscillators::resynth::targeting::{PitchMode, ScaleId, TargetSet};

    #[test]
    fn spectral_frame_converts_target_families_to_bounded_partials() {
        let frame =
            PitchFrame::from_candidates(4, 0.8, 0.2, &[PitchCandidate::new(60.0, 1.0, 0.9)])
                .targeted(PitchMode::Target(TargetSet::PlayedNote), 64.0, 60.0);
        let spectral = SpectralFrame::from_targeted(&frame, 48_000.0, 0.35);
        assert_eq!(spectral.partial_count, 1);
        assert_eq!(spectral.partials[0].target_midi, 64.0);
        assert!((spectral.partials[0].frequency_hz - 329.62756).abs() < 0.01);
        assert_eq!(spectral.residual_gain, 0.35);
    }

    #[test]
    fn renderer_preserves_residual_and_onset_paths() {
        let frame = SpectralFrame {
            onset: 1.0,
            residual_gain: 0.5,
            ..SpectralFrame::default()
        };
        let mut renderer = SpectralRenderer::new(48_000.0);
        renderer.set_frame(frame);
        assert_eq!(renderer.render_sample(0.4, 0.25), 0.45);
    }

    #[test]
    fn renderer_advances_partial_phase_without_allocating() {
        let frame =
            PitchFrame::from_candidates(0, 1.0, 0.0, &[PitchCandidate::new(69.0, 1.0, 1.0)])
                .targeted(PitchMode::Spectral, 69.0, 69.0);
        let spectral = SpectralFrame::from_targeted(&frame, 48_000.0, 0.0);
        let mut renderer = SpectralRenderer::new(48_000.0);
        renderer.set_frame(spectral);
        let first = renderer.render_sample(0.0, 0.0);
        let second = renderer.render_sample(0.0, 0.0);
        assert_eq!(first, 0.0);
        assert!(second > 0.0);
    }

    #[test]
    fn aligned_partial_bank_is_gain_bounded() {
        let mut frame = SpectralFrame {
            partial_count: MAX_SPECTRAL_PARTIALS as u8,
            harmonic_gain: 1.0 / MAX_SPECTRAL_PARTIALS as f32,
            ..SpectralFrame::default()
        };
        for partial in frame.partials.iter_mut() {
            partial.phase = std::f32::consts::FRAC_PI_2;
            partial.amplitude = 1.0;
            partial.confidence = 1.0;
        }
        let mut renderer = SpectralRenderer::new(48_000.0);
        renderer.set_frame(frame);
        let output = renderer.render_sample(0.0, 0.0);
        assert!(output.is_finite());
        assert!(output.abs() <= 1.0);
    }

    #[test]
    fn target_scale_reaches_the_shared_engine() {
        let frame =
            PitchFrame::from_candidates(0, 1.0, 0.0, &[PitchCandidate::new(63.0, 1.0, 1.0)])
                .targeted(
                    PitchMode::Target(TargetSet::Scale(ScaleId::Major)),
                    60.0,
                    60.0,
                );
        let spectral = SpectralFrame::from_targeted(&frame, 48_000.0, 0.0);
        assert_eq!(spectral.partials[0].target_midi, 64.0);
    }
}
