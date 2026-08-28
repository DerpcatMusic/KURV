//! Steady oscillator-bank block rendering kernels.

use super::*;

impl VaVoice {
    pub(super) fn steady_voice_amplitude(&self, settings: VoiceSettings, sustain: f32) -> f32 {
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        sustain.clamp(0.0, 1.0) * velocity_gain * pressure_gain
    }

    pub(in crate::voices) fn render_structural_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        oscillator_bank: &ActiveOscillatorRenderSet,
        controls: &[StructuralOscillatorFrameControl],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(oscillator_bank.active());
        debug_assert!(!oscillator_bank.transitioning());
        debug_assert_eq!(controls.len(), SAMPLES);
        std::array::from_fn(|frame| {
            self.advance_envelope(sample_rate, false);
            self.advance_glide();
            self.render_oscillator_bank(oscillator_bank, settings, sample_rate, &controls[frame])
        })
    }

    pub(in crate::voices) fn render_voice_structural_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        oscillator_bank: &ActiveOscillatorRenderSet,
        controls: &[StructuralOscillatorFrameControl],
        program: &VoiceLfoProgram,
        routes: &VoiceStructuralRouteFrame,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(oscillator_bank.active());
        debug_assert!(!oscillator_bank.transitioning());
        debug_assert_eq!(controls.len(), SAMPLES);
        std::array::from_fn(|frame| {
            if !self.active() {
                return (0.0, 0.0);
            }
            let mut modulation = crate::StructuralModulationFrame::default();
            routes.evaluate(self.modulation.next(program), &mut modulation);
            let control = merge_voice_structural_block_control(
                &controls[frame],
                &modulation,
                oscillator_bank,
            );
            self.advance_envelope(sample_rate, false);
            self.advance_glide();
            self.render_oscillator_bank(oscillator_bank, settings, sample_rate, &control)
        })
    }

    pub(super) fn settled_oscillator_bank_voice_eligible(
        &self,
        active: &ActiveOscillatorRenderSet,
    ) -> bool {
        if active.entries().iter().any(|entry| {
            entry.current.engine == OscillatorEngineKind::Resynth
                || entry.target.engine == OscillatorEngineKind::Resynth
        }) {
            return false;
        }
        if !self.held
            || self.is_gliding()
            || self.amplitude_level() <= f32::EPSILON
            || self.envelope.sustain <= f32::EPSILON
            || (self.group_envelope_count != 0 && !self.group_envelopes[0].is_sustaining())
        {
            return false;
        }
        true
    }

    pub(in crate::voices) fn settled_grouped_bank_voice_eligible(
        &self,
        active: &ActiveOscillatorRenderSet,
    ) -> bool {
        if !self.settled_oscillator_bank_voice_eligible(active) {
            return false;
        }
        let count = usize::from(self.group_envelope_count);
        count == 0
            || (0..count).all(|group| {
                self.group_active_mask & (1 << group) == 0
                    || self.group_envelopes[group].is_sustaining()
            })
    }

    pub(super) fn render_settled_oscillator_bank_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
    ) -> [(f32, f32); SAMPLES] {
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let entries = active.entries();
        let mut offset = 0;
        while offset < entries.len() {
            if entries[offset].current.render_voices == 1 {
                if let Some((count, shape, pulse_width)) =
                    Self::structural_single_lane_run(&entries[offset..], timbre)
                {
                    self.accumulate_structural_single_lane_bank_block(
                        &entries[offset..offset + count],
                        settings,
                        base_step,
                        shape,
                        pulse_width,
                        &mut left,
                        &mut right,
                    );
                    offset += count;
                    continue;
                }
            }
            let entry = &entries[offset];
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            let shape = (oscillator.shape + timbre).clamp(0.0, 3.0);
            if oscillator.render_voices == 8
                && oscillator.custom_mix <= f32::EPSILON
                && !oscillator.phase_warp.active()
                && (shape - 2.0).abs() <= f32::EPSILON
            {
                self.accumulate_structural_saw8_block(
                    slot,
                    oscillator,
                    settings,
                    sample_rate,
                    base_step,
                    &mut left,
                    &mut right,
                );
            } else {
                self.accumulate_structural_oscillator_block(
                    slot,
                    oscillator,
                    settings,
                    sample_rate,
                    base_step,
                    shape,
                    &mut left,
                    &mut right,
                );
            }
            offset += 1;
        }
        if self.stage == EnvelopeStage::Sustain {
            if self.group_envelope_count == 0 {
                self.envelope_level = self.envelope.sustain.clamp(0.0, 1.0);
            }
            let amplitude = self.amplitude_level() * velocity_gain * pressure_gain;
            return std::array::from_fn(|frame| {
                (
                    left[frame].reduce_add() * amplitude,
                    right[frame].reduce_add() * amplitude,
                )
            });
        }
        std::array::from_fn(|frame| {
            self.advance_envelope(sample_rate, false);
            let amplitude = self.amplitude_level() * velocity_gain * pressure_gain;
            (
                left[frame].reduce_add() * amplitude,
                right[frame].reduce_add() * amplitude,
            )
        })
    }

    pub(in crate::voices) fn render_settled_oscillator_bank_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.settled_grouped_bank_voice_eligible(active));
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        if self.group_envelope_count == 0 {
            self.envelope_level = self.envelope.sustain.clamp(0.0, 1.0);
        }
        let voice_amp = self.envelope_level * velocity_gain * pressure_gain;
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let entries = active.entries();
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for group in 0..group_count {
            let mut left = [f32x8::ZERO; SAMPLES];
            let mut right = [f32x8::ZERO; SAMPLES];
            let mut any = false;
            for entry in entries {
                let slot = usize::from(entry.slot);
                if oscillator_group(oscillator_groups, group_count, slot) != group {
                    continue;
                }
                any = true;
                let oscillator = &entry.current;
                let shape = (oscillator.shape + timbre).clamp(0.0, 3.0);
                if oscillator.render_voices == 8
                    && oscillator.custom_mix <= f32::EPSILON
                    && !oscillator.phase_warp.active()
                    && (shape - 2.0).abs() <= f32::EPSILON
                {
                    self.accumulate_structural_saw8_block(
                        slot,
                        oscillator,
                        settings,
                        sample_rate,
                        base_step,
                        &mut left,
                        &mut right,
                    );
                } else {
                    self.accumulate_structural_oscillator_block(
                        slot,
                        oscillator,
                        settings,
                        sample_rate,
                        base_step,
                        shape,
                        &mut left,
                        &mut right,
                    );
                }
            }
            if !any {
                continue;
            }
            let gain = voice_amp
                * if self.group_envelope_count == 0 {
                    1.0
                } else {
                    self.group_envelopes[group].level
                };
            for frame in 0..SAMPLES {
                output[group][frame] = (
                    left[frame].reduce_add() * gain,
                    right[frame].reduce_add() * gain,
                );
            }
        }
        output
    }

    pub(in crate::voices) fn render_phase_mod_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        groups: &[GeneratorRtGroup],
        group_count: usize,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.settled_grouped_bank_voice_eligible(active));
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        if self.group_envelope_count == 0 {
            self.envelope_level = self.envelope.sustain.clamp(0.0, 1.0);
        }
        let voice_amp = self.envelope_level * velocity_gain * pressure_gain;
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for (group_index, group) in groups.iter().take(group_count).enumerate() {
            let mut oscillator_outputs = [[0.0_f32; SAMPLES]; MAX_OSCILLATORS];
            let mut rendered = 0_u32;
            for module in group.modules() {
                let GeneratorRtModule::Oscillator(slot) = *module else {
                    debug_assert!(false, "phase-mod block rendering excludes filters");
                    continue;
                };
                let slot = slot.index();
                if active.mask & (1 << slot) == 0 {
                    continue;
                }
                let oscillator = &active.entry(slot).current;
                let shape = (oscillator.shape + timbre).clamp(0.0, 3.0);
                if oscillator.engine == OscillatorEngineKind::Va
                    && oscillator.render_voices == 1
                    && oscillator.custom_mix <= f32::EPSILON
                    && !oscillator.phase_warp.active()
                    && !oscillator.jitter_active()
                {
                    self.oscillator_bank.jitter_ratios[slot][0] = 1.0;
                    self.oscillator_bank.jitter_steps[slot][0] = 0.0;
                    self.oscillator_bank.jitter_remaining[slot] = 0;
                    let phase_delta = shortest_phase_delta(
                        self.oscillator_bank.applied_phase_positions[slot],
                        oscillator.phase_position,
                    );
                    if phase_delta != 0.0 {
                        self.oscillator_bank.oscillators[slot][0].offset_phase(phase_delta);
                        self.oscillator_bank.applied_phase_positions[slot] =
                            oscillator.phase_position;
                    }
                    let phase_step = (base_step * oscillator.pitch_ratio).min(0.45);
                    for frame in 0..SAMPLES {
                        let phase_mod = oscillator
                            .phase_mod_source
                            .checked_sub(1)
                            .filter(|source| rendered & (1 << *source) != 0)
                            .map_or(0.0, |source| {
                                oscillator_outputs[usize::from(source)][frame]
                                    * oscillator.phase_mod_amount
                            })
                            .clamp(-1.0, 1.0);
                        let lane = &mut self.oscillator_bank.oscillators[slot][0];
                        if phase_mod != 0.0 {
                            lane.offset_phase(phase_mod);
                        }
                        let sample = lane.generate_shape_step(
                            shape,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        if phase_mod != 0.0 {
                            lane.offset_phase(-phase_mod);
                        }
                        let left = sample * oscillator.left_gain;
                        let right = sample * oscillator.right_gain;
                        oscillator_outputs[slot][frame] = (left + right) * 0.5;
                        output[group_index][frame].0 += left;
                        output[group_index][frame].1 += right;
                    }
                    rendered |= 1 << slot;
                    continue;
                }
                for frame in 0..SAMPLES {
                    let phase_mod = oscillator
                        .phase_mod_source
                        .checked_sub(1)
                        .filter(|source| rendered & (1 << *source) != 0)
                        .map_or(0.0, |source| {
                            oscillator_outputs[usize::from(source)][frame]
                                * oscillator.phase_mod_amount
                        })
                        .clamp(-1.0, 1.0);
                    let mut left = 0.0;
                    let mut right = 0.0;
                    self.accumulate_structural_oscillator(
                        slot,
                        slot,
                        oscillator,
                        None,
                        settings,
                        sample_rate,
                        base_step,
                        shape,
                        phase_mod,
                        &mut left,
                        &mut right,
                    );
                    oscillator_outputs[slot][frame] = (left + right) * 0.5;
                    output[group_index][frame].0 += left;
                    output[group_index][frame].1 += right;
                }
                rendered |= 1 << slot;
            }
            let gain = voice_amp
                * if self.group_envelope_count == 0 {
                    1.0
                } else {
                    self.group_envelopes[group_index].level
                };
            for sample in &mut output[group_index] {
                sample.0 *= gain;
                sample.1 *= gain;
            }
        }
        output
    }

    pub(super) fn structural_single_lane_run(
        entries: &[ActiveOscillatorRenderEntry],
        timbre: f32,
    ) -> Option<(usize, f32, f32)> {
        if entries.len() < 3 {
            return None;
        }
        let first = &entries[0].current;
        let shape = (first.shape + timbre).clamp(0.0, 3.0);
        let pulse_width = first.pulse_width;
        let count = entries
            .iter()
            .take_while(|entry| {
                let oscillator = &entry.current;
                oscillator.engine == OscillatorEngineKind::Va
                    && oscillator.render_voices == 1
                    && oscillator.custom_mix <= f32::EPSILON
                    && !oscillator.phase_warp.active()
                    && (oscillator.shape + timbre).clamp(0.0, 3.0).to_bits() == shape.to_bits()
                    && oscillator.pulse_width.to_bits() == pulse_width.to_bits()
            })
            .count();
        (count >= 3).then_some((count, shape, pulse_width))
    }

    pub(super) fn structural_single_lane_pair(
        entries: &[ActiveOscillatorRenderEntry],
        timbre: f32,
    ) -> Option<(f32, f32)> {
        if entries.len() < 2 {
            return None;
        }
        let first = &entries[0].current;
        let shape = (first.shape + timbre).clamp(0.0, 3.0);
        let pulse_width = first.pulse_width;
        entries[..2]
            .iter()
            .all(|entry| {
                let oscillator = &entry.current;
                oscillator.engine == OscillatorEngineKind::Va
                    && oscillator.render_voices == 1
                    && oscillator.custom_mix <= f32::EPSILON
                    && !oscillator.phase_warp.active()
                    && (oscillator.shape + timbre).clamp(0.0, 3.0).to_bits() == shape.to_bits()
                    && oscillator.pulse_width.to_bits() == pulse_width.to_bits()
            })
            .then_some((shape, pulse_width))
    }

    #[inline(never)]
    pub(super) fn structural_small_unison_pair(
        entries: &[ActiveOscillatorRenderEntry],
        timbre: f32,
    ) -> Option<(usize, f32, f32)> {
        if entries.len() != 2 {
            return None;
        }
        let first = &entries[0].current;
        let shape = (first.shape + timbre).clamp(0.0, 3.0);
        let pulse_width = first.pulse_width;
        let mut lanes = 0;
        for entry in entries {
            let oscillator = &entry.current;
            let voices = usize::from(oscillator.render_voices);
            if oscillator.engine != OscillatorEngineKind::Va
                || !(2..=4).contains(&voices)
                || oscillator.custom_mix > f32::EPSILON
                || oscillator.phase_warp.active()
                || (oscillator.shape + timbre).clamp(0.0, 3.0).to_bits() != shape.to_bits()
                || oscillator.pulse_width.to_bits() != pulse_width.to_bits()
            {
                return None;
            }
            lanes += voices;
        }
        Some((lanes, shape, pulse_width))
    }

    #[inline(never)]
    pub(super) fn render_settled_two_oscillator_bank_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        entries: &[ActiveOscillatorRenderEntry],
        shape: f32,
        pulse_width: f32,
    ) -> [(f32, f32); SAMPLES] {
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let mut amplitude = [0.0; SAMPLES];
        for value in &mut amplitude {
            self.advance_envelope(sample_rate, false);
            *value = self.amplitude_level() * velocity_gain * pressure_gain;
        }

        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        self.accumulate_structural_single_lane_pack4(
            entries,
            settings,
            base_step,
            shape,
            pulse_width,
            &mut left,
            &mut right,
        );
        std::array::from_fn(|frame| {
            (
                left[frame].reduce_add() * amplitude[frame],
                right[frame].reduce_add() * amplitude[frame],
            )
        })
    }

    #[inline(never)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the exact-two small-unison renderer keeps its fixed render context allocation-free"
    )]
    pub(super) fn render_settled_two_small_unison_oscillator_bank_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        entries: &[ActiveOscillatorRenderEntry],
        lanes: usize,
        shape: f32,
        pulse_width: f32,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert_eq!(entries.len(), 2);
        debug_assert!((4..=8).contains(&lanes));
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let mut amplitude = [0.0; SAMPLES];
        for value in &mut amplitude {
            self.advance_envelope(sample_rate, false);
            *value = self.amplitude_level() * velocity_gain * pressure_gain;
        }

        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let mut packed = [VaOscillator::default(); 8];
        let mut phase_steps = [0.0; 8];
        let mut left_gains = [0.0; 8];
        let mut right_gains = [0.0; 8];
        let mut packed_lane = 0;
        for entry in entries {
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            self.advance_settled_structural_jitter_block::<SAMPLES>(slot, oscillator, sample_rate);
            for lane in 0..usize::from(oscillator.render_voices) {
                packed[packed_lane] = self.oscillator_bank.oscillators[slot][lane];
                phase_steps[packed_lane] =
                    (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[lane])
                        .min(0.45);
                left_gains[packed_lane] = oscillator.left_gain * oscillator.lane_left_gains[lane];
                right_gains[packed_lane] =
                    oscillator.right_gain * oscillator.lane_right_gains[lane];
                packed_lane += 1;
            }
        }
        debug_assert_eq!(packed_lane, lanes);

        if lanes == 4 {
            let phase_step = f32x4::from(std::array::from_fn(|lane| phase_steps[lane]));
            let left_gain = f32x4::from(std::array::from_fn(|lane| left_gains[lane]));
            let right_gain = f32x4::from(std::array::from_fn(|lane| right_gains[lane]));
            if (shape - 2.0).abs() <= f32::EPSILON {
                accumulate_saw4_block_constant(
                    &mut packed[..4],
                    phase_step,
                    left_gain,
                    right_gain,
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
            } else {
                accumulate_shape4_block_constant(
                    &mut packed[..4],
                    phase_step,
                    left_gain,
                    right_gain,
                    &mut left,
                    &mut right,
                    shape,
                    pulse_width,
                    settings.antialiasing,
                );
            }
        } else if (shape - 2.0).abs() <= f32::EPSILON {
            accumulate_saw8_block_constant(
                &mut packed,
                f32x8::from(phase_steps),
                f32x8::from(left_gains),
                f32x8::from(right_gains),
                &mut left,
                &mut right,
                settings.antialiasing,
            );
        } else {
            accumulate_shape8_block_constant(
                &mut packed,
                f32x8::from(phase_steps),
                f32x8::from(left_gains),
                f32x8::from(right_gains),
                &mut left,
                &mut right,
                shape,
                pulse_width,
                settings.antialiasing,
            );
        }

        packed_lane = 0;
        for entry in entries {
            let slot = usize::from(entry.slot);
            for lane in 0..usize::from(entry.current.render_voices) {
                self.oscillator_bank.oscillators[slot][lane] = packed[packed_lane];
                packed_lane += 1;
            }
        }
        std::array::from_fn(|frame| {
            (
                left[frame].reduce_add() * amplitude[frame],
                right[frame].reduce_add() * amplitude[frame],
            )
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the structural instance pack keeps its fixed render context allocation-free"
    )]
    pub(super) fn accumulate_structural_single_lane_bank_block<const SAMPLES: usize>(
        &mut self,
        entries: &[ActiveOscillatorRenderEntry],
        settings: VoiceSettings,
        base_step: f32,
        shape: f32,
        pulse_width: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        let mut offset = 0;
        while entries.len() - offset >= 8 {
            self.accumulate_structural_single_lane_pack8(
                &entries[offset..offset + 8],
                settings,
                base_step,
                shape,
                pulse_width,
                left,
                right,
            );
            offset += 8;
        }
        let remaining = entries.len() - offset;
        if remaining >= 5 {
            self.accumulate_structural_single_lane_pack8(
                &entries[offset..],
                settings,
                base_step,
                shape,
                pulse_width,
                left,
                right,
            );
            return;
        }
        if remaining >= 3 {
            self.accumulate_structural_single_lane_pack4(
                &entries[offset..],
                settings,
                base_step,
                shape,
                pulse_width,
                left,
                right,
            );
            return;
        }
        for entry in &entries[offset..] {
            let slot = usize::from(entry.slot);
            self.accumulate_structural_oscillator_block(
                slot,
                &entry.current,
                settings,
                self.sample_rate,
                base_step,
                shape,
                left,
                right,
            );
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the eight-wide instance pack keeps its fixed render context allocation-free"
    )]
    pub(super) fn accumulate_structural_single_lane_pack8<const SAMPLES: usize>(
        &mut self,
        entries: &[ActiveOscillatorRenderEntry],
        settings: VoiceSettings,
        base_step: f32,
        shape: f32,
        pulse_width: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        debug_assert!((5..=8).contains(&entries.len()));
        let mut packed = [VaOscillator::default(); 8];
        let mut phase_steps = [0.0; 8];
        let mut left_gains = [0.0; 8];
        let mut right_gains = [0.0; 8];
        for (lane, entry) in entries.iter().enumerate() {
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            packed[lane] = self.oscillator_bank.oscillators[slot][0];
            phase_steps[lane] = (base_step * oscillator.pitch_ratio).min(0.45);
            left_gains[lane] = oscillator.left_gain;
            right_gains[lane] = oscillator.right_gain;
            self.oscillator_bank.jitter_remaining[slot] = 0;
        }
        if (shape - 2.0).abs() <= f32::EPSILON {
            accumulate_saw8_block_constant(
                &mut packed,
                f32x8::from(phase_steps),
                f32x8::from(left_gains),
                f32x8::from(right_gains),
                left,
                right,
                settings.antialiasing,
            );
        } else {
            accumulate_shape8_block_constant(
                &mut packed,
                f32x8::from(phase_steps),
                f32x8::from(left_gains),
                f32x8::from(right_gains),
                left,
                right,
                shape,
                pulse_width,
                settings.antialiasing,
            );
        }
        for (lane, entry) in entries.iter().enumerate() {
            self.oscillator_bank.oscillators[usize::from(entry.slot)][0] = packed[lane];
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the four-wide instance pack keeps its fixed render context allocation-free"
    )]
    #[inline(always)]
    pub(super) fn accumulate_structural_single_lane_pack4<const SAMPLES: usize>(
        &mut self,
        entries: &[ActiveOscillatorRenderEntry],
        settings: VoiceSettings,
        base_step: f32,
        shape: f32,
        pulse_width: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        debug_assert!((2..=4).contains(&entries.len()));
        let mut packed = [VaOscillator::default(); 4];
        let mut phase_steps = [0.0; 4];
        let mut left_gains = [0.0; 4];
        let mut right_gains = [0.0; 4];
        for (lane, entry) in entries.iter().enumerate() {
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            packed[lane] = self.oscillator_bank.oscillators[slot][0];
            phase_steps[lane] = (base_step * oscillator.pitch_ratio).min(0.45);
            left_gains[lane] = oscillator.left_gain;
            right_gains[lane] = oscillator.right_gain;
            self.oscillator_bank.jitter_remaining[slot] = 0;
        }
        if (shape - 2.0).abs() <= f32::EPSILON {
            accumulate_saw4_block_constant(
                &mut packed,
                f32x4::from(phase_steps),
                f32x4::from(left_gains),
                f32x4::from(right_gains),
                left,
                right,
                settings.antialiasing,
            );
        } else {
            accumulate_shape4_block_constant(
                &mut packed,
                f32x4::from(phase_steps),
                f32x4::from(left_gains),
                f32x4::from(right_gains),
                left,
                right,
                shape,
                pulse_width,
                settings.antialiasing,
            );
        }
        for (lane, entry) in entries.iter().enumerate() {
            self.oscillator_bank.oscillators[usize::from(entry.slot)][0] = packed[lane];
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the exact x8 Saw renderer keeps its fixed render context allocation-free"
    )]
    pub(super) fn accumulate_structural_saw8_block<const SAMPLES: usize>(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        self.advance_settled_structural_jitter_block::<SAMPLES>(slot, oscillator, sample_rate);
        let phase_step = f32x8::from(std::array::from_fn(|lane| {
            (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[lane]).min(0.45)
        }));
        let left_gain = f32x8::from(std::array::from_fn(|lane| {
            oscillator.left_gain * oscillator.lane_left_gains[lane]
        }));
        let right_gain = f32x8::from(std::array::from_fn(|lane| {
            oscillator.right_gain * oscillator.lane_right_gains[lane]
        }));
        accumulate_saw8_block_constant(
            &mut self.oscillator_bank.oscillators[slot][..8],
            phase_step,
            left_gain,
            right_gain,
            left,
            right,
            settings.antialiasing,
        );
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the structural block renderer keeps its fixed render context allocation-free"
    )]
    pub(super) fn accumulate_structural_oscillator_block<const SAMPLES: usize>(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        let voices = usize::from(oscillator.render_voices);
        if oscillator.engine == OscillatorEngineKind::Noise {
            let phase_step = (base_step * oscillator.pitch_ratio).min(0.45);
            let texture = (oscillator.pulse_width - 0.03) / 0.94;
            for frame in 0..SAMPLES {
                let (noise_left, noise_right) = self.oscillator_bank.noise[slot].next(
                    phase_step,
                    shape / 3.0,
                    texture,
                    oscillator.phase_warp.amount,
                    voices,
                    &oscillator.lane_left_gains,
                    &oscillator.lane_right_gains,
                );
                left[frame] += f32x8::splat(noise_left * oscillator.left_gain * 0.125);
                right[frame] += f32x8::splat(noise_right * oscillator.right_gain * 0.125);
            }
            return;
        }
        if oscillator.jitter_active() {
            self.accumulate_jittered_structural_oscillator_block(
                slot,
                oscillator,
                settings,
                sample_rate,
                base_step,
                shape,
                left,
                right,
            );
            return;
        }
        if voices == 1 {
            self.oscillator_bank.jitter_remaining[slot] = 0;
            let phase_step = (base_step * oscillator.pitch_ratio).min(0.45);
            for frame in 0..SAMPLES {
                let sample = if oscillator.custom_mix > f32::EPSILON {
                    self.oscillator_bank.oscillators[slot][0].generate_custom_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                    )
                } else if oscillator.phase_warp.active() {
                    self.oscillator_bank.oscillators[slot][0].generate_shape_step_warped(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    )
                } else {
                    self.oscillator_bank.oscillators[slot][0].generate_shape_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    )
                };
                left[frame] += f32x8::splat(sample * oscillator.left_gain * 0.125);
                right[frame] += f32x8::splat(sample * oscillator.right_gain * 0.125);
            }
            return;
        }

        self.advance_settled_structural_jitter_block::<SAMPLES>(slot, oscillator, sample_rate);
        let packs = voices / 8;
        for pack in 0..packs {
            let index = pack * 8;
            let phase_step = f32x8::from(std::array::from_fn(|lane| {
                (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[index + lane])
                    .min(0.45)
            }));
            let left_gain = f32x8::from(std::array::from_fn(|lane| {
                oscillator.left_gain * oscillator.lane_left_gains[index + lane]
            }));
            let right_gain = f32x8::from(std::array::from_fn(|lane| {
                oscillator.right_gain * oscillator.lane_right_gains[index + lane]
            }));
            let oscillators = &mut self.oscillator_bank.oscillators[slot][index..index + 8];
            if oscillator.custom_mix > f32::EPSILON {
                accumulate_custom8_block_constant(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            } else if oscillator.phase_warp.active() {
                accumulate_shape8_block_constant_warped(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                accumulate_saw8_block_constant(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    settings.antialiasing,
                );
            } else {
                accumulate_shape8_block_constant(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                );
            }
        }

        let mut tail_start = packs * 8;
        if voices - tail_start >= 4 {
            let index = tail_start;
            let phase_step = f32x4::from(std::array::from_fn(|lane| {
                (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[index + lane])
                    .min(0.45)
            }));
            let left_gain = f32x4::from(std::array::from_fn(|lane| {
                oscillator.left_gain * oscillator.lane_left_gains[index + lane]
            }));
            let right_gain = f32x4::from(std::array::from_fn(|lane| {
                oscillator.right_gain * oscillator.lane_right_gains[index + lane]
            }));
            let oscillators = &mut self.oscillator_bank.oscillators[slot][index..index + 4];
            if oscillator.custom_mix > f32::EPSILON {
                accumulate_custom4_block_constant(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            } else if oscillator.phase_warp.active() {
                accumulate_shape4_block_constant_warped(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                accumulate_saw4_block_constant(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    settings.antialiasing,
                );
            } else {
                accumulate_shape4_block_constant(
                    oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                );
            }
            tail_start += 4;
        }

        for lane in tail_start..voices {
            let phase_step =
                (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[lane]).min(0.45);
            for frame in 0..SAMPLES {
                let sample = if oscillator.custom_mix > f32::EPSILON {
                    self.oscillator_bank.oscillators[slot][lane].generate_custom_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                    )
                } else if oscillator.phase_warp.active() {
                    self.oscillator_bank.oscillators[slot][lane].generate_shape_step_warped(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    )
                } else {
                    self.oscillator_bank.oscillators[slot][lane].generate_shape_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    )
                };
                left[frame] += f32x8::splat(
                    sample * oscillator.left_gain * oscillator.lane_left_gains[lane] * 0.125,
                );
                right[frame] += f32x8::splat(
                    sample * oscillator.right_gain * oscillator.lane_right_gains[lane] * 0.125,
                );
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the jittered structural block renderer keeps its fixed render context allocation-free"
    )]
    pub(super) fn accumulate_jittered_structural_oscillator_block<const SAMPLES: usize>(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        let voices = usize::from(oscillator.render_voices);
        debug_assert!(voices > 1);
        debug_assert!(oscillator.jitter_active());
        let packs = voices / 8;
        let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
        let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
        let oscillator_step = base_step * oscillator.pitch_ratio;
        for frame in 0..SAMPLES {
            self.advance_structural_jitter(slot, slot, oscillator, sample_rate);
            for pack in 0..packs {
                let index = pack * 8;
                steps[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                    let lane = index + lane;
                    self.oscillator_bank.jitter_ratios[slot][lane] +=
                        self.oscillator_bank.jitter_steps[slot][lane];
                    (oscillator_step
                        * oscillator.lane_pitch_ratios[lane]
                        * self.oscillator_bank.jitter_ratios[slot][lane])
                        .min(0.45)
                }));
            }
            for (tail, lane) in (packs * 8..voices).enumerate() {
                self.oscillator_bank.jitter_ratios[slot][lane] +=
                    self.oscillator_bank.jitter_steps[slot][lane];
                tail_steps[tail][frame] = (oscillator_step
                    * oscillator.lane_pitch_ratios[lane]
                    * self.oscillator_bank.jitter_ratios[slot][lane])
                    .min(0.45);
            }
        }

        for (pack, phase_steps) in steps.iter().copied().enumerate().take(packs) {
            let index = pack * 8;
            let left_gain = f32x8::from(std::array::from_fn(|lane| {
                oscillator.left_gain * oscillator.lane_left_gains[index + lane]
            }));
            let right_gain = f32x8::from(std::array::from_fn(|lane| {
                oscillator.right_gain * oscillator.lane_right_gains[index + lane]
            }));
            let oscillators = &mut self.oscillator_bank.oscillators[slot][index..index + 8];
            if oscillator.custom_mix > f32::EPSILON {
                accumulate_custom8_block(
                    oscillators,
                    phase_steps,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            } else if !oscillator.phase_warp.active() && shape == 2.0 {
                accumulate_saw8_block(
                    oscillators,
                    phase_steps,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    settings.antialiasing,
                );
            } else {
                accumulate_shape8_block_steps(
                    oscillators,
                    phase_steps,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            }
        }

        let mut tail_start = packs * 8;
        if voices - tail_start >= 4 {
            let phase_steps = std::array::from_fn(|frame| {
                f32x4::from(std::array::from_fn(|lane| tail_steps[lane][frame]))
            });
            let left_gain = f32x4::from(std::array::from_fn(|lane| {
                oscillator.left_gain * oscillator.lane_left_gains[tail_start + lane]
            }));
            let right_gain = f32x4::from(std::array::from_fn(|lane| {
                oscillator.right_gain * oscillator.lane_right_gains[tail_start + lane]
            }));
            let oscillators =
                &mut self.oscillator_bank.oscillators[slot][tail_start..tail_start + 4];
            if oscillator.custom_mix > f32::EPSILON {
                accumulate_custom4_block(
                    oscillators,
                    phase_steps,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            } else if !oscillator.phase_warp.active() && shape == 2.0 {
                accumulate_saw4_block(
                    oscillators,
                    phase_steps,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    settings.antialiasing,
                );
            } else {
                accumulate_shape4_block_steps(
                    oscillators,
                    phase_steps,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    shape,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                );
            }
            tail_start += 4;
        }

        for (tail, lane) in (tail_start..voices).enumerate() {
            for frame in 0..SAMPLES {
                let phase_step = tail_steps[tail + tail_start - packs * 8][frame];
                let sample = if oscillator.custom_mix > f32::EPSILON {
                    self.oscillator_bank.oscillators[slot][lane].generate_custom_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                    )
                } else if oscillator.phase_warp.active() {
                    self.oscillator_bank.oscillators[slot][lane].generate_shape_step_warped(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    )
                } else {
                    self.oscillator_bank.oscillators[slot][lane].generate_shape_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    )
                };
                left[frame] += f32x8::splat(
                    sample * oscillator.left_gain * oscillator.lane_left_gains[lane] * 0.125,
                );
                right[frame] += f32x8::splat(
                    sample * oscillator.right_gain * oscillator.lane_right_gains[lane] * 0.125,
                );
            }
        }
    }

    pub(super) fn finish_saw_block<const SAMPLES: usize>(
        &mut self,
        mut output: [(f32, f32); SAMPLES],
        amplitude: &[f32; SAMPLES],
        settings: VoiceSettings,
        swarm_clocks: &[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        shapes: Option<&[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion: Option<&[[UnisonMotionFrame; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion_mask: OscillatorMask,
    ) -> [(f32, f32); SAMPLES] {
        let primary = settings.oscillator(0);
        if primary.enabled
            && (primary.level.to_bits() != 1.0_f32.to_bits()
                || primary.pan.to_bits() != 0.0_f32.to_bits())
        {
            let (left, right) = primary.channel_gains();
            for sample in &mut output {
                sample.0 *= left;
                sample.1 *= right;
            }
        }
        for oscillator in 1..LEGACY_OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                self.mix_secondary_saw_block(
                    &mut output,
                    amplitude,
                    settings,
                    oscillator,
                    &swarm_clocks[oscillator],
                    shapes.map(|shapes| &shapes[oscillator]),
                    motion,
                    motion_mask,
                );
            }
        }
        output
    }

    #[inline(never)]
    pub(super) fn render_single_lane_primary_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        oscillator: OscillatorSettings,
        shape: f32,
        shapes: Option<&[f32; SAMPLES]>,
        amplitude: &[f32; SAMPLES],
        swarm_clocks: &[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        all_shapes: Option<&[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion: Option<&[[UnisonMotionFrame; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion_mask: OscillatorMask,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(!self.unison.settings.motion_active());
        debug_assert_eq!(self.unison.left[0].to_bits(), 1.0_f32.to_bits());
        debug_assert_eq!(self.unison.right[0].to_bits(), 1.0_f32.to_bits());
        debug_assert_eq!(self.unison.gain.to_bits(), 1.0_f32.to_bits());
        let phase_step = tuned_phase_step(self.phase_steps[0], oscillator.pitch_ratio);
        let samples = self.render_single_lane_block(
            0,
            oscillator,
            shape,
            shapes,
            phase_step,
            settings.antialiasing,
        );
        let output = std::array::from_fn(|frame| {
            let sample = samples[frame] * amplitude[frame];
            (sample, sample)
        });
        self.finish_saw_block(
            output,
            amplitude,
            settings,
            swarm_clocks,
            all_shapes,
            motion,
            motion_mask,
        )
    }

    #[inline(never)]
    pub(super) fn render_single_lane_block<const SAMPLES: usize>(
        &mut self,
        oscillator_index: usize,
        oscillator: OscillatorSettings,
        shape: f32,
        shapes: Option<&[f32; SAMPLES]>,
        phase_step: f32,
        antialiasing: Antialiasing,
    ) -> [f32; SAMPLES] {
        if oscillator.custom_active() {
            std::array::from_fn(|_| {
                self.oscillators[oscillator_index][0].generate_custom_step(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            })
        } else if oscillator.phase_warp_active() {
            std::array::from_fn(|frame| {
                self.oscillators[oscillator_index][0].generate_shape_step_warped(
                    shapes.map_or(shape, |shapes| shapes[frame]),
                    phase_step,
                    oscillator.pulse_width,
                    antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            })
        } else {
            std::array::from_fn(|frame| {
                self.oscillators[oscillator_index][0].generate_shape_step(
                    shapes.map_or(shape, |shapes| shapes[frame]),
                    phase_step,
                    oscillator.pulse_width,
                    antialiasing,
                )
            })
        }
    }

    pub(super) fn mix_secondary_saw_block<const SAMPLES: usize>(
        &mut self,
        output: &mut [(f32, f32); SAMPLES],
        amplitude: &[f32; SAMPLES],
        settings: VoiceSettings,
        oscillator_index: usize,
        swarm_clocks: &[f32; SAMPLES],
        shapes: Option<&[f32; SAMPLES]>,
        motion: Option<&[[UnisonMotionFrame; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion_mask: OscillatorMask,
    ) {
        let oscillator = settings.oscillator(oscillator_index);
        let secondary = oscillator_index - 1;
        let shape = self.effective_oscillator_shape(settings, oscillator_index);
        let shapes = shapes.map(|shapes| {
            std::array::from_fn(|frame| {
                self.effective_oscillator_shape_value(settings, oscillator_index, shapes[frame])
            })
        });
        if self.secondary_phase_steps_dirty[secondary] {
            self.refresh_secondary_phase_steps(secondary);
        }
        let unison_settings = self.secondary_unison[secondary].settings;
        let unison_gain = self.secondary_unison[secondary].gain;
        let voice_count = usize::from(self.secondary_unison[secondary].render_voices);
        let secondary_motion_dynamic = motion_mask & (1 << oscillator_index) != 0;
        if voice_count == 1 {
            if secondary_motion_dynamic && let Some(motion) = motion {
                self.configure_secondary_unison_motion_frame(
                    secondary,
                    motion[oscillator_index][SAMPLES - 1],
                );
            }
            debug_assert!(!unison_settings.motion_active());
            debug_assert_eq!(
                self.secondary_unison[secondary].left[0].to_bits(),
                1.0_f32.to_bits()
            );
            debug_assert_eq!(
                self.secondary_unison[secondary].right[0].to_bits(),
                1.0_f32.to_bits()
            );
            debug_assert_eq!(unison_gain.to_bits(), 1.0_f32.to_bits());
            let phase_step =
                self.secondary_oscillator_phase_step(secondary, 0, oscillator.pitch_ratio, None);
            let samples = self.render_single_lane_block(
                oscillator_index,
                oscillator,
                shape,
                shapes.as_ref(),
                phase_step,
                settings.antialiasing,
            );
            let (channel_left, channel_right) = oscillator.channel_gains();
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame] * (amplitude[frame] * channel_left);
                output[frame].1 += samples[frame] * (amplitude[frame] * channel_right);
            }
            return;
        }
        let morph_gains = shapes
            .as_ref()
            .map(|shapes| std::array::from_fn(|frame| shape_morph_gain(shapes[frame])));
        let packs = voice_count / 8;
        let has_simd4_tail = voice_count % 8 >= 4;
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        if !secondary_motion_dynamic && !unison_settings.motion_active() {
            for pack in 0..packs {
                let index = pack * 8;
                let steps = f32x8::from(std::array::from_fn(|lane| {
                    self.secondary_oscillator_phase_step(
                        secondary,
                        index + lane,
                        oscillator.pitch_ratio,
                        None,
                    )
                }));
                let left_gain = f32x8::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].left[index + lane]
                }));
                let right_gain = f32x8::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].right[index + lane]
                }));
                if let (Some(shapes), Some(morph_gains)) = (&shapes, &morph_gains) {
                    accumulate_shape8_block_morphing(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shapes,
                        morph_gains,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                } else if oscillator.custom_active() {
                    accumulate_custom8_block_constant(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    );
                } else if oscillator.phase_warp_active() {
                    accumulate_shape8_block_constant_warped(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    );
                } else if (shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw8_block_constant(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape8_block_constant(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                }
            }
            let mut tail_start = packs * 8;
            if has_simd4_tail {
                let steps = f32x4::from(std::array::from_fn(|lane| {
                    self.secondary_oscillator_phase_step(
                        secondary,
                        tail_start + lane,
                        oscillator.pitch_ratio,
                        None,
                    )
                }));
                let left_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].left[tail_start + lane]
                }));
                let right_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].right[tail_start + lane]
                }));
                if let (Some(shapes), Some(morph_gains)) = (&shapes, &morph_gains) {
                    accumulate_shape4_block_morphing(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shapes,
                        morph_gains,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                } else if oscillator.custom_active() {
                    accumulate_custom4_block_constant(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    );
                } else if oscillator.phase_warp_active() {
                    accumulate_shape4_block_constant_warped(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    );
                } else if (shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw4_block_constant(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape4_block_constant(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                }
                tail_start += 4;
            }
            for index in tail_start..voice_count {
                let phase_step = self.secondary_oscillator_phase_step(
                    secondary,
                    index,
                    oscillator.pitch_ratio,
                    None,
                );
                for frame in 0..SAMPLES {
                    let sample = if oscillator.custom_active() {
                        self.oscillators[oscillator_index][index].generate_custom_step(
                            shape,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            oscillator.phase_warp.mode,
                            oscillator.phase_warp.amount,
                            oscillator.custom_curve,
                            oscillator.custom_mix,
                        )
                    } else {
                        self.oscillators[oscillator_index][index].generate_shape_step(
                            shapes.as_ref().map_or(shape, |shapes| shapes[frame]),
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        )
                    };
                    left[frame] +=
                        f32x8::splat(sample * self.secondary_unison[secondary].left[index] * 0.125);
                    right[frame] += f32x8::splat(
                        sample * self.secondary_unison[secondary].right[index] * 0.125,
                    );
                }
            }
        } else {
            self.set_secondary_swarm_clock(oscillator_index, swarm_clocks[0]);
            if !secondary_motion_dynamic && self.secondary_swarm_update_remaining[secondary] == 0 {
                let update_interval = self.secondary_swarm_update_interval(secondary);
                self.prepare_secondary_swarm_jitter_target(secondary, update_interval);
                self.secondary_swarm_update_remaining[secondary] = update_interval;
            }
            let neutral_tune = oscillator.pitch_ratio.to_bits() == 1.0_f32.to_bits();
            let constant_ramp = !secondary_motion_dynamic
                && usize::from(self.secondary_swarm_update_remaining[secondary]) >= SAMPLES;
            let tuned_final_steps = if neutral_tune || !constant_ramp {
                None
            } else {
                constant_jitter_ramp_final::<SAMPLES>(
                    &self.secondary_phase_steps[secondary],
                    &self.secondary_swarm_pitch_step[secondary],
                    usize::from(unison_settings.voices),
                    oscillator.pitch_ratio,
                )
            };
            if constant_ramp
                && !oscillator.custom_active()
                && (neutral_tune || tuned_final_steps.is_some())
            {
                for pack in 0..packs {
                    let index = pack * 8;
                    let dynamic_step = f32x8::from(std::array::from_fn(|lane| {
                        tuned_phase_step(
                            self.secondary_phase_steps[secondary][index + lane],
                            oscillator.pitch_ratio,
                        )
                    }));
                    let delta = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_swarm_pitch_step[secondary][index + lane]
                            * oscillator.pitch_ratio
                    }));
                    let left_gain = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[index + lane]
                    }));
                    let right_gain = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[index + lane]
                    }));
                    let final_steps: [f32; 8] =
                        if let (Some(shapes), Some(morph_gains)) = (&shapes, &morph_gains) {
                            let steps = std::array::from_fn(|frame| {
                                dynamic_step + delta * f32x8::splat((frame + 1) as f32)
                            });
                            accumulate_shape8_block_dynamic(
                                &mut self.oscillators[oscillator_index][index..index + 8],
                                steps,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                shapes,
                                morph_gains,
                                oscillator.pulse_width,
                                settings.antialiasing,
                            );
                            steps[SAMPLES - 1].into()
                        } else {
                            if is_narrow_spline_ramp::<SAMPLES>(
                                dynamic_step,
                                delta,
                                settings.antialiasing,
                            ) {
                                accumulate_saw8_block_static_gains_narrow_spline(
                                    &mut self.oscillators[oscillator_index][index..index + 8],
                                    dynamic_step,
                                    delta,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    settings.antialiasing,
                                )
                            } else {
                                accumulate_saw8_block_static_gains(
                                    &mut self.oscillators[oscillator_index][index..index + 8],
                                    dynamic_step,
                                    delta,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    settings.antialiasing,
                                )
                            }
                            .into()
                        };
                    if neutral_tune {
                        self.secondary_phase_steps[secondary][index..index + 8]
                            .copy_from_slice(&final_steps);
                    }
                }
                let mut tail_start = packs * 8;
                if has_simd4_tail {
                    let dynamic_step = f32x4::from(std::array::from_fn(|lane| {
                        tuned_phase_step(
                            self.secondary_phase_steps[secondary][tail_start + lane],
                            oscillator.pitch_ratio,
                        )
                    }));
                    let delta = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_swarm_pitch_step[secondary][tail_start + lane]
                            * oscillator.pitch_ratio
                    }));
                    let left_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[tail_start + lane]
                    }));
                    let right_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[tail_start + lane]
                    }));
                    let final_steps: [f32; 4] =
                        if let (Some(shapes), Some(morph_gains)) = (&shapes, &morph_gains) {
                            let steps = std::array::from_fn(|frame| {
                                dynamic_step + delta * f32x4::splat((frame + 1) as f32)
                            });
                            accumulate_shape4_block_dynamic(
                                &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                                steps,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                shapes,
                                morph_gains,
                                oscillator.pulse_width,
                                settings.antialiasing,
                            );
                            steps[SAMPLES - 1].into()
                        } else {
                            accumulate_saw4_block_static_gains(
                                &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                                dynamic_step,
                                delta,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                settings.antialiasing,
                            )
                            .into()
                        };
                    if neutral_tune {
                        self.secondary_phase_steps[secondary][tail_start..tail_start + 4]
                            .copy_from_slice(&final_steps);
                    }
                    tail_start += 4;
                }
                for index in tail_start..voice_count {
                    let mut phase_step = tuned_phase_step(
                        self.secondary_phase_steps[secondary][index],
                        oscillator.pitch_ratio,
                    );
                    let phase_step_delta =
                        self.secondary_swarm_pitch_step[secondary][index] * oscillator.pitch_ratio;
                    for frame in 0..SAMPLES {
                        phase_step += phase_step_delta;
                        let sample = self.oscillators[oscillator_index][index].generate_shape_step(
                            shapes.as_ref().map_or(2.0, |shapes| shapes[frame]),
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        left[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].left[index] * 0.125,
                        );
                        right[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].right[index] * 0.125,
                        );
                    }
                    if neutral_tune {
                        self.secondary_phase_steps[secondary][index] = phase_step;
                    }
                }
                if let Some(final_steps) = tuned_final_steps {
                    let voices = usize::from(unison_settings.voices);
                    self.secondary_phase_steps[secondary][..voices]
                        .copy_from_slice(&final_steps[..voices]);
                }
                self.secondary_swarm_update_remaining[secondary] -= SAMPLES as u16;
                self.set_secondary_swarm_clock(oscillator_index, swarm_clocks[SAMPLES - 2]);
            } else {
                let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
                let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
                for frame in 0..SAMPLES {
                    if secondary_motion_dynamic && let Some(motion) = motion {
                        self.configure_secondary_unison_motion_frame(
                            secondary,
                            motion[oscillator_index][frame],
                        );
                    }
                    self.set_secondary_swarm_clock(oscillator_index, swarm_clocks[frame]);
                    if self.secondary_unison[secondary].settings.motion_active() {
                        self.advance_secondary_swarm(secondary);
                    }
                    for pack in 0..packs {
                        let index = pack * 8;
                        steps[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                            self.secondary_oscillator_phase_step(
                                secondary,
                                index + lane,
                                oscillator.pitch_ratio,
                                None,
                            )
                        }));
                    }
                    for (tail, index) in
                        (packs * 8..usize::from(unison_settings.voices)).enumerate()
                    {
                        tail_steps[tail][frame] = self.secondary_oscillator_phase_step(
                            secondary,
                            index,
                            oscillator.pitch_ratio,
                            None,
                        );
                    }
                }
                for pack in 0..packs {
                    let index = pack * 8;
                    let left_gain = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[index + lane]
                    }));
                    let right_gain = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[index + lane]
                    }));
                    if oscillator.custom_active() {
                        accumulate_custom8_block(
                            &mut self.oscillators[oscillator_index][index..index + 8],
                            steps[pack],
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            oscillator.custom_curve,
                            oscillator.custom_mix,
                            shape,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            oscillator.phase_warp.mode,
                            oscillator.phase_warp.amount,
                        );
                    } else if let (Some(shapes), Some(morph_gains)) = (&shapes, &morph_gains) {
                        accumulate_shape8_block_dynamic(
                            &mut self.oscillators[oscillator_index][index..index + 8],
                            steps[pack],
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            shapes,
                            morph_gains,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                    } else {
                        accumulate_saw8_block(
                            &mut self.oscillators[oscillator_index][index..index + 8],
                            steps[pack],
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            settings.antialiasing,
                        );
                    }
                }
                let mut tail_start = packs * 8;
                if has_simd4_tail {
                    let steps4 = std::array::from_fn(|frame| {
                        f32x4::from(std::array::from_fn(|lane| tail_steps[lane][frame]))
                    });
                    let left_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[tail_start + lane]
                    }));
                    let right_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[tail_start + lane]
                    }));
                    if oscillator.custom_active() {
                        accumulate_custom4_block(
                            &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                            steps4,
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            oscillator.custom_curve,
                            oscillator.custom_mix,
                            shape,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            oscillator.phase_warp.mode,
                            oscillator.phase_warp.amount,
                        );
                    } else if let (Some(shapes), Some(morph_gains)) = (&shapes, &morph_gains) {
                        accumulate_shape4_block_dynamic(
                            &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                            steps4,
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            shapes,
                            morph_gains,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                    } else {
                        accumulate_saw4_block(
                            &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                            steps4,
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            settings.antialiasing,
                        );
                    }
                    tail_start += 4;
                }
                for (tail, index) in (tail_start..voice_count).enumerate() {
                    let tail = tail + tail_start - packs * 8;
                    for frame in 0..SAMPLES {
                        let sample = if oscillator.custom_active() {
                            self.oscillators[oscillator_index][index].generate_custom_step(
                                shape,
                                tail_steps[tail][frame],
                                oscillator.pulse_width,
                                settings.antialiasing,
                                oscillator.phase_warp.mode,
                                oscillator.phase_warp.amount,
                                oscillator.custom_curve,
                                oscillator.custom_mix,
                            )
                        } else {
                            self.oscillators[oscillator_index][index].generate_shape_step(
                                shapes.as_ref().map_or(2.0, |shapes| shapes[frame]),
                                tail_steps[tail][frame],
                                oscillator.pulse_width,
                                settings.antialiasing,
                            )
                        };
                        left[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].left[index] * 0.125,
                        );
                        right[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].right[index] * 0.125,
                        );
                    }
                }
            }
        }
        let (channel_left, channel_right) = oscillator.channel_gains();
        for frame in 0..SAMPLES {
            let gain = amplitude[frame] * unison_gain;
            output[frame].0 += left[frame].reduce_add() * (gain * channel_left);
            output[frame].1 += right[frame].reduce_add() * (gain * channel_right);
        }
    }
}
