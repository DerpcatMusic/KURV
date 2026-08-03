//! Analytic source waveforms and oscillator-level spectral transforms.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Saw,
    Pulse,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpectralEffect {
    #[default]
    PhaseDisperse,
    HarmonicStretch,
    Formant,
    SpectralFold,
}
