//! Fixed-capacity, sample-rate modulation engine.
//!
//! Curves are evaluated procedurally from KURV's periodic spline
//! coefficients. No sampled LFO table, allocation, lock, or collection is
//! touched by the audio thread.

use truce_core::events::TransportInfo;

use crate::wave_curve::WaveCurveRt;

pub const LFO_COUNT: usize = 8;
pub const ROUTE_COUNT: usize = 16;

const MAX_RATE_HZ: f32 = 20_000.0;
const NYQUIST_GUARD: f32 = 0.45;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum LfoMode {
    #[default]
    Free,
    Retrigger,
    Sync,
    OneShot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum LfoRateMode {
    #[default]
    Hertz,
    Milliseconds,
    Beat,
    Keytrack,
}

impl LfoRateMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Milliseconds,
            2 => Self::Beat,
            3 => Self::Keytrack,
            _ => Self::Hertz,
        }
    }
}

impl LfoMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Retrigger,
            2 => Self::Sync,
            3 => Self::OneShot,
            _ => Self::Free,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LfoConfig {
    pub rate_hz: f32,
    pub rate_mode: LfoRateMode,
    pub mode: LfoMode,
    pub phase_offset: f32,
    pub sync_division: u8,
    pub bipolar: bool,
}

impl Default for LfoConfig {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            rate_mode: LfoRateMode::Hertz,
            mode: LfoMode::Free,
            phase_offset: 0.0,
            sync_division: 4,
            bipolar: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RouteConfig {
    /// Zero disables the route; 1..=8 selects LFO 1..=8.
    pub source: u8,
    /// Zero disables the destination; remaining values are decoded by
    /// [`ModulationFrame::accumulate`].
    pub target: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModulationFrame {
    pub pitch_semitones: [f32; 3],
    pub shape: [f32; 3],
    pub pulse_width: [f32; 3],
    pub warp: [f32; 3],
    pub level: [f32; 3],
    pub pan: [f32; 3],
    pub unison_detune_amount: [f32; 3],
}

impl ModulationFrame {
    pub fn accumulate(&mut self, route: RouteConfig, amount: f32, sources: [f32; LFO_COUNT]) {
        let source = usize::from(route.source.saturating_sub(1));
        if route.source == 0 || source >= LFO_COUNT || route.target == 0 {
            return;
        }
        let destination = usize::from(route.target - 1);
        let value = sources[source] * amount.clamp(-1.0, 1.0);
        if (18..21).contains(&destination) {
            self.unison_detune_amount[destination - 18] += value;
            return;
        }
        let oscillator = destination / 6;
        let control = destination % 6;
        if oscillator >= 3 {
            return;
        }
        match control {
            0 => self.pitch_semitones[oscillator] += value * 48.0,
            1 => self.shape[oscillator] += value * 3.0,
            2 => self.pulse_width[oscillator] += value * 0.47,
            3 => self.warp[oscillator] += value,
            4 => self.level[oscillator] += value,
            _ => self.pan[oscillator] += value,
        }
    }
}

pub struct LfoBank {
    phases: [f64; LFO_COUNT],
    last_advanced_sample: [u64; LFO_COUNT],
    one_shot_complete: [bool; LFO_COUNT],
    configs: [LfoConfig; LFO_COUNT],
    effective_rates: [f64; LFO_COUNT],
    phase_steps: [f64; LFO_COUNT],
    curves: [WaveCurveRt; LFO_COUNT],
    active_mask: u8,
    sample_clock: u64,
    sample_rate: f32,
    tempo: f64,
    transport_beats: f64,
    transport_seconds: f64,
    transport_playing: bool,
    keytrack_hz: f32,
}

impl Default for LfoBank {
    fn default() -> Self {
        Self {
            phases: [0.0; LFO_COUNT],
            last_advanced_sample: [0; LFO_COUNT],
            one_shot_complete: [false; LFO_COUNT],
            configs: [LfoConfig::default(); LFO_COUNT],
            effective_rates: [0.0; LFO_COUNT],
            phase_steps: [0.0; LFO_COUNT],
            curves: [WaveCurveRt::zero(); LFO_COUNT],
            active_mask: 0,
            sample_clock: 0,
            sample_rate: 44_100.0,
            tempo: 120.0,
            transport_beats: 0.0,
            transport_seconds: 0.0,
            transport_playing: false,
            keytrack_hz: 261.625_55,
        }
    }
}

impl LfoBank {
    pub fn reset(&mut self, sample_rate: f32) {
        self.phases = [0.0; LFO_COUNT];
        self.last_advanced_sample = [0; LFO_COUNT];
        self.one_shot_complete = [false; LFO_COUNT];
        self.sample_clock = 0;
        self.sample_rate = sample_rate.max(1.0);
        self.refresh_phase_steps();
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.catch_up_all();
        self.sample_rate = sample_rate.max(1.0);
        self.refresh_phase_steps();
    }

    pub fn configure(
        &mut self,
        configs: [LfoConfig; LFO_COUNT],
        curves: [Option<WaveCurveRt>; LFO_COUNT],
        active_mask: u8,
        transport: &TransportInfo,
        host_sample_rate: f32,
    ) {
        self.catch_up_all();
        self.configs = configs;
        for (current, update) in self.curves.iter_mut().zip(curves) {
            if let Some(update) = update {
                *current = update;
            }
        }
        self.active_mask = active_mask;
        self.tempo = if transport.tempo.is_finite() && transport.tempo > 0.0 {
            transport.tempo
        } else {
            120.0
        };
        self.transport_beats = if transport.position_beats.is_finite() {
            transport.position_beats
        } else {
            0.0
        };
        self.transport_seconds = if transport.position_seconds.is_finite()
            && (transport.position_seconds != 0.0 || transport.position_samples == 0)
        {
            transport.position_seconds
        } else {
            transport.position_samples as f64 / f64::from(host_sample_rate.max(1.0))
        };
        self.transport_playing = transport.playing;
        self.refresh_phase_steps();
    }

    pub fn note_on(&mut self, note: u8) {
        self.catch_up_all();
        self.keytrack_hz = 440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0);
        for index in 0..LFO_COUNT {
            if matches!(
                self.configs[index].mode,
                LfoMode::Retrigger | LfoMode::OneShot
            ) {
                self.phases[index] = 0.0;
                self.one_shot_complete[index] = false;
                self.last_advanced_sample[index] = self.sample_clock;
            }
        }
        self.refresh_phase_steps();
    }

    pub const fn is_active(&self) -> bool {
        self.active_mask != 0
    }

    pub fn set_active_mask(&mut self, active_mask: u8) {
        self.active_mask = active_mask;
    }

    pub fn next(&mut self) -> [f32; LFO_COUNT] {
        let mut output = [0.0; LFO_COUNT];
        for (index, value) in output.iter_mut().enumerate() {
            if self.active_mask & (1 << index) == 0 {
                continue;
            }
            self.catch_up_phase(index);
            let config = self.configs[index];
            let phase = if config.mode == LfoMode::Sync {
                let cycles = if config.rate_mode == LfoRateMode::Beat {
                    self.transport_beats / sync_beats(config.sync_division)
                } else {
                    self.transport_seconds * self.effective_rates[index]
                };
                (cycles + f64::from(config.phase_offset)).rem_euclid(1.0) as f32
            } else {
                (self.phases[index] + f64::from(config.phase_offset)).rem_euclid(1.0) as f32
            };
            let raw = if config.mode == LfoMode::OneShot && self.one_shot_complete[index] {
                self.curves[index].eval(1.0 - f32::EPSILON)
            } else {
                self.curves[index].eval(phase)
            };
            *value = if config.bipolar {
                raw.clamp(-1.0, 1.0)
            } else {
                raw.mul_add(0.5, 0.5).clamp(0.0, 1.0)
            };
            self.advance_phase(index);
        }
        self.sample_clock = self.sample_clock.wrapping_add(1);
        self.advance_transport();
        output
    }

    pub fn advance_silent(&mut self, samples: usize) {
        self.sample_clock = self.sample_clock.wrapping_add(samples as u64);
        self.advance_transport_by(samples as u64);
    }

    fn advance_phase(&mut self, index: usize) {
        let config = self.configs[index];
        if config.mode == LfoMode::Sync
            || (config.mode == LfoMode::OneShot && self.one_shot_complete[index])
        {
            self.last_advanced_sample[index] = self.sample_clock.wrapping_add(1);
            return;
        }
        let next = self.phases[index] + self.phase_steps[index];
        if config.mode == LfoMode::OneShot && next >= 1.0 {
            self.phases[index] = 1.0 - f64::EPSILON;
            self.one_shot_complete[index] = true;
        } else {
            self.phases[index] = next.rem_euclid(1.0);
        }
        self.last_advanced_sample[index] = self.sample_clock.wrapping_add(1);
    }

    fn catch_up_all(&mut self) {
        for index in 0..LFO_COUNT {
            self.catch_up_phase(index);
        }
    }

    fn catch_up_phase(&mut self, index: usize) {
        let samples = self
            .sample_clock
            .saturating_sub(self.last_advanced_sample[index]);
        if samples == 0 {
            return;
        }
        let config = self.configs[index];
        if config.mode == LfoMode::OneShot {
            let next = self.phases[index] + self.phase_steps[index] * samples as f64;
            if next >= 1.0 {
                self.phases[index] = 1.0 - f64::EPSILON;
                self.one_shot_complete[index] = true;
            } else {
                self.phases[index] = next;
            }
        } else if config.mode != LfoMode::Sync {
            self.phases[index] =
                (self.phases[index] + self.phase_steps[index] * samples as f64).rem_euclid(1.0);
        }
        self.last_advanced_sample[index] = self.sample_clock;
    }

    fn advance_transport(&mut self) {
        self.advance_transport_by(1);
    }

    fn advance_transport_by(&mut self, samples: u64) {
        if self.transport_playing {
            let seconds = samples as f64 / f64::from(self.sample_rate);
            self.transport_beats += self.tempo / 60.0 * seconds;
            self.transport_seconds += seconds;
        }
    }

    fn refresh_phase_steps(&mut self) {
        let sample_rate = self.sample_rate;
        let tempo = self.tempo;
        let keytrack_hz = self.keytrack_hz;
        for (index, config) in self.configs.into_iter().enumerate() {
            let rate = f64::from(effective_rate(config, sample_rate, tempo, keytrack_hz));
            self.effective_rates[index] = rate;
            self.phase_steps[index] = rate / f64::from(sample_rate);
        }
    }
}

fn effective_rate(config: LfoConfig, sample_rate: f32, tempo: f64, keytrack_hz: f32) -> f32 {
    let rate = match config.rate_mode {
        LfoRateMode::Hertz => config.rate_hz,
        LfoRateMode::Milliseconds => 1_000.0 / config.rate_hz.max(0.01),
        LfoRateMode::Beat => (tempo as f32 / 60.0) / sync_beats(config.sync_division) as f32,
        LfoRateMode::Keytrack => keytrack_hz * keytrack_multiplier(config.rate_hz),
    };
    rate.clamp(0.0, MAX_RATE_HZ.min(sample_rate * NYQUIST_GUARD))
}

pub fn keytrack_multiplier(rate_value: f32) -> f32 {
    let rate_value = rate_value.clamp(0.01, 20_000.0);
    if rate_value <= 1.0 {
        2.0_f32.powf(5.0 * rate_value.log10() / 2.0)
    } else {
        2.0_f32.powf(5.0 * rate_value.log10() / 20_000.0_f32.log10())
    }
}

/// Cycle duration in quarter-note beats. This includes straight, triplet,
/// and dotted choices without approximating tempo as an LFO-rate ramp.
pub const fn sync_beats(index: u8) -> f64 {
    const BEATS: [f64; 16] = [
        1.0 / 16.0,
        1.0 / 12.0,
        1.0 / 8.0,
        1.0 / 6.0,
        1.0 / 4.0,
        1.0 / 3.0,
        1.0 / 2.0,
        2.0 / 3.0,
        1.0,
        4.0 / 3.0,
        2.0,
        8.0 / 3.0,
        4.0,
        8.0,
        16.0,
        32.0,
    ];
    BEATS[if index > 15 { 15 } else { index } as usize]
}
