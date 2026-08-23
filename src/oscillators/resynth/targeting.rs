//! Ideal pitch-mode targeting: what each detected pitch family should become.
//!
//! Pure decision logic with no DSP, allocation or threading. The spectral
//! renderer consumes [`target_note`] results per detected harmonic family;
//! classic rendering ignores this module entirely.

/// Top-level pitch behavior of the resynth oscillator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum PitchMode {
    /// Ordinary granular resampling. Pitch and read speed stay coupled and
    /// no spectral correction is applied.
    #[default]
    Classic = 0,
    /// Duration-preserving free spectral shift by the played-note interval.
    /// Polyphonic intervals inside the source remain intact.
    Spectral = 1,
    /// Spectral correction toward an absolute target set.
    Target(TargetSet) = 2,
}

impl PitchMode {
    pub const COUNT: usize = 3;

    pub const fn index(self) -> usize {
        match self {
            Self::Classic => 0,
            Self::Spectral => 1,
            Self::Target(_) => 2,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Spectral => "Spectral",
            Self::Target(_) => "Target",
        }
    }
}

/// Absolute correction target for [`PitchMode::Target`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum TargetSet {
    /// Correct every detected pitch family toward the exact played MIDI note,
    /// including its absolute octave.
    #[default]
    PlayedNote = 0,
    /// Map every detected polyphonic pitch family independently to the nearest
    /// absolute note of the selected scale.
    Scale(ScaleId) = 1,
}

/// Supported correction scales. Semitone sets are absolute within an octave.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum ScaleId {
    #[default]
    Chromatic = 0,
    Major = 1,
    NaturalMinor = 2,
    HarmonicMinor = 3,
    Dorian = 4,
    Mixolydian = 5,
    MajorPentatonic = 6,
    MinorPentatonic = 7,
    WholeTone = 8,
}

impl ScaleId {
    pub const ALL: [Self; 9] = [
        Self::Chromatic,
        Self::Major,
        Self::NaturalMinor,
        Self::HarmonicMinor,
        Self::Dorian,
        Self::Mixolydian,
        Self::MajorPentatonic,
        Self::MinorPentatonic,
        Self::WholeTone,
    ];

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Chromatic),
            1 => Some(Self::Major),
            2 => Some(Self::NaturalMinor),
            3 => Some(Self::HarmonicMinor),
            4 => Some(Self::Dorian),
            5 => Some(Self::Mixolydian),
            6 => Some(Self::MajorPentatonic),
            7 => Some(Self::MinorPentatonic),
            8 => Some(Self::WholeTone),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Chromatic => "Chromatic",
            Self::Major => "Major",
            Self::NaturalMinor => "Natural Minor",
            Self::HarmonicMinor => "Harmonic Minor",
            Self::Dorian => "Dorian",
            Self::Mixolydian => "Mixolydian",
            Self::MajorPentatonic => "Major Pentatonic",
            Self::MinorPentatonic => "Minor Pentatonic",
            Self::WholeTone => "Whole Tone",
        }
    }

    /// Ascending semitone offsets from the scale root, spanning one octave.
    pub const fn degrees(self) -> &'static [u8] {
        match self {
            Self::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::MajorPentatonic => &[0, 2, 4, 7, 9],
            Self::MinorPentatonic => &[0, 3, 5, 7, 10],
            Self::WholeTone => &[0, 2, 4, 6, 8, 10],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn played_note_targets_the_exact_midi_note() {
        let correction = target_correction(PitchMode::Target(TargetSet::PlayedNote), 57.3, 60.0);
        assert!((correction - 2.7).abs() < 1.0e-4);
    }

    #[test]
    fn classic_and_spectral_apply_no_absolute_correction() {
        for mode in [PitchMode::Classic, PitchMode::Spectral] {
            let correction = target_correction(mode, 57.3, 60.0);
            assert_eq!(correction, 0.0);
        }
    }

    #[test]
    fn scale_target_snaps_each_family_to_the_nearest_scale_note() {
        // C major over MIDI root C: D# (63) is a semitone below E (64).
        let scale = PitchMode::Target(TargetSet::Scale(ScaleId::Major));
        assert!((target_correction(scale, 63.0, 60.0) - 1.0).abs() < 1.0e-4);
        // B-flat (70) is equidistant from A and B; ties resolve upward.
        let correction_b_flat = target_correction(scale, 70.0, 60.0);
        assert!((correction_b_flat - 1.0).abs() < 1.0e-4);
    }

    #[test]
    fn scale_target_preserves_octave_of_detected_pitch() {
        let scale = PitchMode::Target(TargetSet::Scale(ScaleId::Major));
        // Same pitch class three octaves up keeps the same relative correction.
        let low = target_correction(scale, 63.0, 60.0);
        let high = target_correction(scale, 63.0 + 36.0, 60.0);
        assert!((low - high).abs() < 1.0e-4);
    }

    #[test]
    fn pentatonic_snaps_by_minimum_distance_not_rounding_down() {
        // In C major pentatonic, F (65) sits between E (64) and G (67);
        // nearest is E, one semitone down.
        let scale = PitchMode::Target(TargetSet::Scale(ScaleId::MajorPentatonic));
        assert!((target_correction(scale, 65.0, 60.0) + 1.0).abs() < 1.0e-4);
    }

    #[test]
    fn chromatic_target_is_a_no_op_for_semitone_pitches() {
        let scale = PitchMode::Target(TargetSet::Scale(ScaleId::Chromatic));
        for pitch in [21.0, 63.0, 108.0] {
            assert_eq!(target_correction(scale, pitch, 60.0), 0.0);
        }
    }

    #[test]
    fn chromatic_target_quantizes_microtonal_detune_to_the_semitone() {
        let scale = PitchMode::Target(TargetSet::Scale(ScaleId::Chromatic));
        assert!((target_correction(scale, 57.3, 60.0) + 0.3).abs() < 1.0e-4);
        assert!((target_correction(scale, 57.6, 60.0) - 0.4).abs() < 1.0e-4);
    }

    #[test]
    fn played_note_interval_is_relative_to_played_note_not_root() {
        // Detected family at 64 against played note 67 must move +3.
        let mode = PitchMode::Target(TargetSet::PlayedNote);
        assert!((target_correction(mode, 64.0, 67.0) - 3.0).abs() < 1.0e-4);
    }

    #[test]
    fn scale_ids_round_trip() {
        for value in 0..ScaleId::ALL.len() as u8 {
            assert_eq!(
                ScaleId::from_u8(value).map(|scale| scale as u8),
                Some(value)
            );
        }
        assert!(ScaleId::from_u8(9).is_none());
    }

    #[test]
    fn pitch_modes_have_stable_wire_tags_and_scale_payloads() {
        for mode in [
            PitchMode::Classic,
            PitchMode::Spectral,
            PitchMode::Target(TargetSet::PlayedNote),
            PitchMode::Target(TargetSet::Scale(ScaleId::Dorian)),
        ] {
            let (tag, scale) = mode.to_wire();
            assert_eq!(PitchMode::from_wire(tag, scale), Some(mode));
        }
        assert!(PitchMode::from_wire(99, 0).is_none());
        assert!(PitchMode::from_wire(2, 99).is_none());
    }
}

impl PitchMode {
    #[must_use]
    pub const fn to_wire(self) -> (u8, u8) {
        match self {
            Self::Classic => (0, 0),
            Self::Spectral => (1, 0),
            Self::Target(TargetSet::PlayedNote) => (2, 0),
            Self::Target(TargetSet::Scale(scale)) => (2, scale as u8 + 1),
        }
    }

    #[must_use]
    pub const fn from_wire(tag: u8, scale: u8) -> Option<Self> {
        match tag {
            0 if scale == 0 => Some(Self::Classic),
            1 if scale == 0 => Some(Self::Spectral),
            2 if scale == 0 => Some(Self::Target(TargetSet::PlayedNote)),
            2 => match ScaleId::from_u8(scale - 1) {
                Some(scale) => Some(Self::Target(TargetSet::Scale(scale))),
                None => None,
            },
            _ => None,
        }
    }
}

/// Semitone correction applied to one detected pitch family.
///
/// * `detected` is the family's absolute MIDI pitch.
/// * `played` is the exact played MIDI note.
/// Classic and spectral free shift never correct absolutely; only
/// [`TargetSet`] modes do.
pub fn target_correction(mode: PitchMode, detected: f32, played: f32) -> f32 {
    match mode {
        PitchMode::Classic | PitchMode::Spectral => 0.0,
        PitchMode::Target(TargetSet::PlayedNote) => played - detected,
        PitchMode::Target(TargetSet::Scale(scale)) => {
            // Snap on the pitch-class circle relative to the scale root that
            // follows the played note. Equal distances resolve upward so the
            // mapping is deterministic and always lands inside the octave.
            let root_class = played.rem_euclid(12.0);
            let detected_class = detected.rem_euclid(12.0);
            let mut best_offset = 0.0_f32;
            let mut best_key = (f32::INFINITY, f32::NEG_INFINITY);
            for degree in scale.degrees() {
                let mut offset =
                    (root_class + f32::from(*degree) - detected_class).rem_euclid(12.0);
                if offset > 6.0 {
                    offset -= 12.0;
                }
                let key = (offset.abs(), -offset);
                if key < best_key {
                    best_key = key;
                    best_offset = offset;
                }
            }
            best_offset
        }
    }
}
