//! Worker-side pitch-family and spectral metadata primitives.
//!
//! The renderer consumes prepared frames; it never runs pitch detection or
//! allocates these structures. Candidate extraction can be replaced by a
//! stronger detector without changing the realtime target policy.

use super::targeting::{PitchMode, target_correction};

pub const MAX_PITCH_FAMILIES: usize = 8;
pub const HARMONIC_MASK_WORDS: usize = 64;
pub const HARMONIC_MASK_BINS: usize = HARMONIC_MASK_WORDS * 64;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PitchCandidate {
    pub midi: f32,
    pub strength: f32,
    pub confidence: f32,
}

impl PitchCandidate {
    #[must_use]
    pub const fn new(midi: f32, strength: f32, confidence: f32) -> Self {
        Self {
            midi,
            strength,
            confidence,
        }
    }

    #[must_use]
    fn score(self) -> f32 {
        self.strength.max(0.0) * self.confidence.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PitchFamily {
    pub midi: f32,
    pub confidence: f32,
    pub strength: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PitchFrame {
    pub frame_index: u32,
    pub energy: f32,
    pub onset: f32,
    pub families: [PitchFamily; MAX_PITCH_FAMILIES],
    pub family_count: u8,
}

impl PitchFrame {
    #[must_use]
    pub fn from_candidates(
        frame_index: u32,
        energy: f32,
        onset: f32,
        candidates: &[PitchCandidate],
    ) -> Self {
        let mut ranked = [PitchCandidate::default(); MAX_PITCH_FAMILIES];
        let mut ranked_count = 0_usize;
        for candidate in candidates.iter().copied().filter(|candidate| {
            candidate.midi.is_finite()
                && (0.0..=140.0).contains(&candidate.midi)
                && candidate.strength.is_finite()
                && candidate.confidence.is_finite()
                && candidate.score() > 0.0
        }) {
            let insertion = ranked[..ranked_count]
                .iter()
                .position(|other| candidate.score() > other.score())
                .unwrap_or(ranked_count);
            if ranked_count < MAX_PITCH_FAMILIES {
                ranked_count += 1;
            } else if insertion >= MAX_PITCH_FAMILIES {
                continue;
            }
            let last = ranked_count.saturating_sub(1);
            let shift_end = last.min(MAX_PITCH_FAMILIES - 1);
            for index in (insertion..shift_end).rev() {
                ranked[index + 1] = ranked[index];
            }
            ranked[insertion] = candidate;
        }

        let mut frame = Self {
            frame_index,
            energy: energy.max(0.0),
            onset: onset.clamp(0.0, 1.0),
            ..Self::default()
        };
        for candidate in ranked.iter().copied().take(ranked_count) {
            if frame.family_count as usize >= MAX_PITCH_FAMILIES {
                break;
            }
            if frame
                .families
                .iter()
                .take(usize::from(frame.family_count))
                .any(|family| {
                    is_duplicate_family(
                        candidate.midi,
                        family.midi,
                        candidate.confidence,
                        family.confidence,
                    )
                })
            {
                continue;
            }
            frame.families[usize::from(frame.family_count)] = PitchFamily {
                midi: candidate.midi,
                confidence: candidate.confidence.clamp(0.0, 1.0),
                strength: candidate.strength.max(0.0),
            };
            frame.family_count += 1;
        }
        frame
    }

    #[must_use]
    pub fn targeted(
        self,
        mode: PitchMode,
        played_midi: f32,
        source_root_midi: f32,
    ) -> TargetedPitchFrame {
        let mut targeted = TargetedPitchFrame {
            frame_index: self.frame_index,
            energy: self.energy,
            onset: self.onset,
            ..TargetedPitchFrame::default()
        };
        for family in self
            .families
            .iter()
            .copied()
            .take(usize::from(self.family_count))
        {
            let correction = if played_midi.is_finite() && source_root_midi.is_finite() {
                match mode {
                    PitchMode::Classic => 0.0,
                    PitchMode::Spectral => played_midi - source_root_midi,
                    PitchMode::Target(_) => target_correction(mode, family.midi, played_midi),
                }
            } else {
                0.0
            };
            let index = usize::from(targeted.family_count);
            targeted.families[index] = TargetedPitch {
                source_midi: family.midi,
                target_midi: family.midi + correction,
                correction,
                confidence: family.confidence,
                strength: family.strength,
            };
            targeted.family_count += 1;
        }
        targeted
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetedPitch {
    pub source_midi: f32,
    pub target_midi: f32,
    pub correction: f32,
    pub confidence: f32,
    pub strength: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetedPitchFrame {
    pub frame_index: u32,
    pub energy: f32,
    pub onset: f32,
    pub families: [TargetedPitch; MAX_PITCH_FAMILIES],
    pub family_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HarmonicMask {
    words: [u64; HARMONIC_MASK_WORDS],
}

impl Default for HarmonicMask {
    fn default() -> Self {
        Self {
            words: [0; HARMONIC_MASK_WORDS],
        }
    }
}

impl HarmonicMask {
    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    pub fn set(&mut self, bin: usize, tonal: bool) {
        if bin >= HARMONIC_MASK_BINS {
            return;
        }
        let word = bin / 64;
        let bit = 1_u64 << (bin % 64);
        if tonal {
            self.words[word] |= bit;
        } else {
            self.words[word] &= !bit;
        }
    }

    #[must_use]
    pub const fn is_tonal(&self, bin: usize) -> bool {
        if bin >= HARMONIC_MASK_BINS {
            return false;
        }
        self.words[bin / 64] & (1_u64 << (bin % 64)) != 0
    }

    #[must_use]
    pub const fn tonal_weight(&self, bin: usize) -> f32 {
        if self.is_tonal(bin) { 1.0 } else { 0.0 }
    }
}

#[inline]
fn is_duplicate_family(
    candidate_midi: f32,
    existing_midi: f32,
    candidate_confidence: f32,
    existing_confidence: f32,
) -> bool {
    let distance = (candidate_midi - existing_midi).abs();
    if distance < 0.5 {
        return true;
    }
    let candidate_hz = 440.0 * 2.0_f32.powf((candidate_midi - 69.0) / 12.0);
    let existing_hz = 440.0 * 2.0_f32.powf((existing_midi - 69.0) / 12.0);
    let ratio = candidate_hz / existing_hz.max(1.0e-6);
    let harmonic =
        (2..=8).any(|multiple| (ratio - multiple as f32).abs() < 0.035 * multiple as f32);
    harmonic && existing_confidence >= candidate_confidence * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscillators::resynth::targeting::{ScaleId, TargetSet};

    #[test]
    fn frame_keeps_ranked_pitch_families_and_metadata() {
        let candidates = [
            PitchCandidate::new(64.0, 0.4, 0.8),
            PitchCandidate::new(60.0, 1.0, 1.0),
            PitchCandidate::new(67.0, 0.7, 0.9),
        ];
        let frame = PitchFrame::from_candidates(12, 0.75, 1.4, &candidates);
        assert_eq!(frame.frame_index, 12);
        assert_eq!(frame.energy, 0.75);
        assert_eq!(frame.onset, 1.0);
        assert_eq!(frame.family_count, 3);
        assert_eq!(frame.families[0].midi, 60.0);
        assert_eq!(frame.families[1].midi, 67.0);
    }

    #[test]
    fn harmonic_duplicate_is_suppressed_but_polyphonic_interval_survives() {
        let candidates = [
            PitchCandidate::new(60.0, 1.0, 1.0),
            PitchCandidate::new(72.0, 0.8, 0.8),
            PitchCandidate::new(67.0, 0.7, 0.8),
        ];
        let frame = PitchFrame::from_candidates(0, 1.0, 0.0, &candidates);
        assert_eq!(frame.family_count, 2);
        assert!(
            frame.families[..2]
                .iter()
                .any(|family| (family.midi - 60.0).abs() < 0.01)
        );
        assert!(
            frame.families[..2]
                .iter()
                .any(|family| (family.midi - 67.0).abs() < 0.01)
        );
    }

    #[test]
    fn target_policy_operates_independently_on_each_family() {
        let frame = PitchFrame::from_candidates(
            3,
            0.5,
            0.2,
            &[
                PitchCandidate::new(63.0, 1.0, 1.0),
                PitchCandidate::new(65.0, 0.8, 1.0),
            ],
        );
        let targeted = frame.targeted(
            PitchMode::Target(TargetSet::Scale(ScaleId::Major)),
            60.0,
            60.0,
        );
        assert_eq!(targeted.family_count, 2);
        assert!((targeted.families[0].target_midi - 64.0).abs() < 1.0e-4);
        assert!((targeted.families[1].target_midi - 65.0).abs() < 1.0e-4);
    }

    #[test]
    fn spectral_policy_preserves_intervals_while_shifting_from_source_root() {
        let frame = PitchFrame::from_candidates(
            0,
            1.0,
            0.0,
            &[
                PitchCandidate::new(60.0, 1.0, 1.0),
                PitchCandidate::new(67.0, 0.8, 1.0),
            ],
        );
        let targeted = frame.targeted(PitchMode::Spectral, 64.0, 60.0);
        assert_eq!(targeted.families[0].target_midi, 64.0);
        assert_eq!(targeted.families[1].target_midi, 71.0);
        assert_eq!(
            targeted.families[1].target_midi - targeted.families[0].target_midi,
            7.0
        );
    }

    #[test]
    fn harmonic_mask_is_bounded_and_bit_exact() {
        let mut mask = HarmonicMask::default();
        mask.set(0, true);
        mask.set(127, true);
        mask.set(HARMONIC_MASK_BINS, true);
        assert!(mask.is_tonal(0));
        assert!(mask.is_tonal(127));
        assert!(!mask.is_tonal(HARMONIC_MASK_BINS));
        mask.set(127, false);
        assert!(!mask.is_tonal(127));
        assert_eq!(mask.tonal_weight(0), 1.0);
    }
}
