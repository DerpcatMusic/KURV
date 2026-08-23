use super::shared::*;

#[derive(Clone)]
pub struct SampleLoopArtifact {
    pub source_sample_rate: f32,
    pub root_hz: f32,
    pub(crate) samples: Box<[f32]>,
    periodic_integral: Box<[f64]>,
    pub(super) periodic_mips: Box<[PeriodicMipLevel]>,
    pub(crate) source_start_frames: usize,
    pub(crate) source_span_frames: usize,
    pub(crate) source_total_frames: usize,
    pub(crate) crossfade_frames: usize,
}

impl SampleLoopArtifact {
    pub fn compile(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: Option<f32>,
        position: f32,
    ) -> Result<Self, ArtifactBuildError> {
        Self::compile_with_cancel(source, source_sample_rate, root_hz, position, &|| false)
    }

    pub(crate) fn compile_with_cancel(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: Option<f32>,
        position: f32,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let root_hz = root_hz.ok_or(ArtifactBuildError::RootRequired)?;
        validate_source(source)?;
        let source_rate = source_sample_rate as f32;
        let period = (source_rate / root_hz).clamp(4.0, MAX_SAMPLE_PERIOD_FRAMES);
        let desired_overlap = ((period * 2.0).round() as usize).clamp(32, 2_048);
        // Reserve the desired overlap when the source can contain it beside a
        // loop. For a short/low-root source, split the region evenly so both
        // the loop and its eventual effective overlap remain representable.
        let available = if source.len() >= desired_overlap * 2 {
            source.len() - desired_overlap
        } else {
            source.len() / 2
        };
        if available < MIN_SAMPLE_LOOP_FRAMES {
            return Err(ArtifactBuildError::Empty);
        }
        let maximum_loop = available.min(SAMPLE_MAX_FRAMES);
        let mut loop_frames = ((maximum_loop as f32) * 0.75)
            .round()
            .clamp(MIN_SAMPLE_LOOP_FRAMES as f32, maximum_loop as f32)
            as usize;
        let periods = (loop_frames as f32 / period).floor().max(1.0);
        loop_frames =
            ((period * periods).round() as usize).clamp(MIN_SAMPLE_LOOP_FRAMES, available);
        let effective_overlap = desired_overlap.min(loop_frames);
        let required = loop_frames + effective_overlap;
        let start_limit = source.len().saturating_sub(required);
        let nominal = (position.clamp(0.0, 1.0) * start_limit as f32).round() as usize;
        let start = best_periodic_start(source, nominal, start_limit, loop_frames, period);

        // Circular overlap: output starts after the incoming overlap. The end
        // crossfades outgoing material into the immediately preceding source
        // segment, so the final->first wrap remains time-adjacent.
        let mut output = vec![0.0_f32; loop_frames];
        let body = loop_frames - effective_overlap;
        for (index, sample) in output[..body].iter_mut().enumerate() {
            *sample = source[start + effective_overlap + index];
        }
        for index in 0..effective_overlap {
            let phase = (index as f32 + 0.5) / effective_overlap as f32;
            let incoming_gain = (phase * std::f32::consts::FRAC_PI_2).sin();
            let outgoing_gain = (phase * std::f32::consts::FRAC_PI_2).cos();
            let outgoing = source[(start + loop_frames + index).min(source.len() - 1)];
            let incoming = source[(start + index).min(source.len() - 1)];
            output[body + index] = (outgoing * outgoing_gain + incoming * incoming_gain)
                * std::f32::consts::FRAC_1_SQRT_2;
        }
        remove_dc_and_peak_normalize(&mut output);
        let correction_frames = output.len().min(128);
        if correction_frames >= 2 {
            let delta = output[0] - output[output.len() - 1];
            let start = output.len() - correction_frames;
            for (index, sample) in output[start..].iter_mut().enumerate() {
                let x = index as f32 / (correction_frames - 1) as f32;
                let smooth = x * x * (3.0 - 2.0 * x);
                *sample += delta * smooth;
            }
        }
        remove_dc_and_peak_normalize(&mut output);
        let periodic_integral = periodic_integral_prefix_with_cancel(&output, should_cancel)?;
        let periodic_mips = build_periodic_mips_with_cancel(&output, should_cancel)?;
        Ok(Self {
            source_sample_rate: source_rate,
            root_hz,
            samples: output.into_boxed_slice(),
            periodic_integral,
            periodic_mips,
            source_start_frames: start,
            source_span_frames: required,
            source_total_frames: source.len(),
            crossfade_frames: effective_overlap,
        })
    }

    /// Restores a legacy persisted payload without inventing unavailable
    /// source-region or seam coordinates. Four zero receipt fields explicitly
    /// mean that source-loop visual metadata is unavailable.
    #[must_use]
    pub(crate) fn from_persisted(
        source_sample_rate: f32,
        root_hz: f32,
        samples: Box<[f32]>,
    ) -> Self {
        let periodic_integral = periodic_integral_prefix(&samples);
        let periodic_mips = build_periodic_mips(&samples);
        Self {
            source_sample_rate,
            root_hz,
            samples,
            periodic_integral,
            periodic_mips,
            source_start_frames: 0,
            source_span_frames: 0,
            source_total_frames: 0,
            crossfade_frames: 0,
        }
    }

    /// Restores a versioned payload only when its immutable compiler receipt
    /// describes this exact loop and a bounded region of its source.
    #[must_use]
    pub(crate) fn from_persisted_with_receipt(
        source_sample_rate: f32,
        root_hz: f32,
        samples: Box<[f32]>,
        source_start_frames: usize,
        source_span_frames: usize,
        source_total_frames: usize,
        crossfade_frames: usize,
    ) -> Option<Self> {
        let source_end = source_start_frames.checked_add(source_span_frames)?;
        let loop_frames = source_span_frames.checked_sub(crossfade_frames)?;
        if source_total_frames == 0
            || source_span_frames == 0
            || source_end > source_total_frames
            || crossfade_frames > source_span_frames
            || loop_frames != samples.len()
        {
            return None;
        }
        let periodic_integral = periodic_integral_prefix(&samples);
        let periodic_mips = build_periodic_mips(&samples);
        Some(Self {
            source_sample_rate,
            root_hz,
            samples,
            periodic_integral,
            periodic_mips,
            source_start_frames,
            source_span_frames,
            source_total_frames,
            crossfade_frames,
        })
    }

    #[must_use]
    pub fn frames(&self) -> usize {
        self.samples.len()
    }

    /// First source frame retained by the compiled circular-loop region.
    #[must_use]
    pub fn source_start_frames(&self) -> usize {
        self.source_start_frames
    }

    /// Source-frame extent retained by the loop, including its overlap seam.
    #[must_use]
    pub fn source_span_frames(&self) -> usize {
        self.source_span_frames
    }

    /// Total frame count of the bounded source used by this compilation.
    #[must_use]
    pub fn source_total_frames(&self) -> usize {
        self.source_total_frames
    }

    /// Number of source frames participating in the circular crossfade.
    #[must_use]
    pub fn crossfade_frames(&self) -> usize {
        self.crossfade_frames
    }

    #[cfg(test)]
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    #[cfg(test)]
    #[inline]
    #[must_use]
    pub fn eval(&self, phase: f32) -> f32 {
        periodic_cubic(&self.samples, phase)
    }

    #[inline]
    #[must_use]
    pub fn eval_bandlimited(&self, phase: f32, source_frames_per_output: f32) -> f32 {
        periodic_mip_sample(
            &self.samples,
            &self.periodic_integral,
            &self.periodic_mips,
            phase,
            source_frames_per_output,
        )
    }

    #[inline]
    #[must_use]
    pub fn phase_increment(&self, target_hz: f32, host_sample_rate: f32) -> f32 {
        (target_hz.max(0.0) / self.root_hz) * (self.source_sample_rate / host_sample_rate.max(1.0))
            / self.samples.len().max(1) as f32
    }
}

#[derive(Clone)]
pub struct SourceAuditionArtifact {
    pub source_sample_rate: f32,
    pub(crate) samples: Box<[f32]>,
    integral_blocks: Box<[f64]>,
    one_shot_mips: Box<[OneShotMipLevel]>,
}

impl SourceAuditionArtifact {
    pub fn compile(source: &[f32], source_sample_rate: u32) -> Result<Self, ArtifactBuildError> {
        Self::compile_with_cancel(source, source_sample_rate, &|| false)
    }

    pub(crate) fn compile_with_cancel(
        source: &[f32],
        source_sample_rate: u32,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        validate_source(source)?;
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let mut samples = source.to_vec();
        remove_dc_and_peak_normalize(&mut samples);
        let integral_blocks = one_shot_integral_blocks_with_cancel(&samples, should_cancel)?;
        let one_shot_mips = build_one_shot_mips_with_cancel(&samples, should_cancel)?;
        Ok(Self {
            source_sample_rate: source_sample_rate as f32,
            samples: samples.into_boxed_slice(),
            integral_blocks,
            one_shot_mips,
        })
    }

    #[must_use]
    pub fn silence() -> Self {
        Self {
            source_sample_rate: 48_000.0,
            samples: vec![0.0].into_boxed_slice(),
            integral_blocks: vec![0.0, 0.0].into_boxed_slice(),
            one_shot_mips: Vec::new().into_boxed_slice(),
        }
    }

    #[inline]
    #[must_use]
    pub fn sample_one_shot(&self, position: f64) -> f32 {
        if !position.is_finite() || position < 0.0 || position >= self.samples.len() as f64 {
            return 0.0;
        }
        let first = position.floor() as usize;
        let second = (first + 1).min(self.samples.len() - 1);
        let mix = (position - first as f64) as f32;
        (self.samples[second] - self.samples[first]).mul_add(mix, self.samples[first])
    }

    #[inline]
    #[must_use]
    pub fn sample_one_shot_filtered(&self, position: f64, source_step: f32) -> f32 {
        one_shot_mip_sample(
            &self.samples,
            &self.integral_blocks,
            &self.one_shot_mips,
            position,
            source_step,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SourceAuditionState {
    pub position: f64,
    generation: u64,
    cached_frame: u64,
    cached_output: f32,
    cache_valid: bool,
}

impl Default for SourceAuditionState {
    fn default() -> Self {
        Self {
            position: 0.0,
            generation: 0,
            cached_frame: 0,
            cached_output: 0.0,
            cache_valid: false,
        }
    }
}

impl SourceAuditionState {
    pub fn render(
        &mut self,
        artifact: &SourceAuditionArtifact,
        generation: u64,
        host_sample_rate: f32,
        frame: u64,
    ) -> f32 {
        if self.cache_valid && self.cached_frame == frame && self.generation == generation {
            return self.cached_output;
        }
        if self.generation != generation {
            self.position = 0.0;
            self.generation = generation;
        }
        let source_step = artifact.source_sample_rate / host_sample_rate.max(1.0);
        let output = artifact.sample_one_shot_filtered(self.position, source_step);
        self.position += f64::from(source_step);
        self.cached_frame = frame;
        self.cached_output = output;
        self.cache_valid = true;
        output
    }
}
