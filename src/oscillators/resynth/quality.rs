//! Machine-local RESYNTH analysis quality.
//!
//! The pitch curve and RICH reconstruction hop/window are compiled on the
//! worker. Playback only looks the curve up. Changing quality rebuilds
//! artifacts; it never runs in the audio callback.

use std::sync::atomic::{AtomicU8, Ordering};

static CURRENT: AtomicU8 = AtomicU8::new(ResynthQuality::Standard as u8);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum ResynthQuality {
    Eco = 0,
    #[default]
    Standard = 1,
    High = 2,
    Ultra = 3,
}

impl ResynthQuality {
    pub const ALL: [Self; 4] = [Self::Eco, Self::Standard, Self::High, Self::Ultra];

    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Eco,
            2 => Self::High,
            3 => Self::Ultra,
            _ => Self::Standard,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Eco => "ECO",
            Self::Standard => "STANDARD",
            Self::High => "HIGH",
            Self::Ultra => "ULTRA",
        }
    }

    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Eco => "Coarse pitch map, fewest RICH harmonics",
            Self::Standard => "Default pitch map and RICH harmonic count",
            Self::High => "Finer pitch map, more RICH harmonics",
            Self::Ultra => "Finest pitch map and RICH harmonic count",
        }
    }

    #[must_use]
    pub const fn hop_seconds(self) -> f32 {
        match self {
            Self::Eco => 0.02,
            Self::Standard => 0.01,
            Self::High => 0.005,
            Self::Ultra => 0.002_5,
        }
    }

    #[must_use]
    pub const fn window_seconds(self) -> f32 {
        match self {
            Self::Eco => 0.064,
            Self::Standard => 0.096,
            Self::High => 0.096,
            Self::Ultra => 0.128,
        }
    }

    #[must_use]
    pub const fn max_points(self) -> usize {
        match self {
            Self::Eco => 512,
            Self::Standard => 2_048,
            Self::High => 4_096,
            Self::Ultra => 8_192,
        }
    }

    #[must_use]
    pub const fn fft_size(self) -> usize {
        match self {
            Self::Eco | Self::Standard => 2_048,
            Self::High | Self::Ultra => 4_096,
        }
    }

    #[must_use]
    pub const fn pitch_fft_size(self) -> usize {
        match self {
            Self::Eco => 8_192,
            Self::Standard => 16_384,
            Self::High => 32_768,
            Self::Ultra => 65_536,
        }
    }

    #[must_use]
    pub const fn reconstruction_hop(self) -> usize {
        self.fft_size()
            / match self {
                Self::Eco => 2,
                Self::Standard => 4,
                Self::High | Self::Ultra => 8,
            }
    }

    #[must_use]
    pub const fn max_harmonics(self) -> usize {
        match self {
            Self::Eco => 48,
            Self::Standard => 96,
            Self::High | Self::Ultra => 128,
        }
    }

    #[must_use]
    pub fn locked_grain_density(self) -> f32 {
        self.locked_grain_density_at(48_000.0)
    }

    #[must_use]
    pub fn locked_grain_size(self) -> f32 {
        self.locked_grain_size_at(48_000.0)
    }

    #[must_use]
    pub fn locked_grain_density_at(self, sample_rate: f32) -> f32 {
        (sample_rate.max(1.0) / self.reconstruction_hop() as f32).clamp(1.0, 2_000.0)
    }

    #[must_use]
    pub fn locked_grain_size_at(self, sample_rate: f32) -> f32 {
        let seconds = (self.fft_size() as f32 / sample_rate.max(1.0)).clamp(0.005, 1.0);
        (seconds / 0.005).log(200.0).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn current() -> Self {
        Self::from_u8(CURRENT.load(Ordering::Acquire))
    }

    pub fn set_current(self) {
        CURRENT.store(self as u8, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_wire_round_trips() {
        for quality in ResynthQuality::ALL {
            assert_eq!(ResynthQuality::from_u8(quality as u8), quality);
        }
        assert_eq!(ResynthQuality::from_u8(99), ResynthQuality::Standard);
    }

    #[test]
    fn finer_quality_has_more_detail() {
        assert!(ResynthQuality::Ultra.hop_seconds() < ResynthQuality::Eco.hop_seconds());
        assert!(ResynthQuality::Ultra.max_points() > ResynthQuality::Eco.max_points());
        assert!(
            ResynthQuality::Ultra.reconstruction_hop() < ResynthQuality::Eco.reconstruction_hop()
        );
        assert!(ResynthQuality::Ultra.max_harmonics() > ResynthQuality::Eco.max_harmonics());
    }
}
