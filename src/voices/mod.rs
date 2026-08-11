mod oscillator_bank;
mod poly_synth;
mod unison;
mod voice;

use crate::generators::MAX_OSCILLATORS;

pub const MAX_UNISON: usize = 64;
pub type OscillatorMask = u32;
pub(crate) const OSCILLATOR_BANK_SIZE: usize = MAX_OSCILLATORS;
const MAX_UNISON_U8: u8 = 64;

use oscillator_bank::unit_hash;
pub(crate) use oscillator_bank::{
    OscillatorDspConfig, StructuralOscillatorFrameControl, fast_exp2, fill_oscillator_unison_layout,
};
pub(crate) use poly_synth::PolySynth;
pub(crate) use voice::*;
