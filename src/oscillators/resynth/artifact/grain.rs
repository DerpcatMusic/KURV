use std::f32::consts::TAU;

use super::super::analysis::PreparedPitchFrameBank;
use super::super::scheduler::density_plan;
use super::super::{GrainDirection, ResynthControls};
use super::shared::*;
use crate::dsp::splitmix64;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

const PITCH_TRACK_PERIOD_SECONDS: f32 = 0.01;
const MIN_PITCH_TRACK_WINDOW_SECONDS: f32 = 0.048;
const MAX_PITCH_TRACK_POINTS: usize = 2_048;

#[cfg(test)]
static SOURCE_READS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub struct GrainSourceArtifact {
    pub source_sample_rate: f32,
    pub root_hz: Option<f32>,
    pub controls: ResynthControls,
    pub(crate) samples: Box<[f32]>,
    pub(super) reflected_mips: Box<[ReflectedMipLevel]>,
    pub(crate) side_samples: Box<[f32]>,
    side_mips: Box<[ReflectedMipLevel]>,
    pub(crate) tuned_samples: Box<[f32]>,
    tuned_mips: Box<[ReflectedMipLevel]>,
    pub(crate) tuned_side_samples: Box<[f32]>,
    tuned_side_mips: Box<[ReflectedMipLevel]>,
    pub(crate) transients: Box<[u32]>,
    pub(crate) pitch_frames: PreparedPitchFrameBank,
}

impl GrainSourceArtifact {
    #[must_use]
    pub fn silence() -> Self {
        Self {
            source_sample_rate: 48_000.0,
            root_hz: None,
            controls: ResynthControls::default(),
            samples: vec![0.0].into_boxed_slice(),
            reflected_mips: Vec::new().into_boxed_slice(),
            side_samples: Vec::new().into_boxed_slice(),
            side_mips: Vec::new().into_boxed_slice(),
            tuned_samples: Vec::new().into_boxed_slice(),
            tuned_mips: Vec::new().into_boxed_slice(),
            tuned_side_samples: Vec::new().into_boxed_slice(),
            tuned_side_mips: Vec::new().into_boxed_slice(),
            transients: Vec::new().into_boxed_slice(),
            pitch_frames: PreparedPitchFrameBank::default(),
        }
    }

    pub fn compile(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: Option<f32>,
        controls: ResynthControls,
    ) -> Result<Self, ArtifactBuildError> {
        Self::compile_with_cancel(source, source_sample_rate, root_hz, controls, &|| false)
    }

    pub(crate) fn compile_with_cancel(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: Option<f32>,
        controls: ResynthControls,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        Self::compile_channels_with_cancel(
            source,
            None,
            source_sample_rate,
            root_hz,
            controls,
            should_cancel,
        )
    }

    pub(crate) fn compile_channels_with_cancel(
        mid: &[f32],
        side: Option<&[f32]>,
        source_sample_rate: u32,
        root_hz: Option<f32>,
        controls: ResynthControls,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        validate_source(mid)?;
        if let Some(side) = side {
            validate_source(side)?;
        }
        let stride = mid.len().div_ceil(GRAIN_MAX_SOURCE_FRAMES).max(1);
        let projection = bandlimit_source_by_stride_with_cancel(
            mid,
            source_sample_rate as f32,
            stride,
            should_cancel,
        )?;
        debug_assert_eq!(projection.stride, stride);
        let mut retained = projection.samples;
        let mut retained_side = if let Some(side) = side {
            let side = bandlimit_source_by_stride_with_cancel(
                side,
                source_sample_rate as f32,
                stride,
                should_cancel,
            )?;
            debug_assert_eq!(side.samples.len(), retained.len());
            side.samples
        } else {
            Vec::new()
        };
        if retained_side.is_empty() {
            remove_dc_and_peak_normalize(&mut retained);
        } else {
            remove_dc_and_stereo_peak_normalize(&mut retained, &mut retained_side);
        }
        let pitch_map = root_hz
            .map(|_| {
                build_pitch_map_with_cancel(&retained, projection.sample_rate as u32, should_cancel)
            })
            .transpose()?;
        let mean_flux = retained
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .sum::<f32>()
            / retained.len().max(2) as f32;
        let mut transients = Vec::with_capacity(128);
        let mut last = 0_usize;
        for index in 2..retained.len().saturating_sub(2) {
            if index & 4_095 == 0 && should_cancel() {
                return Err(ArtifactBuildError::Cancelled);
            }
            let flux = (retained[index] - retained[index - 1]).abs();
            if flux > mean_flux * 6.0
                && flux >= (retained[index - 1] - retained[index - 2]).abs()
                && flux > (retained[index + 1] - retained[index]).abs()
                && index.saturating_sub(last) >= 256
            {
                transients.push(u32::try_from(index).unwrap_or(u32::MAX));
                last = index;
                if transients.len() == 128 {
                    break;
                }
            }
        }
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let reflected_mips = build_reflected_mips_with_cancel(&retained, should_cancel)?;
        let side_mips = build_reflected_mips_with_cancel(&retained_side, should_cancel)?;
        let (tuned_samples, tuned_side_samples) = root_hz.map_or_else(
            || Ok((Vec::new(), Vec::new())),
            |root_hz| {
                let pitch_map = pitch_map.as_deref().unwrap_or_default();
                super::spectral_tune::tune_stereo_with_cancel(
                    &retained,
                    &retained_side,
                    projection.sample_rate,
                    root_hz,
                    |position| pitch_at(pitch_map, position, root_hz),
                    should_cancel,
                )
            },
        )?;
        let tuned_mips = build_reflected_mips_with_cancel(&tuned_samples, should_cancel)?;
        let tuned_side_mips = build_reflected_mips_with_cancel(&tuned_side_samples, should_cancel)?;
        let pitch_frames = PreparedPitchFrameBank::from_pitch_track(
            pitch_map.as_deref().unwrap_or_default(),
            retained.len(),
            &transients,
        );
        Ok(Self {
            source_sample_rate: projection.sample_rate,
            root_hz,
            controls: controls.sanitized(),
            samples: retained.into_boxed_slice(),
            reflected_mips,
            side_samples: retained_side.into_boxed_slice(),
            side_mips,
            tuned_samples: tuned_samples.into_boxed_slice(),
            tuned_mips,
            tuned_side_samples: tuned_side_samples.into_boxed_slice(),
            tuned_side_mips,
            pitch_frames,
            transients: transients.into_boxed_slice(),
        })
    }

    #[must_use]
    pub(crate) fn from_persisted(
        source_sample_rate: f32,
        root_hz: Option<f32>,
        controls: ResynthControls,
        samples: Box<[f32]>,
        transients: Box<[u32]>,
    ) -> Self {
        Self::from_persisted_channels(
            source_sample_rate,
            root_hz,
            controls,
            samples,
            Vec::new().into_boxed_slice(),
            Vec::new().into_boxed_slice(),
            Vec::new().into_boxed_slice(),
            transients,
        )
    }

    #[must_use]
    pub(crate) fn from_persisted_with_channels(
        source_sample_rate: f32,
        root_hz: Option<f32>,
        controls: ResynthControls,
        samples: Box<[f32]>,
        side_samples: Box<[f32]>,
        tuned_samples: Box<[f32]>,
        tuned_side_samples: Box<[f32]>,
        transients: Box<[u32]>,
    ) -> Self {
        Self::from_persisted_channels(
            source_sample_rate,
            root_hz,
            controls,
            samples,
            side_samples,
            tuned_samples,
            tuned_side_samples,
            transients,
        )
    }

    fn from_persisted_channels(
        source_sample_rate: f32,
        root_hz: Option<f32>,
        controls: ResynthControls,
        samples: Box<[f32]>,
        side_samples: Box<[f32]>,
        tuned_samples: Box<[f32]>,
        tuned_side_samples: Box<[f32]>,
        transients: Box<[u32]>,
    ) -> Self {
        // Persisted PCM was already normalized when the build produced it.
        // Re-normalizing here would corrupt DC-bearing content and break
        // bit-exact restore, so the stored samples are authoritative.
        let pitch_track =
            build_pitch_map_with_cancel(&samples, source_sample_rate.max(1.0) as u32, &|| false)
                .unwrap_or_default();
        let pitch_frames =
            PreparedPitchFrameBank::from_pitch_track(&pitch_track, samples.len(), &transients);
        Self {
            source_sample_rate,
            root_hz,
            controls: controls.sanitized(),
            reflected_mips: build_reflected_mips(&samples),
            side_mips: build_reflected_mips(&side_samples),
            tuned_mips: build_reflected_mips(&tuned_samples),
            tuned_side_mips: build_reflected_mips(&tuned_side_samples),
            pitch_frames,
            samples,
            side_samples,
            tuned_samples,
            tuned_side_samples,
            transients,
        }
    }

    #[inline]
    pub(super) fn pitch_frame_at(&self, position: f32) -> super::super::analysis::PitchFrame {
        self.pitch_frames.frame_at(position)
    }

    #[inline]
    pub(super) fn sample_filtered(&self, position: f32, source_step: f32) -> f32 {
        #[cfg(test)]
        SOURCE_READS.fetch_add(1, Ordering::Relaxed);
        reflected_mip_sample(&self.samples, &self.reflected_mips, position, source_step)
    }

    #[inline]
    fn sample_side_filtered(&self, position: f32, source_step: f32) -> f32 {
        #[cfg(test)]
        SOURCE_READS.fetch_add(1, Ordering::Relaxed);
        if self.side_samples.is_empty() {
            0.0
        } else {
            reflected_mip_sample(&self.side_samples, &self.side_mips, position, source_step)
        }
    }

    #[inline]
    fn sample_tuned_filtered(&self, position: f32, source_step: f32) -> f32 {
        #[cfg(test)]
        SOURCE_READS.fetch_add(1, Ordering::Relaxed);
        if self.tuned_samples.is_empty() {
            self.sample_filtered(position, source_step)
        } else {
            reflected_mip_sample(&self.tuned_samples, &self.tuned_mips, position, source_step)
        }
    }

    #[inline]
    fn sample_tuned_side_filtered(&self, position: f32, source_step: f32) -> f32 {
        #[cfg(test)]
        SOURCE_READS.fetch_add(1, Ordering::Relaxed);
        if self.tuned_side_samples.is_empty() {
            self.sample_side_filtered(position, source_step)
        } else {
            reflected_mip_sample(
                &self.tuned_side_samples,
                &self.tuned_side_mips,
                position,
                source_step,
            )
        }
    }
}

#[cfg(test)]
pub(super) fn reset_source_reads() {
    SOURCE_READS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(super) fn source_reads() -> usize {
    SOURCE_READS.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug)]
pub struct GrainLayerState {
    pub(super) position: f32,
    pub(super) age: u32,
    pub(super) length: u32,
    pub(super) source_step: f32,
    pub(super) gain: f32,
    pub(super) pan: f32,
    pub(super) pitch: f32,
    pub(super) active: bool,
}

impl Default for GrainLayerState {
    fn default() -> Self {
        Self {
            position: 0.0,
            age: 0,
            length: 0,
            source_step: 0.0,
            gain: 1.0,
            pan: 0.0,
            pitch: 0.0,
            active: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GrainSchedulerState {
    pub(super) layers: [GrainLayerState; GRAIN_LAYERS],
    filter_states: [f32; GRAIN_LAYERS],
    side_filter_states: [f32; GRAIN_LAYERS],
    pan_left: [f32; GRAIN_LAYERS],
    pan_right: [f32; GRAIN_LAYERS],
    active_indices: [u8; GRAIN_LAYERS],
    active_count: u8,
    event: u64,
    spawn_countdown: f32,
    cursor: f32,
    last_position: f32,
    cursor_forward: bool,
    cursor_valid: bool,
    cached_frame: u64,
    cache_valid: bool,
}

const _: () = assert!(std::mem::size_of::<GrainSchedulerState>() <= 8 * 1024);

impl Default for GrainSchedulerState {
    fn default() -> Self {
        Self {
            layers: [GrainLayerState::default(); GRAIN_LAYERS],
            filter_states: [0.0; GRAIN_LAYERS],
            side_filter_states: [0.0; GRAIN_LAYERS],
            pan_left: [1.0; GRAIN_LAYERS],
            pan_right: [1.0; GRAIN_LAYERS],
            active_indices: [0; GRAIN_LAYERS],
            active_count: 0,
            event: 0,
            spawn_countdown: 0.0,
            cursor: 0.0,
            last_position: 0.0,
            cursor_forward: true,
            cursor_valid: false,
            cached_frame: 0,
            cache_valid: false,
        }
    }
}

#[cfg(test)]
pub(super) fn grain_density_count(controls: ResynthControls) -> usize {
    controls
        .grain_density
        .round()
        .clamp(1.0, GRAIN_LAYERS as f32) as usize
}

#[inline]
fn effective_spectral_tune(controls: ResynthControls) -> f32 {
    match controls.pitch_mode {
        super::super::PitchMode::Classic => 0.0,
        super::super::PitchMode::Spectral | super::super::PitchMode::Target(_) => {
            controls.grain_tune.clamp(0.0, 1.0)
        }
    }
}

#[inline]
fn gate_spectral_tune(tune: f32, frame: super::super::analysis::PitchFrame) -> f32 {
    if tune <= 0.0 || frame.family_count == 0 {
        return 0.0;
    }
    let confidence = frame.families[0].confidence.clamp(0.0, 1.0);
    (tune * confidence * (1.0 - frame.onset.clamp(0.0, 1.0))).clamp(0.0, 1.0)
}

impl GrainSchedulerState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn active_count(&self) -> usize {
        self.layers.iter().filter(|layer| layer.active).count()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn write_telemetry(
        &self,
        source_len: usize,
        positions: &mut [f32; GRAIN_TELEMETRY],
        progress: &mut [f32; GRAIN_TELEMETRY],
        gains: &mut [f32; GRAIN_TELEMETRY],
    ) -> u8 {
        self.write_telemetry_ex(source_len, positions, progress, gains, None, None)
    }

    pub(crate) fn write_telemetry_ex(
        &self,
        source_len: usize,
        positions: &mut [f32; GRAIN_TELEMETRY],
        progress: &mut [f32; GRAIN_TELEMETRY],
        gains: &mut [f32; GRAIN_TELEMETRY],
        mut pans: Option<&mut [f32; GRAIN_TELEMETRY]>,
        mut pitches: Option<&mut [f32; GRAIN_TELEMETRY]>,
    ) -> u8 {
        positions.fill(0.0);
        progress.fill(0.0);
        gains.fill(0.0);
        if let Some(pans) = pans.as_deref_mut() {
            pans.fill(0.0);
        }
        if let Some(pitches) = pitches.as_deref_mut() {
            pitches.fill(0.0);
        }
        let source_max = source_len.saturating_sub(1) as f32;
        let mut active_mask = 0_u8;
        let mut slot = 0_usize;
        for layer in &self.layers {
            if !layer.active || slot >= GRAIN_TELEMETRY {
                continue;
            }
            active_mask |= 1_u8 << slot;
            let position = reflected_position(layer.position, source_max);
            positions[slot] = (position / source_max.max(1.0)).clamp(0.0, 1.0);
            progress[slot] = (layer.age as f32 / layer.length.max(1) as f32).clamp(0.0, 1.0);
            gains[slot] = grain_window_shaped(progress[slot], 0.0, 0.0, 0.0, 0.0);
            if let Some(pans) = pans.as_deref_mut() {
                pans[slot] = layer.pan;
            }
            if let Some(pitches) = pitches.as_deref_mut() {
                pitches[slot] = layer.pitch;
            }
            slot += 1;
        }
        active_mask
    }

    #[cfg(test)]
    #[inline]
    pub fn render(
        &mut self,
        artifact: &GrainSourceArtifact,
        target_hz: f32,
        host_sample_rate: f32,
        note_seed: u64,
        frame_id: u64,
    ) -> f32 {
        let (left, right) = self.render_cloud(
            artifact,
            target_hz,
            host_sample_rate,
            note_seed,
            frame_id,
            artifact.controls,
            artifact.controls.position,
            artifact.controls.grain_spray,
        );
        (left + right) * 0.5
    }

    #[cfg(test)]
    #[inline]
    pub fn render_lane(
        &mut self,
        artifact: &GrainSourceArtifact,
        target_hz: f32,
        host_sample_rate: f32,
        note_seed: u64,
        frame_id: u64,
        _lane_index: usize,
    ) -> f32 {
        self.render(artifact, target_hz, host_sample_rate, note_seed, frame_id)
    }

    #[inline]
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn render_lane_with(
        &mut self,
        artifact: &GrainSourceArtifact,
        target_hz: f32,
        host_sample_rate: f32,
        note_seed: u64,
        frame_id: u64,
        _lane_index: usize,
        controls: ResynthControls,
        _render_voices: u8,
    ) -> f32 {
        let (left, right) = self.render_cloud(
            artifact,
            target_hz,
            host_sample_rate,
            note_seed,
            frame_id,
            controls,
            controls.position,
            controls.grain_spray,
        );
        (left + right) * 0.5
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn render_cloud(
        &mut self,
        artifact: &GrainSourceArtifact,
        target_hz: f32,
        host_sample_rate: f32,
        note_seed: u64,
        frame_id: u64,
        controls: ResynthControls,
        phase_position: f32,
        phase_random: f32,
    ) -> (f32, f32) {
        // Timeline and pitch stay independent. Keyboard pitch maps the detected
        // root; Tune crossfades to the worker-rendered, spectrally corrected
        // stereo source without changing grain position or duration.
        let pitch_ratio = artifact
            .root_hz
            .map_or(1.0, |root| target_hz.max(0.0) / root)
            .clamp(0.0, 1_024.0);
        let source_max = artifact.samples.len().saturating_sub(1) as f32;
        let new_frame = !self.cache_valid || self.cached_frame != frame_id;
        if new_frame && self.cache_valid {
            let mut active = 0_usize;
            while active < usize::from(self.active_count) {
                let index = usize::from(self.active_indices[active]);
                let layer = &mut self.layers[index];
                layer.age = layer.age.saturating_add(1);
                layer.position += layer.source_step * pitch_ratio;
                if layer.age >= layer.length {
                    layer.active = false;
                    self.active_count -= 1;
                    self.active_indices[active] =
                        self.active_indices[usize::from(self.active_count)];
                } else {
                    active += 1;
                }
            }
        }
        if new_frame {
            let requested_position =
                (controls.position + phase_position.clamp(0.0, 1.0)).rem_euclid(1.0);
            if self.cursor_valid {
                self.cursor += (requested_position - self.last_position) * source_max;
            } else {
                self.cursor = requested_position * source_max;
                self.cursor_valid = true;
            }
            self.last_position = requested_position;
            self.advance_cursor(artifact, host_sample_rate, controls.grain_direction());

            let duration_seconds = Self::grain_duration_seconds(controls);
            let rate_hz = density_plan(controls.grain_density, duration_seconds, GRAIN_LAYERS)
                .effective_rate_hz
                .min(host_sample_rate.max(1.0));
            let period = if rate_hz > 0.0 {
                host_sample_rate.max(1.0) / rate_hz
            } else {
                f32::INFINITY
            };
            self.spawn_countdown = self.spawn_countdown.min(period);
            if self.spawn_countdown <= 0.0 {
                self.spawn(
                    artifact,
                    host_sample_rate,
                    note_seed,
                    controls,
                    1.0 - (1.0 - controls.grain_spray) * (1.0 - phase_random.clamp(0.0, 1.0)),
                );
                let random = splitmix64(controls.seed ^ note_seed ^ self.event.rotate_left(19));
                let unit = (random as u32) as f32 / u32::MAX as f32;
                let jitter_octaves = (unit * 2.0 - 1.0) * controls.grain_timing;
                self.spawn_countdown += period * 2.0_f32.powf(jitter_octaves);
            }
            self.spawn_countdown -= 1.0;
            self.cached_frame = frame_id;
            self.cache_valid = true;
        }

        let mut left = 0.0_f32;
        let mut right = 0.0_f32;
        let mut window_sum = 0.0_f32;
        let mut window_energy = 0.0_f32;
        let filter_cutoff = controls.grain_filter_cutoff.clamp(0.0, 1.0);
        let cutoff_hz =
            20.0 + filter_cutoff.powi(4) * (host_sample_rate.max(100.0) * 0.45 - 20.0).max(0.0);
        let filter_coefficient = (TAU * cutoff_hz / host_sample_rate.max(1.0)).clamp(0.0, 1.0);
        for active in 0..usize::from(self.active_count) {
            let index = usize::from(self.active_indices[active]);
            let layer = &mut self.layers[index];
            let phase = layer.age as f32 / layer.length.max(1) as f32;
            let window = grain_window_shaped(
                phase,
                controls.grain_envelope,
                controls.grain_attack,
                controls.grain_hold,
                controls.grain_release,
            );
            let source_step = layer.source_step * pitch_ratio;
            let tune = if matches!(controls.pitch_mode, super::super::PitchMode::Classic) {
                0.0
            } else {
                let pitch_position = (layer.position / source_max.max(1.0)).clamp(0.0, 1.0);
                gate_spectral_tune(
                    effective_spectral_tune(controls),
                    artifact.pitch_frame_at(pitch_position),
                )
            };
            let (mid, side) = if tune <= f32::EPSILON {
                (
                    artifact.sample_filtered(layer.position, source_step.abs()),
                    artifact.sample_side_filtered(layer.position, source_step.abs()),
                )
            } else if tune >= 1.0 - f32::EPSILON {
                (
                    artifact.sample_tuned_filtered(layer.position, source_step.abs()),
                    artifact.sample_tuned_side_filtered(layer.position, source_step.abs()),
                )
            } else {
                let dry_mid = artifact.sample_filtered(layer.position, source_step.abs());
                let dry_side = artifact.sample_side_filtered(layer.position, source_step.abs());
                (
                    tune.mul_add(
                        artifact.sample_tuned_filtered(layer.position, source_step.abs()) - dry_mid,
                        dry_mid,
                    ),
                    tune.mul_add(
                        artifact.sample_tuned_side_filtered(layer.position, source_step.abs())
                            - dry_side,
                        dry_side,
                    ),
                )
            };
            let side = side * controls.grain_stereo;
            let (filtered_mid, filtered_side) = if filter_coefficient >= 1.0 {
                (mid, side)
            } else {
                self.filter_states[index] += filter_coefficient * (mid - self.filter_states[index]);
                self.side_filter_states[index] +=
                    filter_coefficient * (side - self.side_filter_states[index]);
                (self.filter_states[index], self.side_filter_states[index])
            };
            let gain = window * layer.gain;
            left += (filtered_mid + filtered_side) * gain * self.pan_left[index];
            right += (filtered_mid - filtered_side) * gain * self.pan_right[index];
            window_sum += window;
            window_energy += window * window;
        }
        let coherent = phase_random <= f32::EPSILON
            && controls.grain_blur <= f32::EPSILON
            && controls.grain_pitch_spread <= f32::EPSILON
            && controls.grain_reverse <= f32::EPSILON;
        let gain = if coherent {
            window_sum.max(1.0).recip()
        } else {
            grain_energy_gain(window_energy)
        };
        (left * gain, right * gain)
    }

    #[inline]
    fn grain_duration_seconds(controls: ResynthControls) -> f32 {
        0.005 * 200.0_f32.powf(controls.grain_size.clamp(0.0, 1.0))
    }

    fn spawn(
        &mut self,
        artifact: &GrainSourceArtifact,
        host_sample_rate: f32,
        note_seed: u64,
        controls: ResynthControls,
        phase_random: f32,
    ) {
        let layer_index = if usize::from(self.active_count) < GRAIN_LAYERS {
            let index = self
                .layers
                .iter()
                .position(|layer| !layer.active)
                .unwrap_or(0);
            self.active_indices[usize::from(self.active_count)] = index as u8;
            self.active_count += 1;
            index
        } else {
            self.active_indices[..usize::from(self.active_count)]
                .iter()
                .copied()
                .map(usize::from)
                .map(|index| (index, &self.layers[index]))
                .min_by(|(_, left), (_, right)| {
                    let left_level = left.gain
                        * grain_window_shaped(
                            left.age as f32 / left.length.max(1) as f32,
                            controls.grain_envelope,
                            controls.grain_attack,
                            controls.grain_hold,
                            controls.grain_release,
                        );
                    let right_level = right.gain
                        * grain_window_shaped(
                            right.age as f32 / right.length.max(1) as f32,
                            controls.grain_envelope,
                            controls.grain_attack,
                            controls.grain_hold,
                            controls.grain_release,
                        );
                    left_level
                        .total_cmp(&right_level)
                        .then_with(|| right.age.cmp(&left.age))
                })
                .map_or(0, |(index, _)| index)
        };
        let random = splitmix64(controls.seed ^ note_seed ^ self.event);
        self.event = self.event.wrapping_add(1);
        let source_max = artifact.samples.len().saturating_sub(1) as f32;
        let size_bits = splitmix64(random ^ 0x9e37_79b9_7f4a_7c15);
        let length_seconds = Self::grain_duration_seconds(controls);
        let length = ((length_seconds * host_sample_rate.max(1.0)).round() as u32).max(16);
        let start_unit = (random as u32) as f32 / u32::MAX as f32;
        let random_start = start_unit * source_max;
        let blur_unit =
            (splitmix64(size_bits ^ 0x94d0_49bb_1331_11eb) as u32) as f32 / u32::MAX as f32;
        let blur_span = artifact.root_hz.map_or(
            artifact.source_sample_rate * length_seconds * 0.25,
            |root| artifact.source_sample_rate / root.max(20.0),
        );
        let blur = (blur_unit * 2.0 - 1.0) * controls.grain_blur * blur_span;
        let start =
            (random_start - self.cursor).mul_add(phase_random.clamp(0.0, 1.0), self.cursor) + blur;
        let mut source_step = artifact.source_sample_rate / host_sample_rate.max(1.0);
        if matches!(controls.grain_direction(), GrainDirection::Backward)
            || matches!(controls.grain_direction(), GrainDirection::PingPong)
                && !self.cursor_forward
        {
            source_step = -source_step;
        }
        let reverse_unit =
            (splitmix64(size_bits ^ 0xd1b5_4a32_d192_ed03) as u32) as f32 / u32::MAX as f32;
        if reverse_unit < controls.grain_reverse {
            source_step = -source_step;
        }
        let pitch_bits = splitmix64(size_bits.rotate_left(17));
        let pitch_unit = (pitch_bits as u32) as f32 / u32::MAX as f32;
        let pitch = controls.grain_pitch + (pitch_unit * 2.0 - 1.0) * controls.grain_pitch_spread;
        if pitch != 0.0 {
            source_step *= 2.0_f32.powf(pitch / 12.0);
        }
        let level_bits = splitmix64(pitch_bits ^ 0xa24b_aed4_96e9_d039);
        let level_unit = (level_bits as u32) as f32 / u32::MAX as f32;
        let gain = (controls.grain_level
            * (1.0 + (level_unit * 2.0 - 1.0) * controls.grain_level_spread))
            .max(0.0);
        let pan_bits = splitmix64(level_bits ^ 0x2c1b_3c6e_c372_f0a3);
        let pan_unit = (pan_bits as u32) as f32 / u32::MAX as f32;
        let pan = (controls.grain_pan + (pan_unit * 2.0 - 1.0) * controls.grain_pan_spread)
            .clamp(-1.0, 1.0);
        self.filter_states[layer_index] = 0.0;
        self.side_filter_states[layer_index] = 0.0;
        self.pan_left[layer_index] = (1.0 - pan).sqrt();
        self.pan_right[layer_index] = (1.0 + pan).sqrt();
        let position = reflected_position(start, source_max);
        self.layers[layer_index] = GrainLayerState {
            position,
            age: 0,
            length,
            source_step,
            gain,
            pan,
            pitch,
            active: true,
        };
    }

    fn advance_cursor(
        &mut self,
        artifact: &GrainSourceArtifact,
        host_sample_rate: f32,
        direction: GrainDirection,
    ) {
        let maximum = artifact.samples.len().saturating_sub(1) as f32;
        let step = artifact.source_sample_rate / host_sample_rate.max(1.0);
        match direction {
            GrainDirection::Hold => return,
            GrainDirection::Forward => self.cursor += step,
            GrainDirection::Backward => self.cursor -= step,
            GrainDirection::PingPong => {
                self.cursor += if self.cursor_forward { step } else { -step };
                if self.cursor >= maximum || self.cursor <= 0.0 {
                    self.cursor_forward = !self.cursor_forward;
                }
            }
        }
        self.cursor = match direction {
            GrainDirection::PingPong => self.cursor.clamp(0.0, maximum),
            _ => self.cursor.rem_euclid(maximum.max(1.0)),
        };
    }
}

fn strongest_channel<'a>(mid: &'a [f32], side: &'a [f32]) -> &'a [f32] {
    let power = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
    };
    if !side.is_empty() && power(side) > power(mid) * 1.01 {
        side
    } else {
        mid
    }
}

fn build_pitch_map_with_cancel(
    samples: &[f32],
    sample_rate: u32,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    let hop = (sample_rate as f32 * PITCH_TRACK_PERIOD_SECONDS)
        .round()
        .max(1.0) as usize;
    let points = samples.len().div_ceil(hop).clamp(1, MAX_PITCH_TRACK_POINTS);
    let window = (sample_rate as f32 * 0.096)
        .clamp(
            sample_rate as f32 * MIN_PITCH_TRACK_WINDOW_SECONDS,
            sample_rate as f32 * 0.15,
        )
        .round() as usize;
    let mut pitches = Vec::with_capacity(points);
    let mut first_valid = None;
    let mut leading_invalid = 0;
    let mut previous = 0.0_f32;
    for point in 0..points {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let center = if points == 1 {
            samples.len() / 2
        } else {
            point * samples.len().saturating_sub(1) / (points - 1)
        };
        let window = window.min(samples.len());
        let start = center
            .saturating_sub(window / 2)
            .min(samples.len().saturating_sub(window));
        let end = start + window;
        let (pitch_hz, confidence) = super::super::estimate_root_window_with_cancel(
            &samples[start..end],
            sample_rate,
            should_cancel,
        )
        .map_err(|_| ArtifactBuildError::Cancelled)?;
        if pitch_hz > 0.0 && confidence >= 0.2 {
            previous = pitch_hz;
            first_valid.get_or_insert(previous);
        } else if first_valid.is_none() {
            leading_invalid += 1;
        }
        pitches.push(previous);
    }
    if let Some(first) = first_valid {
        pitches[..leading_invalid].fill(first);
    }
    Ok(pitches)
}

#[inline]
fn pitch_at(pitches: &[f32], position: f32, fallback: f32) -> f32 {
    let index =
        (position.clamp(0.0, 1.0) * pitches.len().saturating_sub(1) as f32).round() as usize;
    pitches
        .get(index)
        .copied()
        .filter(|pitch| *pitch > 0.0)
        .unwrap_or(fallback)
}

#[inline]
pub(super) fn grain_window_shaped(
    phase: f32,
    shape: f32,
    attack: f32,
    hold: f32,
    release: f32,
) -> f32 {
    let phase = phase.clamp(0.0, 1.0);
    let stages = attack + hold + release;
    if stages > 1.0e-4 {
        let scale = stages.max(1.0e-4);
        let attack = attack / scale;
        let hold = hold / scale;
        if phase < attack {
            let t = phase / attack.max(1.0e-4);
            return t * t * (3.0 - 2.0 * t);
        }
        if phase < attack + hold {
            return 1.0;
        }
        let t = (phase - attack - hold) / (1.0 - attack - hold).max(1.0e-4);
        return 1.0 - t * t * (3.0 - 2.0 * t);
    }
    let product = phase * (1.0 - phase);
    let hann = 16.0 * product * product;
    if shape <= 0.0 {
        return hann;
    }
    let fade = 0.08 * (1.0 - shape) + 0.012;
    let rect = if phase < fade {
        phase / fade
    } else if phase > 1.0 - fade {
        (1.0 - phase) / fade
    } else {
        1.0
    };
    hann + (rect - hann) * shape.clamp(0.0, 1.0)
}

#[inline]
pub(super) fn grain_energy_gain(window_energy: f32) -> f32 {
    window_energy.max(1.0).sqrt().recip()
}

#[cfg(test)]
mod mode_tests {
    use super::*;
    use crate::oscillators::resynth::targeting::{PitchMode, TargetSet};

    #[test]
    fn classic_selects_dry_source_while_spectral_modes_honor_tune() {
        let mut controls = ResynthControls::default();
        controls.grain_tune = 1.0;
        assert_eq!(effective_spectral_tune(controls), 0.0);
        controls.pitch_mode = PitchMode::Spectral;
        assert_eq!(effective_spectral_tune(controls), 1.0);
        controls.pitch_mode = PitchMode::Target(TargetSet::PlayedNote);
        controls.grain_tune = 0.35;
        assert_eq!(effective_spectral_tune(controls), 0.35);
        let voiced = super::super::super::analysis::PitchFrame::from_candidates(
            0,
            1.0,
            0.0,
            &[super::super::super::analysis::PitchCandidate::new(
                60.0, 1.0, 0.8,
            )],
        );
        assert!((gate_spectral_tune(0.35, voiced) - 0.28).abs() < 1.0e-6);
        let onset = super::super::super::analysis::PitchFrame {
            onset: 1.0,
            ..voiced
        };
        assert_eq!(gate_spectral_tune(0.35, onset), 0.0);
    }
}
