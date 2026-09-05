//! Steady oscillator-bank block rendering kernels.

use super::*;

#[inline(always)]
fn apply_generator_gain_route(
    base_left: f32,
    base_right: f32,
    base_gain_position: Option<(f32, f32)>,
    source: f32,
    control: crate::OscillatorControl,
    amount: f32,
) -> (f32, f32, f32) {
    if control == crate::OscillatorControl::RingModAmount {
        let wet = amount.abs();
        return (
            base_left,
            base_right,
            (1.0 - wet) + source * amount.signum() * wet,
        );
    }
    let delta = source * amount;
    if delta == 0.0 {
        return (base_left, base_right, 1.0);
    }
    let (base_level, base_pan) =
        base_gain_position.expect("level/pan route must prepare its base gain");
    let level = (base_level
        + if control == crate::OscillatorControl::Level {
            delta
        } else {
            0.0
        })
    .clamp(0.0, 1.0);
    let pan = (base_pan
        + if control == crate::OscillatorControl::Pan {
            delta
        } else {
            0.0
        })
    .clamp(-1.0, 1.0);
    (level * (1.0 - pan).sqrt(), level * (1.0 + pan).sqrt(), 1.0)
}

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
        let gain_mask = controls[0].mask;
        if !self.is_gliding()
            && gain_mask.count_ones() == 1
            && controls
                .iter()
                .all(|control| control.mask == gain_mask && control.gain_only_mask == gain_mask)
        {
            let slot = gain_mask.trailing_zeros() as usize;
            let gains = std::array::from_fn(|frame| {
                let control = controls[frame].slots[slot];
                (control.left_gain, control.right_gain)
            });
            return self.render_gain_modulated_oscillator_bank_block::<SAMPLES>(
                settings,
                sample_rate,
                oscillator_bank,
                slot,
                &gains,
            );
        }
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
        if !self.is_gliding()
            && let Some(slot) = routes.oscillator_gain_slot()
            && controls
                .iter()
                .all(|control| control.mask == 1 << slot && control.gain_only_mask == control.mask)
        {
            let gains = std::array::from_fn(|frame| {
                let mut modulation = crate::StructuralModulationFrame::default();
                routes.evaluate(self.modulation.next(program), &mut modulation);
                let control = merge_voice_structural_block_control(
                    &controls[frame],
                    &modulation,
                    oscillator_bank,
                );
                let gain = control.slots[slot];
                (gain.left_gain, gain.right_gain)
            });
            return self.render_gain_modulated_oscillator_bank_block::<SAMPLES>(
                settings,
                sample_rate,
                oscillator_bank,
                slot,
                &gains,
            );
        }
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

    pub(in crate::voices) fn oscillator_bank_block_voice_eligible(
        &self,
        _active: &ActiveOscillatorRenderSet,
    ) -> bool {
        !self.is_gliding()
    }

    pub(super) fn settled_oscillator_bank_voice_eligible(
        &self,
        active: &ActiveOscillatorRenderSet,
    ) -> bool {
        if !self.oscillator_bank_block_voice_eligible(active)
            || !self.held
            || self.amplitude_level() <= f32::EPSILON
            || self.envelope.sustain <= f32::EPSILON
            || self.envelope_declicker.active()
            || (self.group_envelope_count != 0 && !self.group_envelopes[0].is_sustaining())
        {
            return false;
        }
        true
    }

    fn group_envelope_gain_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        group_count: usize,
    ) -> [[f32; SAMPLES]; MAX_OUTPUT_PAIRS] {
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let performance_gain = velocity_gain * pressure_gain;
        let mut gains = [[0.0; SAMPLES]; MAX_OUTPUT_PAIRS];
        if self.voice_amplitude_settled() {
            if self.group_envelope_count == 0 {
                self.envelope_level = self.envelope.sustain.clamp(0.0, 1.0);
            }
            let group_gains = self.group_envelope_gains();
            for group in 0..group_count {
                gains[group].fill(self.envelope_level * group_gains[group] * performance_gain);
            }
            return gains;
        }
        for frame in 0..SAMPLES {
            self.advance_envelope(sample_rate, false);
            let group_gains = self.group_envelope_gains();
            for group in 0..group_count {
                gains[group][frame] = self.envelope_level * group_gains[group] * performance_gain;
            }
        }
        gains
    }

    pub(in crate::voices) fn settled_grouped_bank_voice_eligible(
        &self,
        active: &ActiveOscillatorRenderSet,
    ) -> bool {
        if !self.settled_oscillator_bank_voice_eligible(active) {
            return false;
        }
        self.group_envelopes_settled()
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
            if oscillator.engine == OscillatorEngineKind::Va
                && oscillator.render_voices == 8
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
        if self.voice_amplitude_settled() {
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

    fn render_gain_modulated_oscillator_bank_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        target_slot: usize,
        gains: &[(f32, f32); SAMPLES],
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
        let mut target_left = [f32x8::ZERO; SAMPLES];
        let mut target_right = [f32x8::ZERO; SAMPLES];
        for entry in active.entries() {
            let slot = usize::from(entry.slot);
            let mut unity = entry.current;
            let (oscillator, output_left, output_right) = if slot == target_slot {
                unity.left_gain = 1.0;
                unity.right_gain = 1.0;
                (&unity, &mut target_left, &mut target_right)
            } else {
                (&entry.current, &mut left, &mut right)
            };
            let shape = (oscillator.shape + timbre).clamp(0.0, 3.0);
            if oscillator.engine == OscillatorEngineKind::Va
                && oscillator.render_voices == 8
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
                    output_left,
                    output_right,
                );
            } else {
                self.accumulate_structural_oscillator_block(
                    slot,
                    oscillator,
                    settings,
                    sample_rate,
                    base_step,
                    shape,
                    output_left,
                    output_right,
                );
            }
        }
        for frame in 0..SAMPLES {
            left[frame] += target_left[frame] * f32x8::splat(gains[frame].0);
            right[frame] += target_right[frame] * f32x8::splat(gains[frame].1);
        }
        if self.voice_amplitude_settled() {
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
        debug_assert!(self.oscillator_bank_block_voice_eligible(active));
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let active_group_mask = if self.group_envelope_count == 0 {
            u8::MAX
        } else {
            (0..group_count).fold(0, |mask, group| {
                mask | (u8::from(self.group_envelopes[group].active()) << group)
            })
        };
        let envelope_gains =
            self.group_envelope_gain_block::<SAMPLES>(settings, sample_rate, group_count);
        let entries = active.entries();
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for group in 0..group_count {
            if active_group_mask & (1 << group) == 0 {
                continue;
            }
            let mut left = [f32x8::ZERO; SAMPLES];
            let mut right = [f32x8::ZERO; SAMPLES];
            let mut any = false;
            for entry in entries {
                let slot = usize::from(entry.slot);
                if oscillator_group(oscillator_groups, group_count, slot) != group {
                    continue;
                }
                let oscillator = &entry.current;
                if oscillator.engine == OscillatorEngineKind::Va
                    && oscillator.render_voices == 1
                    && oscillator.custom_mix <= f32::EPSILON
                    && !oscillator.phase_warp.active()
                    && oscillator.left_gain == 0.0
                    && oscillator.right_gain == 0.0
                {
                    self.oscillator_bank.jitter_remaining[slot] = 0;
                    let phase_step = (self.structural_base_step(oscillator, base_step)
                        * oscillator.pitch_ratio)
                        .min(0.45);
                    for _ in 0..SAMPLES {
                        self.oscillator_bank.oscillators[slot][0].offset_phase(phase_step);
                    }
                    continue;
                }
                any = true;
                let shape = (oscillator.shape + timbre).clamp(0.0, 3.0);
                if oscillator.engine == OscillatorEngineKind::Va
                    && oscillator.render_voices == 8
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
            for frame in 0..SAMPLES {
                output[group][frame] = (
                    left[frame].reduce_add() * envelope_gains[group][frame],
                    right[frame].reduce_add() * envelope_gains[group][frame],
                );
            }
        }
        output
    }

    pub(in crate::voices) fn render_ordered_oscillator_groups_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        filters: &[FilterCoefficients; MAX_FILTERS],
        filter_block: Option<&[[FilterCoefficients; MAX_FILTERS]]>,
        gain_modulation: Option<(usize, &[(f32, f32); SAMPLES])>,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.oscillator_bank_block_voice_eligible(active));
        debug_assert!(filter_block.is_none_or(|block| block.len() == SAMPLES));
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let active_group_mask = if self.group_envelope_count == 0 {
            u8::MAX
        } else {
            (0..group_count).fold(0, |mask, group| {
                mask | (u8::from(self.group_envelopes[group].active()) << group)
            })
        };
        let envelope_gains =
            self.group_envelope_gain_block::<SAMPLES>(settings, sample_rate, group_count);
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for (group_index, group) in groups.iter().take(group_count).enumerate() {
            if group.oscillator_mask() == 0
                || self.group_active_mask & (1 << group_index) == 0
                || active_group_mask & (1 << group_index) == 0
            {
                continue;
            }
            let mut left = [f32x8::ZERO; SAMPLES];
            let mut right = [f32x8::ZERO; SAMPLES];
            for (module_index, module) in group.modules().iter().enumerate() {
                match *module {
                    GeneratorRtModule::Oscillator(slot) => {
                        let slot = slot.index();
                        if active.mask & (1 << slot) == 0 {
                            continue;
                        }
                        let base_oscillator = &active.entry(slot).current;
                        let gains = gain_modulation
                            .filter(|(target, _)| *target == slot)
                            .map(|(_, gains)| gains);
                        let ratio_brickwall =
                            downstream_ratio_brickwall(group.modules(), module_index, filters);
                        if let Some(gains) = gains {
                            let mut oscillator = *base_oscillator;
                            oscillator.left_gain = 1.0;
                            oscillator.right_gain = 1.0;
                            let shape = (oscillator.shape + timbre).clamp(0.0, 3.0);
                            let mut modulated_left = [f32x8::ZERO; SAMPLES];
                            let mut modulated_right = [f32x8::ZERO; SAMPLES];
                            self.accumulate_ordered_oscillator_block(
                                slot,
                                &oscillator,
                                settings,
                                sample_rate,
                                base_step,
                                shape,
                                ratio_brickwall,
                                &mut modulated_left,
                                &mut modulated_right,
                            );
                            for frame in 0..SAMPLES {
                                left[frame] += modulated_left[frame] * f32x8::splat(gains[frame].0);
                                right[frame] +=
                                    modulated_right[frame] * f32x8::splat(gains[frame].1);
                            }
                        } else {
                            let shape = (base_oscillator.shape + timbre).clamp(0.0, 3.0);
                            self.accumulate_ordered_oscillator_block(
                                slot,
                                base_oscillator,
                                settings,
                                sample_rate,
                                base_step,
                                shape,
                                ratio_brickwall,
                                &mut left,
                                &mut right,
                            );
                        }
                    }
                    GeneratorRtModule::Filter(slot) => {
                        let slot = slot.index();
                        for frame in 0..SAMPLES {
                            let filtered = self.filters[slot].process(
                                filter_block.map_or(filters[slot], |block| block[frame][slot]),
                                left[frame].reduce_add(),
                                right[frame].reduce_add(),
                            );
                            left[frame] =
                                f32x8::from([filtered.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
                            right[frame] =
                                f32x8::from([filtered.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
                        }
                    }
                    GeneratorRtModule::Aux(_) => {}
                }
            }
            for frame in 0..SAMPLES {
                output[group_index][frame] = (
                    left[frame].reduce_add() * envelope_gains[group_index][frame],
                    right[frame].reduce_add() * envelope_gains[group_index][frame],
                );
            }
        }
        output
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the ordered oscillator kernel keeps its fixed render context allocation-free"
    )]
    fn accumulate_ordered_oscillator_block<const SAMPLES: usize>(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        ratio_brickwall: Option<(f32, f32)>,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        if ratio_brickwall.is_none()
            && oscillator.engine == OscillatorEngineKind::Va
            && oscillator.render_voices == 8
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
                left,
                right,
            );
        } else {
            self.accumulate_structural_oscillator_ratio_block(
                slot,
                oscillator,
                settings,
                sample_rate,
                base_step,
                shape,
                ratio_brickwall,
                left,
                right,
            );
        }
    }

    pub(in crate::voices) fn render_phase_mod_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        controls: Option<&[StructuralOscillatorFrameControl]>,
        voice_modulation: Option<(&VoiceLfoProgram, &VoiceStructuralRouteFrame)>,
        filters: &[FilterCoefficients; MAX_FILTERS],
        filter_block: Option<&[[FilterCoefficients; MAX_FILTERS]]>,
        aux: &[AuxConfig; MAX_AUX_MODULES],
        generator_routes: Option<&GeneratorStructuralRouteFrame>,
        generator_route_amounts: Option<(u8, &[f32])>,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.settled_grouped_bank_voice_eligible(active));
        debug_assert!(controls.is_none_or(|controls| controls.len() == SAMPLES));
        debug_assert!(filter_block.is_none_or(|block| block.len() == SAMPLES));
        debug_assert!(generator_route_amounts.is_none_or(|(_, amounts)| amounts.len() == SAMPLES));
        let voice_has_regular_routes =
            voice_modulation.is_some_and(|(_, routes)| routes.has_regular_routes());
        let voice_has_generator_depth =
            voice_modulation.is_some_and(|(_, routes)| routes.has_generator_depth());
        let mut voice_generator_route_amounts = [0.0_f32; SAMPLES];
        let mut voice_depth_buffered = false;
        let generator_route_amounts = (|| {
            let (program, voice_routes) = voice_modulation?;
            if !voice_routes.generator_depth_only() {
                return None;
            }
            let generator_routes = generator_routes?;
            let target = voice_routes.generator_depth_target()?;
            if generator_route_amounts.is_some_and(|(external, _)| external != target) {
                return None;
            }
            let base = generator_routes.route_amount(target)?;
            for (frame, amount) in voice_generator_route_amounts.iter_mut().enumerate() {
                *amount = voice_routes.generator_depth_amount(
                    self.modulation.next(program),
                    target,
                    generator_route_amounts.map_or(base, |(_, amounts)| amounts[frame]),
                );
            }
            voice_depth_buffered = true;
            Some(target)
        })()
        .map(|target| (target, voice_generator_route_amounts.as_slice()))
        .or(generator_route_amounts);
        let voice_requires_sample_routes = voice_has_regular_routes
            || voice_modulation
                .is_some_and(|(_, routes)| routes.generator_depth_only() && !voice_depth_buffered);
        if voice_modulation.is_none()
            && generator_route_amounts.is_none()
            && controls.is_none_or(|controls| controls.iter().all(|control| control.mask == 0))
            && generator_routes.is_some_and(|routes| {
                routes.source_mask() == 0
                    && !routes.filter_routes_active()
                    && !routes.aux_routes_active()
            })
        {
            return self.render_ordered_oscillator_groups_block(
                settings,
                sample_rate,
                active,
                groups,
                group_count,
                filters,
                filter_block,
                None,
            );
        }
        if let Some(generator_routes) = generator_routes {
            let combined_voice_filter = voice_modulation.is_some_and(|(_, routes)| {
                generator_routes.single_filter_route().is_some_and(
                    |(route, _, slot, _, control)| {
                        routes
                            .combined_generator_child(route)
                            .is_some_and(|(_, _, _, target)| {
                                matches!(
                                    target,
                                    crate::ResolvedModularTarget::Filter {
                                        slot: target_slot,
                                        control: target_control,
                                    } if target_slot == slot && target_control == control
                                )
                            })
                    },
                )
            });
            let gain_only_controls = controls.is_some_and(|controls| {
                controls.iter().any(|control| control.mask != 0)
                    && controls
                        .iter()
                        .all(|control| control.mask == control.gain_only_mask)
            });
            let oscillator_controls_only = controls.is_some()
                && generator_routes.source_mask() == 0
                && !generator_routes.filter_routes_active()
                && !generator_routes.aux_routes_active();
            let dynamic_filter_gain_only = filter_block.is_some()
                && gain_only_controls
                && groups.iter().take(group_count).all(|group| {
                    group.modules().iter().all(|module| {
                        !matches!(
                            module,
                            GeneratorRtModule::Filter(slot)
                                if filters[slot.index()].is_ratio_brickwall()
                        )
                    })
                });
            if !voice_requires_sample_routes
                && oscillator_controls_only
                && (filter_block.is_none() || dynamic_filter_gain_only)
            {
                let controls = controls.expect("oscillator controls must be present");
                let gain_mask = controls[0].mask;
                if gain_mask.count_ones() == 1
                    && controls.iter().all(|control| {
                        control.mask == gain_mask && control.gain_only_mask == gain_mask
                    })
                {
                    let slot = gain_mask.trailing_zeros() as usize;
                    let gains = std::array::from_fn(|frame| {
                        let control = controls[frame].slots[slot];
                        (control.left_gain, control.right_gain)
                    });
                    return self.render_ordered_oscillator_groups_block(
                        settings,
                        sample_rate,
                        active,
                        groups,
                        group_count,
                        filters,
                        filter_block,
                        Some((slot, &gains)),
                    );
                }
            }
            if (!voice_requires_sample_routes || combined_voice_filter)
                && (generator_routes.gain_block_eligible()
                    || gain_only_controls
                    || oscillator_controls_only)
                && generator_routes.ratio_filter_source_mask(filters) == 0
                && (filter_block.is_none() || dynamic_filter_gain_only)
            {
                return self.render_generator_gain_grouped_block(
                    settings,
                    sample_rate,
                    active,
                    groups,
                    group_count,
                    controls,
                    filters,
                    filter_block,
                    aux,
                    generator_routes,
                    generator_route_amounts,
                    voice_modulation,
                );
            }
            let mixed_route = (settings.fast_audio_rate_modulation
                && controls.is_none()
                && voice_modulation.is_none()
                && generator_route_amounts.is_none()
                && groups.iter().take(group_count).all(|group| {
                    group
                        .modules()
                        .iter()
                        .all(|module| matches!(module, GeneratorRtModule::Oscillator(_)))
                }))
            .then(|| generator_routes.mixed_phase_gain_routes())
            .flatten();
            if !voice_requires_sample_routes
                && filter_block.is_none()
                && (generator_routes.phase_block_eligible()
                    || generator_routes.pitch_block_eligible()
                    || mixed_route.is_some())
            {
                return self.render_generator_time_grouped_block(
                    settings,
                    sample_rate,
                    active,
                    groups,
                    group_count,
                    filters,
                    generator_routes,
                    generator_route_amounts,
                    mixed_route,
                );
            }
            let neutral = StructuralOscillatorFrameControl::default();
            let static_ratio_bands = structural_ratio_bands(groups, group_count, filters);
            let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
            for frame in 0..SAMPLES {
                let mut stems = [(0.0_f32, 0.0_f32); MAX_OUTPUT_PAIRS];
                let frame_filters = filter_block.map_or(filters, |block| &block[frame]);
                let base_control = controls.map_or(&neutral, |controls| &controls[frame]);
                let mut voice_structural = crate::StructuralModulationFrame::default();
                let voice_values = voice_requires_sample_routes.then(|| {
                    let (program, routes) =
                        voice_modulation.expect("voice routes need their LFO program");
                    let values = self.modulation.next(program);
                    routes.evaluate(values, &mut voice_structural);
                    values
                });
                let voice_control;
                let frame_control = if voice_structural.oscillator_mask != 0 {
                    voice_control = merge_voice_structural_block_control(
                        base_control,
                        &voice_structural,
                        active,
                    );
                    &voice_control
                } else {
                    base_control
                };
                let mut voice_filters;
                let frame_filters = if voice_structural.filter_mask != 0 {
                    voice_filters = *frame_filters;
                    let mut mask = voice_structural.filter_mask;
                    while mask != 0 {
                        let slot = mask.trailing_zeros() as usize;
                        mask &= mask - 1;
                        voice_filters[slot] = generator_filter_coefficient(
                            frame_filters[slot],
                            voice_structural.filters[slot],
                        );
                    }
                    &voice_filters
                } else {
                    frame_filters
                };
                let mut route_amount =
                    generator_route_amounts.map(|(target, amounts)| (target, amounts[frame]));
                let mut voice_generator_routes;
                let generator_routes =
                    if let Some(values) = voice_values.filter(|_| voice_has_generator_depth) {
                        voice_generator_routes = *generator_routes;
                        let (_, routes) = voice_modulation.expect("voice values need their routes");
                        routes.apply_generator_depth(
                            values,
                            &mut voice_generator_routes,
                            route_amount.take(),
                        );
                        &voice_generator_routes
                    } else {
                        generator_routes
                    };
                let ratio_filter_sources = generator_routes.ratio_filter_source_mask(frame_filters);
                let mut routed_ratio_filters;
                let frame_filters = if ratio_filter_sources == 0 {
                    frame_filters
                } else {
                    routed_ratio_filters = *frame_filters;
                    for slot in 0..MAX_FILTERS {
                        if routed_ratio_filters[slot].is_ratio_brickwall() {
                            routed_ratio_filters[slot] = generator_filter_coefficient(
                                routed_ratio_filters[slot],
                                generator_routes.filter_delta(
                                    slot,
                                    &self.generator_feedback_taps,
                                    route_amount,
                                ),
                            );
                        }
                    }
                    &routed_ratio_filters
                };
                let dynamic_ratio_bands;
                let ratio_bands = if filter_block.is_some()
                    || voice_structural.filter_mask != 0
                    || ratio_filter_sources != 0
                {
                    dynamic_ratio_bands =
                        structural_ratio_bands(groups, group_count, frame_filters);
                    &dynamic_ratio_bands
                } else {
                    &static_ratio_bands
                };
                self.render_ordered_oscillator_groups(
                    active,
                    settings,
                    sample_rate,
                    frame_control,
                    &mut stems,
                    groups,
                    group_count,
                    frame_filters,
                    ratio_bands,
                    aux,
                    generator_routes,
                    route_amount,
                );
                for group in 0..group_count.min(MAX_OUTPUT_PAIRS) {
                    output[group][frame] = stems[group];
                }
            }
            return output;
        }
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
        let voice_phase_mod = voice_modulation.map(|(program, routes)| {
            let mut values = [[0.0_f32; SAMPLES]; MAX_OSCILLATORS];
            for frame in 0..SAMPLES {
                routes.accumulate_phase_mod_depth(
                    self.modulation.next(program),
                    frame,
                    &mut values,
                );
            }
            values
        });
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        let mut oscillator_outputs = [[0.0_f32; SAMPLES]; MAX_OSCILLATORS];
        let mut rendered = 0_u32;
        for (group_index, group) in groups.iter().take(group_count).enumerate() {
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
                    let phase_step = (self.structural_base_step(oscillator, base_step)
                        * oscillator.pitch_ratio)
                        .min(0.45);
                    let mut frame = 0;
                    while frame + 8 <= SAMPLES {
                        let phase_mod_amount: [f32; 8] = std::array::from_fn(|lane| {
                            let frame = frame + lane;
                            (controls
                                .and_then(|controls| controls[frame].get(slot))
                                .map_or(oscillator.phase_mod_amount, |control| {
                                    control.phase_mod_amount
                                })
                                + voice_phase_mod
                                    .as_ref()
                                    .map_or(0.0, |values| values[slot][frame]))
                            .clamp(-1.0, 1.0)
                        });
                        let source: [f32; 8] = std::array::from_fn(|lane| {
                            oscillator
                                .phase_mod_source
                                .checked_sub(1)
                                .filter(|source| rendered & (1 << *source) != 0)
                                .map_or(0.0, |source| {
                                    oscillator_outputs[usize::from(source)][frame + lane]
                                })
                        });
                        let phase_modulation: [f32; 8] = std::array::from_fn(|lane| {
                            (source[lane]
                                * if oscillator.modulation_mode == GeneratorModMode::Phase {
                                    phase_mod_amount[lane]
                                } else {
                                    0.0
                                })
                            .clamp(-1.0, 1.0)
                        });
                        let samples = generate_shape_time8(
                            &mut self.oscillator_bank.oscillators[slot][0],
                            shape,
                            phase_step,
                            phase_modulation,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        for lane in 0..8 {
                            let mut left = samples[lane] * oscillator.left_gain;
                            let mut right = samples[lane] * oscillator.right_gain;
                            apply_generator_modulation(
                                oscillator.modulation_mode,
                                source[lane],
                                phase_mod_amount[lane],
                                &mut left,
                                &mut right,
                            );
                            oscillator_outputs[slot][frame + lane] = generator_modulation_tap(
                                left,
                                right,
                                oscillator.left_gain,
                                oscillator.right_gain,
                            );
                            output[group_index][frame + lane].0 += left;
                            output[group_index][frame + lane].1 += right;
                        }
                        frame += 8;
                    }
                    while frame < SAMPLES {
                        let phase_mod_amount = (controls
                            .and_then(|controls| controls[frame].get(slot))
                            .map_or(oscillator.phase_mod_amount, |control| {
                                control.phase_mod_amount
                            })
                            + voice_phase_mod
                                .as_ref()
                                .map_or(0.0, |values| values[slot][frame]))
                        .clamp(-1.0, 1.0);
                        let source = oscillator
                            .phase_mod_source
                            .checked_sub(1)
                            .filter(|source| rendered & (1 << *source) != 0)
                            .map_or(0.0, |source| oscillator_outputs[usize::from(source)][frame]);
                        let phase_mod = (source
                            * if oscillator.modulation_mode == GeneratorModMode::Phase {
                                phase_mod_amount
                            } else {
                                0.0
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
                        let mut left = sample * oscillator.left_gain;
                        let mut right = sample * oscillator.right_gain;
                        apply_generator_modulation(
                            oscillator.modulation_mode,
                            source,
                            phase_mod_amount,
                            &mut left,
                            &mut right,
                        );
                        oscillator_outputs[slot][frame] = generator_modulation_tap(
                            left,
                            right,
                            oscillator.left_gain,
                            oscillator.right_gain,
                        );
                        output[group_index][frame].0 += left;
                        output[group_index][frame].1 += right;
                        frame += 1;
                    }
                    rendered |= 1 << slot;
                    continue;
                }
                for frame in 0..SAMPLES {
                    let phase_mod_amount = (controls
                        .and_then(|controls| controls[frame].get(slot))
                        .map_or(oscillator.phase_mod_amount, |control| {
                            control.phase_mod_amount
                        })
                        + voice_phase_mod
                            .as_ref()
                            .map_or(0.0, |values| values[slot][frame]))
                    .clamp(-1.0, 1.0);
                    let source = oscillator
                        .phase_mod_source
                        .checked_sub(1)
                        .filter(|source| rendered & (1 << *source) != 0)
                        .map_or(0.0, |source| oscillator_outputs[usize::from(source)][frame]);
                    let phase_mod = (source
                        * if oscillator.modulation_mode == GeneratorModMode::Phase {
                            phase_mod_amount
                        } else {
                            0.0
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
                        None,
                        phase_mod,
                        &mut left,
                        &mut right,
                    );
                    apply_generator_modulation(
                        oscillator.modulation_mode,
                        source,
                        phase_mod_amount,
                        &mut left,
                        &mut right,
                    );
                    oscillator_outputs[slot][frame] = generator_modulation_tap(
                        left,
                        right,
                        oscillator.left_gain,
                        oscillator.right_gain,
                    );
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

    #[allow(
        clippy::too_many_arguments,
        reason = "the muted-source fast path keeps its realtime eligibility local"
    )]
    fn render_fast_muted_source_tap<const SAMPLES: usize>(
        &mut self,
        fast_source_mask: u32,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        ratio_filtered: bool,
        tap: &mut [f32; SAMPLES],
    ) -> bool {
        let base_step = self.structural_base_step(oscillator, base_step);
        let voices = usize::from(oscillator.render_voices);
        if fast_source_mask & (1 << slot) == 0
            || voices < 2
            || ratio_filtered
            || oscillator.engine != OscillatorEngineKind::Va
            || oscillator.custom_mix > f32::EPSILON
            || oscillator.positioned_wave
            || oscillator.phase_warp.active()
            || oscillator.jitter_active()
            || self.oscillator_bank.jitter_ratios[slot][..voices]
                .iter()
                .any(|ratio| ratio.to_bits() != 1.0_f32.to_bits())
        {
            return false;
        }

        self.advance_settled_structural_jitter_block::<SAMPLES>(slot, oscillator, sample_rate);
        let phase_delta = shortest_phase_delta(
            self.oscillator_bank.applied_phase_positions[slot],
            oscillator.phase_position,
        );
        if phase_delta != 0.0 {
            VaOscillator::offset_phases(
                &mut self.oscillator_bank.oscillators[slot][..voices],
                phase_delta,
            );
            self.oscillator_bank.applied_phase_positions[slot] = oscillator.phase_position;
        }

        let phase_step =
            (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[0]).min(0.45);
        let level = ((oscillator.left_gain * oscillator.left_gain
            + oscillator.right_gain * oscillator.right_gain)
            * 0.5)
            .sqrt();
        let lane = &mut self.oscillator_bank.oscillators[slot][0];
        let mut frame = 0;
        while frame + 8 <= SAMPLES {
            let samples = generate_shape_time8(
                lane,
                shape,
                phase_step,
                [0.0; 8],
                oscillator.pulse_width,
                settings.antialiasing,
            );
            for offset in 0..8 {
                tap[frame + offset] = samples[offset] * level;
            }
            frame += 8;
        }
        while frame < SAMPLES {
            tap[frame] = lane.generate_shape_step(
                shape,
                phase_step,
                oscillator.pulse_width,
                settings.antialiasing,
            ) * level;
            frame += 1;
        }
        for lane in 1..voices {
            let phase_step =
                (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[lane]).min(0.45);
            self.oscillator_bank.oscillators[slot][lane].advance_phase(phase_step * SAMPLES as f32);
        }
        true
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the fixed generator graph block keeps the render context allocation-free"
    )]
    fn render_generator_time_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        filters: &[FilterCoefficients; MAX_FILTERS],
        generator_routes: &GeneratorStructuralRouteFrame,
        generator_route_amounts: Option<(u8, &[f32])>,
        mixed_route: Option<(usize, usize, f32, crate::OscillatorControl, f32)>,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        let phase_route = generator_routes.phase_block_eligible() || mixed_route.is_some();
        let settled = active.entries().iter().all(|entry| {
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            let voices = usize::from(oscillator.render_voices);
            if oscillator.engine != OscillatorEngineKind::Va {
                !generator_routes.target_active(slot)
            } else {
                oscillator.custom_mix <= f32::EPSILON
                    && !oscillator.phase_warp.active()
                    && (voices == 1 || !oscillator.jitter_active())
                    && self.oscillator_bank.jitter_ratios[slot][..voices]
                        .iter()
                        .all(|ratio| ratio.to_bits() == 1.0_f32.to_bits())
            }
        }) && groups
            .iter()
            .take(group_count)
            .flat_map(GeneratorRtGroup::modules)
            .all(|module| match *module {
                GeneratorRtModule::Oscillator(_) => true,
                GeneratorRtModule::Filter(slot) => !filters[slot.index()].is_ratio_brickwall(),
                GeneratorRtModule::Aux(_) => false,
            });
        if !settled {
            let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
            let control = StructuralOscillatorFrameControl::default();
            let aux = [AuxConfig::default(); MAX_AUX_MODULES];
            let ratio_bands = structural_ratio_bands(groups, group_count, filters);
            for frame in 0..SAMPLES {
                let mut stems = [(0.0_f32, 0.0_f32); MAX_OUTPUT_PAIRS];
                self.render_ordered_oscillator_groups(
                    active,
                    settings,
                    sample_rate,
                    &control,
                    &mut stems,
                    groups,
                    group_count,
                    filters,
                    &ratio_bands,
                    &aux,
                    generator_routes,
                    generator_route_amounts.map(|(target, amounts)| (target, amounts[frame])),
                );
                for group in 0..group_count {
                    output[group][frame] = stems[group];
                }
            }
            return output;
        }

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
        let mut oscillator_audio = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OSCILLATORS];
        let mut oscillator_taps = [[0.0_f32; SAMPLES]; MAX_OSCILLATORS];

        for &slot in generator_routes.order() {
            let slot = usize::from(slot);
            if active.mask & (1 << slot) == 0 {
                continue;
            }
            let base_oscillator = &active.entry(slot).current;
            let gain_target = mixed_route.is_some_and(|(_, target, _, _, _)| target == slot);
            let mut unity_oscillator = *base_oscillator;
            if gain_target {
                unity_oscillator.left_gain = 1.0;
                unity_oscillator.right_gain = 1.0;
            }
            let oscillator = if gain_target {
                &unity_oscillator
            } else {
                base_oscillator
            };
            let base_step = self.structural_base_step(oscillator, base_step);
            let shape =
                (oscillator.shape + Self::oscillator_timbre(oscillator, timbre)).clamp(0.0, 3.0);
            if oscillator.engine != OscillatorEngineKind::Va {
                let mut left = [f32x8::ZERO; SAMPLES];
                let mut right = [f32x8::ZERO; SAMPLES];
                self.accumulate_structural_oscillator_ratio_block(
                    slot,
                    oscillator,
                    settings,
                    sample_rate,
                    base_step,
                    shape,
                    None,
                    &mut left,
                    &mut right,
                );
                for frame in 0..SAMPLES {
                    let sample = (left[frame].reduce_add(), right[frame].reduce_add());
                    oscillator_audio[slot][frame] = sample;
                    if generator_routes.source_mask() & (1 << slot) != 0 {
                        oscillator_taps[slot][frame] = generator_modulation_tap(
                            sample.0,
                            sample.1,
                            oscillator.left_gain,
                            oscillator.right_gain,
                        );
                    }
                }
                continue;
            }
            let voices = usize::from(oscillator.render_voices);
            if self.render_fast_muted_source_tap(
                generator_routes.fast_source_mask(),
                slot,
                oscillator,
                settings,
                sample_rate,
                base_step,
                shape,
                false,
                &mut oscillator_taps[slot],
            ) {
                continue;
            }
            if voices == 1 {
                self.oscillator_bank.jitter_ratios[slot][0] = 1.0;
                self.oscillator_bank.jitter_steps[slot][0] = 0.0;
                self.oscillator_bank.jitter_remaining[slot] = 0;
            } else {
                self.advance_settled_structural_jitter_block::<SAMPLES>(
                    slot,
                    oscillator,
                    sample_rate,
                );
            }
            let initial_phase = self.oscillator_bank.applied_phase_positions[slot];
            if phase_route {
                let mut phase_modulation = [0.0_f32; SAMPLES];
                let phase_modulated = generator_routes.target_active(slot)
                    || shortest_phase_delta(
                        initial_phase,
                        oscillator.phase_position.rem_euclid(1.0),
                    ) != 0.0;
                if phase_modulated {
                    if let Some((source, target, amount, _, _)) = mixed_route
                        && target == slot
                    {
                        for frame in 0..SAMPLES {
                            phase_modulation[frame] = oscillator_taps[source][frame] * amount;
                        }
                    } else {
                        generator_routes.accumulate_phase_block(
                            slot,
                            &oscillator_taps,
                            generator_route_amounts,
                            &mut phase_modulation,
                        );
                    }
                    for modulation in &mut phase_modulation {
                        let target = (oscillator.phase_position + *modulation).rem_euclid(1.0);
                        *modulation = shortest_phase_delta(initial_phase, target);
                    }
                }
                let lane_pack_end = voices / 8 * 8;
                let lane_pack_end = if lane_pack_end == 0 {
                    0
                } else {
                    let first_narrow =
                        (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[0])
                            .min(0.45)
                            < 0.25;
                    if oscillator.lane_pitch_ratios[1..voices].iter().any(|ratio| {
                        ((base_step * oscillator.pitch_ratio * ratio).min(0.45) < 0.25)
                            != first_narrow
                    }) {
                        0
                    } else {
                        lane_pack_end
                    }
                };
                let dense_spline = lane_pack_end == voices
                    && voices >= 8
                    && shape == 2.0
                    && (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[0])
                        .min(0.45)
                        < 0.25
                    && matches!(
                        settings.antialiasing,
                        Antialiasing::Spline | Antialiasing::SplineOptimized
                    );
                let mut lane_output =
                    dense_spline.then(|| ([f32x8::ZERO; SAMPLES], [f32x8::ZERO; SAMPLES]));
                for lane in (0..lane_pack_end).step_by(8) {
                    let phase_steps = std::array::from_fn(|offset| {
                        (base_step
                            * oscillator.pitch_ratio
                            * oscillator.lane_pitch_ratios[lane + offset])
                            .min(0.45)
                    });
                    let left_gains: [f32; 8] = std::array::from_fn(|offset| {
                        oscillator.left_gain * oscillator.lane_left_gains[lane + offset]
                    });
                    let right_gains: [f32; 8] = std::array::from_fn(|offset| {
                        oscillator.right_gain * oscillator.lane_right_gains[lane + offset]
                    });
                    let oscillator_states =
                        &mut self.oscillator_bank.oscillators[slot][lane..lane + 8];
                    if let Some((left, right)) = &mut lane_output {
                        accumulate_spline_saw8_phase_modulated_lanes_block(
                            oscillator_states,
                            phase_steps,
                            phase_modulated.then_some(&phase_modulation),
                            settings.antialiasing == Antialiasing::SplineOptimized,
                            f32x8::from(left_gains),
                            f32x8::from(right_gains),
                            left,
                            right,
                        );
                    } else if shape == 2.0
                        && phase_steps[0] < 0.25
                        && matches!(
                            settings.antialiasing,
                            Antialiasing::Spline | Antialiasing::SplineOptimized
                        )
                    {
                        accumulate_spline_saw8_phase_modulated_block(
                            oscillator_states,
                            phase_steps,
                            &phase_modulation,
                            settings.antialiasing == Antialiasing::SplineOptimized,
                            left_gains,
                            right_gains,
                            &mut oscillator_audio[slot],
                        );
                    } else {
                        accumulate_shape8_phase_modulated_block(
                            oscillator_states,
                            shape,
                            phase_steps,
                            &phase_modulation,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            left_gains,
                            right_gains,
                            &mut oscillator_audio[slot],
                        );
                    }
                    for oscillator_state in oscillator_states {
                        oscillator_state.offset_phase(phase_modulation[SAMPLES - 1]);
                    }
                }
                if let Some((left, right)) = lane_output {
                    for frame in 0..SAMPLES {
                        oscillator_audio[slot][frame].0 += left[frame].reduce_add();
                        oscillator_audio[slot][frame].1 += right[frame].reduce_add();
                    }
                }
                let mut tail_start = lane_pack_end;
                while shape == 2.0
                    && tail_start + 4 <= voices
                    && matches!(
                        settings.antialiasing,
                        Antialiasing::Spline | Antialiasing::SplineOptimized
                    )
                {
                    let phase_steps: [f32; 4] = std::array::from_fn(|offset| {
                        (base_step
                            * oscillator.pitch_ratio
                            * oscillator.lane_pitch_ratios[tail_start + offset])
                            .min(0.45)
                    });
                    if phase_steps.iter().any(|step| *step >= 0.25) {
                        break;
                    }
                    let left_gains = std::array::from_fn(|offset| {
                        oscillator.left_gain * oscillator.lane_left_gains[tail_start + offset]
                    });
                    let right_gains = std::array::from_fn(|offset| {
                        oscillator.right_gain * oscillator.lane_right_gains[tail_start + offset]
                    });
                    let oscillator_states =
                        &mut self.oscillator_bank.oscillators[slot][tail_start..tail_start + 4];
                    accumulate_spline_saw4_phase_modulated_block(
                        oscillator_states,
                        phase_steps,
                        &phase_modulation,
                        settings.antialiasing == Antialiasing::SplineOptimized,
                        left_gains,
                        right_gains,
                        &mut oscillator_audio[slot],
                    );
                    for oscillator_state in oscillator_states {
                        oscillator_state.offset_phase(phase_modulation[SAMPLES - 1]);
                    }
                    tail_start += 4;
                }
                for lane in tail_start..voices {
                    let phase_step =
                        (base_step * oscillator.pitch_ratio * oscillator.lane_pitch_ratios[lane])
                            .min(0.45);
                    let left_gain = oscillator.left_gain * oscillator.lane_left_gains[lane];
                    let right_gain = oscillator.right_gain * oscillator.lane_right_gains[lane];
                    let oscillator_state = &mut self.oscillator_bank.oscillators[slot][lane];
                    let mut frame = 0;
                    while frame + 8 <= SAMPLES {
                        let modulation =
                            std::array::from_fn(|offset| phase_modulation[frame + offset]);
                        let samples = generate_shape_time8(
                            oscillator_state,
                            shape,
                            phase_step,
                            modulation,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        for offset in 0..8 {
                            oscillator_audio[slot][frame + offset].0 += samples[offset] * left_gain;
                            oscillator_audio[slot][frame + offset].1 +=
                                samples[offset] * right_gain;
                        }
                        frame += 8;
                    }
                    while frame < SAMPLES {
                        let modulation = phase_modulation[frame];
                        oscillator_state.offset_phase(modulation);
                        let sample = oscillator_state.generate_shape_step(
                            shape,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        oscillator_state.offset_phase(-modulation);
                        oscillator_audio[slot][frame].0 += sample * left_gain;
                        oscillator_audio[slot][frame].1 += sample * right_gain;
                        frame += 1;
                    }
                    oscillator_state.offset_phase(phase_modulation[SAMPLES - 1]);
                }
                self.oscillator_bank.applied_phase_positions[slot] =
                    (initial_phase + phase_modulation[SAMPLES - 1]).rem_euclid(1.0);
            } else {
                let phase_delta = shortest_phase_delta(initial_phase, oscillator.phase_position);
                if phase_delta != 0.0 {
                    for lane in &mut self.oscillator_bank.oscillators[slot][..voices] {
                        lane.offset_phase(phase_delta);
                    }
                    self.oscillator_bank.applied_phase_positions[slot] = oscillator.phase_position;
                }
                let pitch_modulated = generator_routes.target_active(slot);
                let pitch_ratios = if pitch_modulated {
                    let mut semitones = [0.0_f32; SAMPLES];
                    generator_routes.accumulate_pitch_block(
                        slot,
                        &oscillator_taps,
                        generator_route_amounts,
                        &mut semitones,
                    );
                    semitones.map(|semitones| {
                        (oscillator.pitch_ratio * fast_exp2(semitones.clamp(-48.0, 48.0) / 12.0))
                            .clamp(1.0 / 256.0, 256.0)
                    })
                } else {
                    [(oscillator.pitch_ratio * fast_exp2(0.0)).clamp(1.0 / 256.0, 256.0); SAMPLES]
                };
                if voices < 4 {
                    for lane in 0..voices {
                        let lane_ratio = oscillator.lane_pitch_ratios[lane];
                        let left_gain = oscillator.left_gain * oscillator.lane_left_gains[lane];
                        let right_gain = oscillator.right_gain * oscillator.lane_right_gains[lane];
                        let oscillator_state = &mut self.oscillator_bank.oscillators[slot][lane];
                        let mut frame = 0;
                        while frame + 8 <= SAMPLES {
                            let steps = std::array::from_fn(|offset| {
                                (base_step * pitch_ratios[frame + offset] * lane_ratio).min(0.45)
                            });
                            let samples = generate_shape_time8_steps(
                                oscillator_state,
                                shape,
                                steps,
                                [0.0; 8],
                                oscillator.pulse_width,
                                settings.antialiasing,
                            );
                            for offset in 0..8 {
                                oscillator_audio[slot][frame + offset].0 +=
                                    samples[offset] * left_gain;
                                oscillator_audio[slot][frame + offset].1 +=
                                    samples[offset] * right_gain;
                            }
                            frame += 8;
                        }
                        while frame < SAMPLES {
                            let step = (base_step * pitch_ratios[frame] * lane_ratio).min(0.45);
                            let sample = oscillator_state.generate_shape_step(
                                shape,
                                step,
                                oscillator.pulse_width,
                                settings.antialiasing,
                            );
                            oscillator_audio[slot][frame].0 += sample * left_gain;
                            oscillator_audio[slot][frame].1 += sample * right_gain;
                            frame += 1;
                        }
                    }
                } else {
                    let mut left = [f32x8::ZERO; SAMPLES];
                    let mut right = [f32x8::ZERO; SAMPLES];
                    let packs = voices / 8;
                    for pack in 0..packs {
                        let first = pack * 8;
                        let lane_ratios = f32x8::from(std::array::from_fn(|lane| {
                            oscillator.lane_pitch_ratios[first + lane]
                        }));
                        let left_gain = f32x8::from(std::array::from_fn(|lane| {
                            oscillator.left_gain * oscillator.lane_left_gains[first + lane]
                        }));
                        let right_gain = f32x8::from(std::array::from_fn(|lane| {
                            oscillator.right_gain * oscillator.lane_right_gains[first + lane]
                        }));
                        let lanes = &mut self.oscillator_bank.oscillators[slot][first..first + 8];
                        if settings.fast_audio_rate_modulation && !pitch_modulated {
                            let phase_step = (f32x8::splat(base_step * pitch_ratios[0])
                                * lane_ratios)
                                .fast_min(f32x8::splat(0.45));
                            if shape == 2.0 {
                                accumulate_saw8_block_static_gains(
                                    lanes,
                                    phase_step,
                                    f32x8::ZERO,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    settings.antialiasing,
                                );
                            } else {
                                accumulate_shape8_block_constant(
                                    lanes,
                                    phase_step,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    shape,
                                    oscillator.pulse_width,
                                    settings.antialiasing,
                                );
                            }
                        } else {
                            let phase_steps = std::array::from_fn(|frame| {
                                (f32x8::splat(base_step * pitch_ratios[frame]) * lane_ratios)
                                    .fast_min(f32x8::splat(0.45))
                            });
                            if settings.fast_audio_rate_modulation
                                && pitch_modulated
                                && shape == 2.0
                                && SAMPLES > 1
                            {
                                let first_step = phase_steps[0];
                                let step_sum = phase_steps
                                    .iter()
                                    .copied()
                                    .fold(f32x8::ZERO, |sum, step| sum + step);
                                let final_step =
                                    step_sum * f32x8::splat(2.0 / SAMPLES as f32) - first_step;
                                let phase_step_delta =
                                    (final_step - first_step) / f32x8::splat((SAMPLES - 1) as f32);
                                accumulate_saw8_block_static_gains_narrow_spline(
                                    lanes,
                                    first_step - phase_step_delta,
                                    phase_step_delta,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    settings.antialiasing,
                                );
                            } else if shape == 2.0 {
                                accumulate_saw8_block(
                                    lanes,
                                    phase_steps,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    settings.antialiasing,
                                );
                            } else {
                                accumulate_shape8_block_steps(
                                    lanes,
                                    phase_steps,
                                    left_gain,
                                    right_gain,
                                    &mut left,
                                    &mut right,
                                    shape,
                                    oscillator.pulse_width,
                                    settings.antialiasing,
                                    PhaseWarpMode::None,
                                    0.0,
                                );
                            }
                        }
                    }
                    let mut tail = packs * 8;
                    if voices - tail >= 4 {
                        let phase_steps = std::array::from_fn(|frame| {
                            f32x4::from(std::array::from_fn(|lane| {
                                (base_step
                                    * pitch_ratios[frame]
                                    * oscillator.lane_pitch_ratios[tail + lane])
                                    .min(0.45)
                            }))
                        });
                        let left_gain = f32x4::from(std::array::from_fn(|lane| {
                            oscillator.left_gain * oscillator.lane_left_gains[tail + lane]
                        }));
                        let right_gain = f32x4::from(std::array::from_fn(|lane| {
                            oscillator.right_gain * oscillator.lane_right_gains[tail + lane]
                        }));
                        let lanes = &mut self.oscillator_bank.oscillators[slot][tail..tail + 4];
                        if shape == 2.0 {
                            accumulate_saw4_block(
                                lanes,
                                phase_steps,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                settings.antialiasing,
                            );
                        } else {
                            accumulate_shape4_block_steps(
                                lanes,
                                phase_steps,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                shape,
                                oscillator.pulse_width,
                                settings.antialiasing,
                                PhaseWarpMode::None,
                                0.0,
                            );
                        }
                        tail += 4;
                    }
                    for lane in tail..voices {
                        let lane_ratio = oscillator.lane_pitch_ratios[lane];
                        for frame in 0..SAMPLES {
                            let step = (base_step * pitch_ratios[frame] * lane_ratio).min(0.45);
                            let sample = self.oscillator_bank.oscillators[slot][lane]
                                .generate_shape_step(
                                    shape,
                                    step,
                                    oscillator.pulse_width,
                                    settings.antialiasing,
                                );
                            left[frame] += f32x8::splat(
                                sample
                                    * oscillator.left_gain
                                    * oscillator.lane_left_gains[lane]
                                    * 0.125,
                            );
                            right[frame] += f32x8::splat(
                                sample
                                    * oscillator.right_gain
                                    * oscillator.lane_right_gains[lane]
                                    * 0.125,
                            );
                        }
                    }
                    for frame in 0..SAMPLES {
                        oscillator_audio[slot][frame] =
                            (left[frame].reduce_add(), right[frame].reduce_add());
                    }
                }
            }
            if gain_target {
                let (source, _, _, control, amount) =
                    mixed_route.expect("mixed gain target must have its route");
                let base_gain_position = || {
                    let left_power = base_oscillator.left_gain * base_oscillator.left_gain;
                    let right_power = base_oscillator.right_gain * base_oscillator.right_gain;
                    (
                        (left_power + right_power).sqrt() * std::f32::consts::FRAC_1_SQRT_2,
                        (right_power - left_power) / (right_power + left_power).max(f32::EPSILON),
                    )
                };
                match control {
                    crate::OscillatorControl::Level => {
                        let (base_level, base_pan) = base_gain_position();
                        let base_pan = base_pan.clamp(-1.0, 1.0);
                        let left_pan_gain = (1.0 - base_pan).sqrt();
                        let right_pan_gain = (1.0 + base_pan).sqrt();
                        for frame in 0..SAMPLES {
                            let delta = oscillator_taps[source][frame] * amount;
                            let (left_gain, right_gain) = if delta == 0.0 {
                                (base_oscillator.left_gain, base_oscillator.right_gain)
                            } else {
                                let level = (base_level + delta).clamp(0.0, 1.0);
                                (level * left_pan_gain, level * right_pan_gain)
                            };
                            oscillator_audio[slot][frame].0 *= left_gain;
                            oscillator_audio[slot][frame].1 *= right_gain;
                        }
                    }
                    crate::OscillatorControl::Pan => {
                        let (base_level, base_pan) = base_gain_position();
                        let level = base_level.clamp(0.0, 1.0);
                        for frame in 0..SAMPLES {
                            let delta = oscillator_taps[source][frame] * amount;
                            let (left_gain, right_gain) = if delta == 0.0 {
                                (base_oscillator.left_gain, base_oscillator.right_gain)
                            } else {
                                let pan = (base_pan + delta).clamp(-1.0, 1.0);
                                (level * (1.0 - pan).sqrt(), level * (1.0 + pan).sqrt())
                            };
                            oscillator_audio[slot][frame].0 *= left_gain;
                            oscillator_audio[slot][frame].1 *= right_gain;
                        }
                    }
                    crate::OscillatorControl::RingModAmount => {
                        let wet = amount.abs();
                        let dry = 1.0 - wet;
                        let signed_wet = amount.signum() * wet;
                        for frame in 0..SAMPLES {
                            let ring_gain = dry + oscillator_taps[source][frame] * signed_wet;
                            oscillator_audio[slot][frame].0 *=
                                base_oscillator.left_gain * ring_gain;
                            oscillator_audio[slot][frame].1 *=
                                base_oscillator.right_gain * ring_gain;
                        }
                    }
                    _ => unreachable!("mixed route must contain a gain control"),
                }
            }
            if generator_routes.source_mask() & (1 << slot) != 0 {
                for frame in 0..SAMPLES {
                    oscillator_taps[slot][frame] = generator_modulation_tap(
                        oscillator_audio[slot][frame].0,
                        oscillator_audio[slot][frame].1,
                        oscillator.left_gain,
                        oscillator.right_gain,
                    );
                }
            }
        }

        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for (group_index, group) in groups.iter().take(group_count).enumerate() {
            if self.group_active_mask & (1 << group_index) == 0
                || group.oscillator_mask() & generator_routes.fast_source_mask() != 0
            {
                continue;
            }
            for module in group.modules() {
                match *module {
                    GeneratorRtModule::Oscillator(slot) => {
                        for frame in 0..SAMPLES {
                            output[group_index][frame].0 += oscillator_audio[slot.index()][frame].0;
                            output[group_index][frame].1 += oscillator_audio[slot.index()][frame].1;
                        }
                    }
                    GeneratorRtModule::Filter(slot) => {
                        let slot = slot.index();
                        let coefficients = filters[slot];
                        if coefficients.is_phaser() {
                            self.filters[slot].prepare_phaser(coefficients);
                        }
                        for sample in &mut output[group_index] {
                            *sample = if coefficients.is_phaser() {
                                self.filters[slot].process_prepared_phaser(
                                    coefficients,
                                    sample.0,
                                    sample.1,
                                )
                            } else {
                                self.filters[slot].process(coefficients, sample.0, sample.1)
                            };
                        }
                    }
                    GeneratorRtModule::Aux(_) => {
                        debug_assert!(false, "generator time block excludes aux modules");
                    }
                }
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

    #[allow(
        clippy::too_many_arguments,
        reason = "the fixed generator graph block keeps the render context allocation-free"
    )]
    fn render_generator_gain_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        active: &ActiveOscillatorRenderSet,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        controls: Option<&[StructuralOscillatorFrameControl]>,
        filters: &[FilterCoefficients; MAX_FILTERS],
        filter_block: Option<&[[FilterCoefficients; MAX_FILTERS]]>,
        aux: &[AuxConfig; MAX_AUX_MODULES],
        generator_routes: &GeneratorStructuralRouteFrame,
        route_amounts: Option<(u8, &[f32])>,
        voice_modulation: Option<(&VoiceLfoProgram, &VoiceStructuralRouteFrame)>,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
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

        let mut ratio_bands = [None; MAX_OSCILLATORS];
        let mut non_oscillator_group_mask = 0_u32;
        for group in groups.iter().take(group_count) {
            let modules = group.modules();
            if modules
                .iter()
                .any(|module| !matches!(module, GeneratorRtModule::Oscillator(_)))
            {
                non_oscillator_group_mask |= group.oscillator_mask();
            }
            for (module_index, module) in modules.iter().enumerate() {
                if let GeneratorRtModule::Oscillator(slot) = *module {
                    ratio_bands[slot.index()] =
                        downstream_ratio_brickwall(modules, module_index, filters);
                }
            }
        }

        let mut oscillator_audio = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OSCILLATORS];
        let mut oscillator_taps = [[0.0_f32; SAMPLES]; MAX_OSCILLATORS];
        let single_filter_route = generator_routes.single_filter_route();
        let combined_filter_modulation =
            single_filter_route.and_then(|(route_index, generator_source, slot, _, control)| {
                let (program, routes) = voice_modulation?;
                let (lfo_source, base, depth, target) =
                    routes.combined_generator_child(route_index)?;
                matches!(
                    target,
                    crate::ResolvedModularTarget::Filter {
                        slot: target_slot,
                        control: target_control,
                    } if target_slot == slot && target_control == control
                )
                .then(|| {
                    let values: [f32; SAMPLES] = std::array::from_fn(|_| {
                        self.modulation.next(program)[usize::from(lfo_source)]
                    });
                    (
                        route_index,
                        generator_source,
                        usize::from(slot),
                        control,
                        base,
                        depth,
                        values,
                    )
                })
            });
        let single_gain_route = if controls.is_none() {
            generator_routes.single_gain_route()
        } else {
            None
        };
        let controlled_mask = controls.map_or(0, |controls| {
            controls.iter().fold(0, |mask, control| mask | control.mask)
        });
        let fast_source_mask =
            generator_routes.fast_source_mask() & !controlled_mask & !non_oscillator_group_mask;
        for &slot in generator_routes.order() {
            let slot = usize::from(slot);
            if active.mask & (1 << slot) == 0 {
                continue;
            }
            let oscillator = &active.entry(slot).current;
            let mut unity = *oscillator;
            unity.left_gain = 1.0;
            unity.right_gain = 1.0;
            let shape =
                (oscillator.shape + Self::oscillator_timbre(oscillator, timbre)).clamp(0.0, 3.0);
            if self.render_fast_muted_source_tap(
                fast_source_mask,
                slot,
                oscillator,
                settings,
                sample_rate,
                base_step,
                shape,
                ratio_bands[slot].is_some(),
                &mut oscillator_taps[slot],
            ) {
                continue;
            }
            let mut block_left = [f32x8::ZERO; SAMPLES];
            let mut block_right = [f32x8::ZERO; SAMPLES];
            let non_gain_controlled = controls.is_some_and(|controls| {
                controls.iter().any(|control| {
                    control.mask & (1 << slot) != 0 && control.gain_only_mask & (1 << slot) == 0
                })
            });
            if non_gain_controlled {
                for frame in 0..SAMPLES {
                    let absolute = controls.and_then(|controls| controls[frame].get(slot));
                    let mut left = 0.0;
                    let mut right = 0.0;
                    self.accumulate_structural_oscillator(
                        slot,
                        slot,
                        &unity,
                        absolute,
                        settings,
                        sample_rate,
                        base_step,
                        absolute.map_or(shape, |control| control.shape),
                        ratio_bands[slot],
                        0.0,
                        &mut left,
                        &mut right,
                    );
                    block_left[frame] = f32x8::from([left, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
                    block_right[frame] = f32x8::from([right, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
                }
            } else {
                self.accumulate_structural_oscillator_ratio_block(
                    slot,
                    &unity,
                    settings,
                    sample_rate,
                    base_step,
                    shape,
                    ratio_bands[slot],
                    &mut block_left,
                    &mut block_right,
                );
            }
            let needs_tap = generator_routes.source_mask() & (1 << slot) != 0;
            let base_gain_position = single_gain_route
                .filter(|&(_, _, target, control, _)| {
                    target == slot
                        && matches!(
                            control,
                            crate::OscillatorControl::Level | crate::OscillatorControl::Pan
                        )
                })
                .map(|_| {
                    let left_power = oscillator.left_gain * oscillator.left_gain;
                    let right_power = oscillator.right_gain * oscillator.right_gain;
                    (
                        (left_power + right_power).sqrt() * std::f32::consts::FRAC_1_SQRT_2,
                        (right_power - left_power) / (right_power + left_power).max(f32::EPSILON),
                    )
                });
            for frame in 0..SAMPLES {
                let base_absolute = controls.and_then(|controls| controls[frame].get(slot));
                let route_amount = route_amounts.map(|(route, amounts)| (route, amounts[frame]));
                let (left_gain, right_gain, ring_gain) =
                    if let Some((route_index, source, target, control, amount)) = single_gain_route
                    {
                        if target == slot {
                            apply_generator_gain_route(
                                oscillator.left_gain,
                                oscillator.right_gain,
                                base_gain_position,
                                oscillator_taps[source][frame],
                                control,
                                route_amount
                                    .filter(|(route, _)| *route == route_index)
                                    .map_or(amount, |(_, amount)| amount),
                            )
                        } else {
                            (oscillator.left_gain, oscillator.right_gain, 1.0)
                        }
                    } else {
                        let mut generator_delta = crate::StructuralOscillatorDelta::default();
                        let (generator_active, ring_gain) = generator_routes
                            .accumulate_block_frame(
                                slot,
                                frame,
                                &oscillator_taps,
                                route_amount,
                                &mut generator_delta,
                            );
                        let generator_absolute = generator_active.then(|| {
                            let absolute = base_absolute.copied().unwrap_or(
                                StructuralOscillatorAbsoluteControl {
                                    shape: oscillator.shape,
                                    pulse_width: oscillator.pulse_width,
                                    pitch_ratio: oscillator.pitch_ratio,
                                    phase_position: oscillator.phase_position,
                                    phase_warp_amount: oscillator.phase_warp.amount,
                                    phase_mod_amount: 0.0,
                                    left_gain: oscillator.left_gain,
                                    right_gain: oscillator.right_gain,
                                    unison_jitter: oscillator.unison_jitter,
                                    unison_rate: oscillator.jitter_rate_hz,
                                    stereo_x: 0.0,
                                    stereo_y: 0.0,
                                    grain_tune: 0.0,
                                    grain_stereo: 0.0,
                                    rich_balance: 0.0,
                                    rich_formant: 0.0,
                                    rich_air: 0.0,
                                    rich_diffuse: 0.0,
                                    rich_dynamic: 0.0,
                                },
                            );
                            apply_voice_structural_delta(
                                absolute,
                                generator_delta,
                                !oscillator.positioned_wave,
                            )
                        });
                        let absolute = generator_absolute.as_ref().or(base_absolute);
                        (
                            absolute.map_or(oscillator.left_gain, |control| control.left_gain),
                            absolute.map_or(oscillator.right_gain, |control| control.right_gain),
                            ring_gain,
                        )
                    };
                let left = block_left[frame].reduce_add() * left_gain * ring_gain;
                let right = block_right[frame].reduce_add() * right_gain * ring_gain;
                oscillator_audio[slot][frame] = (left, right);
                if needs_tap {
                    oscillator_taps[slot][frame] =
                        generator_modulation_tap(left, right, left_gain, right_gain);
                }
            }
        }

        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for (group_index, group) in groups.iter().take(group_count).enumerate() {
            if self.group_active_mask & (1 << group_index) == 0
                || group.oscillator_mask() & fast_source_mask != 0
            {
                continue;
            }
            for module in group.modules() {
                match *module {
                    GeneratorRtModule::Oscillator(slot) => {
                        for frame in 0..SAMPLES {
                            output[group_index][frame].0 += oscillator_audio[slot.index()][frame].0;
                            output[group_index][frame].1 += oscillator_audio[slot.index()][frame].1;
                        }
                    }
                    GeneratorRtModule::Filter(slot) => {
                        let slot = slot.index();
                        if let Some((_, _, route_slot, control, _, _, _)) =
                            &combined_filter_modulation
                            && *route_slot == slot
                        {
                            if filters[slot].is_phaser()
                                && matches!(
                                    control,
                                    crate::FilterControl::Resonance | crate::FilterControl::Shape
                                )
                            {
                                self.filters[slot].prepare_phaser(filters[slot]);
                            } else if filters[slot].is_svf()
                                && *control == crate::FilterControl::Resonance
                            {
                                self.filters[slot].prepare_svf(filters[slot]);
                            }
                        }
                        for frame in 0..SAMPLES {
                            let base =
                                filter_block.map_or(filters[slot], |block| block[frame][slot]);
                            let combined = combined_filter_modulation.as_ref().and_then(
                                |(
                                    route_index,
                                    generator_source,
                                    route_slot,
                                    control,
                                    child_amount,
                                    depth_amount,
                                    values,
                                )| {
                                    (*route_slot == slot).then(|| {
                                        let child = values[frame];
                                        (
                                            *route_index,
                                            *control,
                                            child.mul_add(
                                                *child_amount,
                                                oscillator_taps[usize::from(*generator_source)]
                                                    [frame]
                                                    * child
                                                    * *depth_amount,
                                            ),
                                        )
                                    })
                                },
                            );
                            if let Some((_, control, value)) = combined {
                                let (left, right) = output[group_index][frame];
                                output[group_index][frame] = if base.is_phaser()
                                    && control == crate::FilterControl::Resonance
                                {
                                    self.filters[slot].process_prepared_phaser_resonance(
                                        &base,
                                        value * 4.0,
                                        left,
                                        right,
                                    )
                                } else if base.is_phaser() && control == crate::FilterControl::Shape
                                {
                                    self.filters[slot]
                                        .process_prepared_phaser_shape(&base, value, left, right)
                                } else if base.is_svf()
                                    && control == crate::FilterControl::Resonance
                                {
                                    self.filters[slot].process_prepared_svf_resonance(
                                        &base,
                                        value * 4.0,
                                        left,
                                        right,
                                    )
                                } else if base.is_scream()
                                    && control == crate::FilterControl::Resonance
                                {
                                    self.filters[slot].process_scream_resonance(
                                        &base,
                                        value * 4.0,
                                        left,
                                        right,
                                    )
                                } else if base.is_scream() && control == crate::FilterControl::Slope
                                {
                                    self.filters[slot]
                                        .process_scream_slope(&base, value, left, right)
                                } else if base.is_scream() && control == crate::FilterControl::Morph
                                {
                                    self.filters[slot]
                                        .process_scream_morph(&base, value, left, right)
                                } else {
                                    let coefficients = match control {
                                        crate::FilterControl::Cutoff => {
                                            base.modulated_cutoff(value * 4.0)
                                        }
                                        crate::FilterControl::Resonance => {
                                            base.modulated_resonance(value * 4.0)
                                        }
                                        crate::FilterControl::Slope => base.modulated_slope(value),
                                        crate::FilterControl::Morph => base.modulated_morph(value),
                                        crate::FilterControl::Shape => base.modulated_shape(value),
                                    };
                                    self.filters[slot].process(coefficients, left, right)
                                };
                                continue;
                            }
                            let coefficients = if !generator_routes.filter_routes_active() {
                                base
                            } else if let Some((route_index, source, route_slot, amount, control)) =
                                single_filter_route
                                && usize::from(route_slot) == slot
                            {
                                let value = oscillator_taps[usize::from(source)][frame]
                                    * route_amounts
                                        .filter(|(target, _)| *target == route_index)
                                        .map_or(amount, |(_, amounts)| amounts[frame])
                                        .clamp(-1.0, 1.0);
                                match control {
                                    crate::FilterControl::Cutoff => {
                                        base.modulated_cutoff(value * 4.0)
                                    }
                                    crate::FilterControl::Resonance => {
                                        base.modulated_resonance(value * 4.0)
                                    }
                                    crate::FilterControl::Slope => base.modulated_slope(value),
                                    crate::FilterControl::Morph => base.modulated_morph(value),
                                    crate::FilterControl::Shape => base.modulated_shape(value),
                                }
                            } else {
                                let source_values =
                                    std::array::from_fn(|source| oscillator_taps[source][frame]);
                                generator_filter_coefficient(
                                    base,
                                    generator_routes.filter_delta(
                                        slot,
                                        &source_values,
                                        route_amounts
                                            .map(|(target, amounts)| (target, amounts[frame])),
                                    ),
                                )
                            };
                            output[group_index][frame] = self.filters[slot].process(
                                coefficients,
                                output[group_index][frame].0,
                                output[group_index][frame].1,
                            );
                        }
                    }
                    GeneratorRtModule::Aux(slot) => {
                        for frame in 0..SAMPLES {
                            if let Some((source, amount)) = generator_routes.aux_route(
                                slot.index(),
                                route_amounts.map(|(target, amounts)| (target, amounts[frame])),
                            ) {
                                let gain = aux[slot.index()].gain * amount.clamp(-1.0, 1.0);
                                let tap = if frame == 0 {
                                    self.aux_oscillator_taps[source]
                                } else {
                                    oscillator_audio[source][frame - 1]
                                };
                                output[group_index][frame].0 += tap.0 * gain;
                                output[group_index][frame].1 += tap.1 * gain;
                            }
                        }
                    }
                }
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
        let mut aux_sources = generator_routes.aux_source_mask();
        while aux_sources != 0 {
            let source = aux_sources.trailing_zeros() as usize;
            aux_sources &= aux_sources - 1;
            let tap = oscillator_audio[source][SAMPLES - 1];
            self.aux_oscillator_taps[source] = if tap.0.is_finite() && tap.1.is_finite() {
                tap
            } else {
                (0.0, 0.0)
            };
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
            let base_step = self.structural_base_step(oscillator, base_step);
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

    #[cfg(test)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the paired probe preserves the production outer-router context"
    )]
    pub(super) fn accumulate_structural_fixed_warp_probe<
        const SAMPLES: usize,
        const INLINE: bool,
    >(
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
        if oscillator.is_silent_resynth() {
            return;
        }
        let voices = usize::from(oscillator.render_voices);
        if oscillator.engine == OscillatorEngineKind::Noise {
            self.accumulate_structural_oscillator_block(
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
        if oscillator.jitter_active() {
            self.accumulate_structural_oscillator_block(
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
            if oscillator.custom_mix > f32::EPSILON
                && oscillator.custom_mix < 1.0
                && (shape == 2.0 || shape == 3.0)
            {
                let samples = self.oscillator_bank.oscillators[slot][0]
                    .generate_custom_block::<SAMPLES>(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                    );
                for frame in 0..SAMPLES {
                    left[frame] += f32x8::splat(samples[frame] * oscillator.left_gain * 0.125);
                    right[frame] += f32x8::splat(samples[frame] * oscillator.right_gain * 0.125);
                }
                return;
            }
            if oscillator.custom_mix <= f32::EPSILON
                && oscillator.phase_warp.active()
                && shape > 2.0
            {
                let samples = if INLINE {
                    self.oscillator_bank.oscillators[slot][0]
                        .generate_shape_block_warped::<SAMPLES>(
                            shape,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            oscillator.phase_warp.mode,
                            oscillator.phase_warp.amount,
                        )
                } else {
                    self.oscillator_bank.oscillators[slot][0]
                        .generate_shape_block_warped_unprepared_probe::<SAMPLES>(
                            shape,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            oscillator.phase_warp.mode,
                            oscillator.phase_warp.amount,
                        )
                };
                for frame in 0..SAMPLES {
                    left[frame] += f32x8::splat(samples[frame] * oscillator.left_gain * 0.125);
                    right[frame] += f32x8::splat(samples[frame] * oscillator.right_gain * 0.125);
                }
                return;
            }
        }
        self.accumulate_structural_oscillator_block(
            slot,
            oscillator,
            settings,
            sample_rate,
            base_step,
            shape,
            left,
            right,
        );
    }

    #[cfg(test)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the paired probe preserves the production structural x8 router"
    )]
    pub(super) fn accumulate_structural_custom_blep_probe<
        const SAMPLES: usize,
        const PRECOMPUTED: bool,
    >(
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
            self.accumulate_structural_oscillator_block(
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
            self.accumulate_structural_oscillator_block(
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

        debug_assert_eq!(voices % 8, 0);
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
                if PRECOMPUTED {
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
                } else {
                    accumulate_custom8_block_constant_unprepared_blep_probe(
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
                }
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
    }

    #[cfg(test)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the paired probe preserves the production structural x4-tail router"
    )]
    pub(super) fn accumulate_structural_custom4_blep_probe<
        const SAMPLES: usize,
        const PRECOMPUTED: bool,
    >(
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
            self.accumulate_structural_oscillator_block(
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
            self.accumulate_structural_oscillator_block(
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

        debug_assert_eq!(voices, 4);
        self.advance_settled_structural_jitter_block::<SAMPLES>(slot, oscillator, sample_rate);
        let packs = voices / 8;
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
                if PRECOMPUTED {
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
                } else {
                    accumulate_custom4_block_constant_unprepared_blep_probe(
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
                }
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
        debug_assert_eq!(tail_start, voices);
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
            phase_steps[lane] = (self.structural_base_step(oscillator, base_step)
                * oscillator.pitch_ratio)
                .min(0.45);
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
            phase_steps[lane] = (self.structural_base_step(oscillator, base_step)
                * oscillator.pitch_ratio)
                .min(0.45);
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
        let base_step = self.structural_base_step(oscillator, base_step);
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
        self.accumulate_structural_oscillator_ratio_block(
            slot,
            oscillator,
            settings,
            sample_rate,
            base_step,
            shape,
            None,
            left,
            right,
        );
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the ratio block renderer adds one source-domain control to the fixed context"
    )]
    fn accumulate_structural_oscillator_ratio_block<const SAMPLES: usize>(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        ratio_brickwall: Option<(f32, f32)>,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        if oscillator.engine.uses_sample_asset() {
            for frame in 0..SAMPLES {
                let (mut sample_left, mut sample_right) = (0.0, 0.0);
                self.accumulate_structural_oscillator(
                    slot,
                    slot,
                    oscillator,
                    None,
                    settings,
                    sample_rate,
                    base_step,
                    shape,
                    ratio_brickwall,
                    0.0,
                    &mut sample_left,
                    &mut sample_right,
                );
                left[frame] += f32x8::from([sample_left, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
                right[frame] += f32x8::from([sample_right, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            }
            return;
        }
        let base_step = self.structural_base_step(oscillator, base_step);
        let voices = usize::from(oscillator.render_voices);
        if let Some(ratio_band) = ratio_brickwall
            && oscillator.engine == OscillatorEngineKind::Va
            && oscillator.custom_mix <= f32::EPSILON
        {
            self.accumulate_structural_ratio_block(
                slot,
                oscillator,
                sample_rate,
                base_step,
                shape,
                ratio_band,
                left,
                right,
            );
            return;
        }
        if oscillator.engine == OscillatorEngineKind::Noise {
            let texture = (oscillator.pulse_width - 0.03) / 0.94;
            for frame in 0..SAMPLES {
                let (noise_left, noise_right) = self.oscillator_bank.noise[slot].next(
                    440.0 / sample_rate,
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
            if oscillator.custom_mix <= f32::EPSILON
                && oscillator.phase_warp.active()
                && shape > 2.0
            {
                let samples = self.oscillator_bank.oscillators[slot][0]
                    .generate_shape_block_warped::<SAMPLES>(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    );
                for frame in 0..SAMPLES {
                    left[frame] += f32x8::splat(samples[frame] * oscillator.left_gain * 0.125);
                    right[frame] += f32x8::splat(samples[frame] * oscillator.right_gain * 0.125);
                }
                return;
            }
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
            if oscillator.custom_mix > f32::EPSILON
                && oscillator.custom_mix < 1.0
                && (shape == 2.0 || shape == 3.0)
            {
                let samples = self.oscillator_bank.oscillators[slot][lane]
                    .generate_custom_block::<SAMPLES>(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                        oscillator.custom_curve,
                        oscillator.custom_mix,
                    );
                for frame in 0..SAMPLES {
                    left[frame] += f32x8::splat(
                        samples[frame]
                            * oscillator.left_gain
                            * oscillator.lane_left_gains[lane]
                            * 0.125,
                    );
                    right[frame] += f32x8::splat(
                        samples[frame]
                            * oscillator.right_gain
                            * oscillator.lane_right_gains[lane]
                            * 0.125,
                    );
                }
                continue;
            }
            if oscillator.custom_mix <= f32::EPSILON
                && oscillator.phase_warp.active()
                && shape > 2.0
            {
                let samples = self.oscillator_bank.oscillators[slot][lane]
                    .generate_shape_block_warped::<SAMPLES>(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                        oscillator.phase_warp.mode,
                        oscillator.phase_warp.amount,
                    );
                for frame in 0..SAMPLES {
                    left[frame] += f32x8::splat(
                        samples[frame]
                            * oscillator.left_gain
                            * oscillator.lane_left_gains[lane]
                            * 0.125,
                    );
                    right[frame] += f32x8::splat(
                        samples[frame]
                            * oscillator.right_gain
                            * oscillator.lane_right_gains[lane]
                            * 0.125,
                    );
                }
                continue;
            }
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
        reason = "the ratio block kernel keeps its bounded oscillator render context explicit"
    )]
    fn accumulate_structural_ratio_block<const SAMPLES: usize>(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        ratio_band: (f32, f32),
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        let Some(source) = PreparedRatioSource::new(
            shape,
            oscillator.pulse_width,
            oscillator.phase_warp.mode,
            oscillator.phase_warp.amount,
            ratio_band,
        ) else {
            return;
        };
        let voices = usize::from(oscillator.render_voices);
        let packs = voices / 8;
        let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
        let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
        let oscillator_step = base_step * oscillator.pitch_ratio;

        if oscillator.jitter_active() {
            for frame in 0..SAMPLES {
                self.advance_structural_jitter(
                    slot,
                    slot,
                    oscillator,
                    oscillator.unison_jitter,
                    oscillator.jitter_rate_hz,
                    sample_rate,
                );
                for (pack, pack_steps) in steps.iter_mut().enumerate().take(packs) {
                    let index = pack * 8;
                    pack_steps[frame] = f32x8::from(std::array::from_fn(|lane| {
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
        } else {
            if voices == 1 {
                self.oscillator_bank.jitter_remaining[slot] = 0;
            } else {
                self.advance_settled_structural_jitter_block::<SAMPLES>(
                    slot,
                    oscillator,
                    sample_rate,
                );
            }
            for (pack, pack_steps) in steps.iter_mut().enumerate().take(packs) {
                let index = pack * 8;
                pack_steps.fill(f32x8::from(std::array::from_fn(|lane| {
                    (oscillator_step * oscillator.lane_pitch_ratios[index + lane]).min(0.45)
                })));
            }
            for (tail, lane) in (packs * 8..voices).enumerate() {
                tail_steps[tail]
                    .fill((oscillator_step * oscillator.lane_pitch_ratios[lane]).min(0.45));
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
            accumulate_shape8_ratio_block(
                &mut self.oscillator_bank.oscillators[slot][index..index + 8],
                phase_steps,
                left_gain,
                right_gain,
                left,
                right,
                &source,
            );
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
            accumulate_shape4_ratio_block(
                &mut self.oscillator_bank.oscillators[slot][tail_start..tail_start + 4],
                phase_steps,
                left_gain,
                right_gain,
                left,
                right,
                &source,
            );
            tail_start += 4;
        }

        for lane in tail_start..voices {
            let tail = lane - packs * 8;
            for frame in 0..SAMPLES {
                let sample = self.oscillator_bank.oscillators[slot][lane]
                    .generate_shape_step_prepared_ratio(tail_steps[tail][frame], &source);
                left[frame] += f32x8::splat(
                    sample * oscillator.left_gain * oscillator.lane_left_gains[lane] * 0.125,
                );
                right[frame] += f32x8::splat(
                    sample * oscillator.right_gain * oscillator.lane_right_gains[lane] * 0.125,
                );
            }
        }
    }

    #[cfg(test)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the paired probe preserves the production structural one-lane router"
    )]
    pub(super) fn accumulate_structural_custom_scalar_blep_probe<
        const SAMPLES: usize,
        const PRECOMPUTED: bool,
    >(
        &mut self,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        settings: VoiceSettings,
        base_step: f32,
        shape: f32,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        debug_assert_eq!(oscillator.engine, OscillatorEngineKind::Va);
        debug_assert_eq!(oscillator.render_voices, 1);
        debug_assert!(!oscillator.jitter_active());
        debug_assert!(oscillator.custom_mix > f32::EPSILON);
        self.oscillator_bank.jitter_remaining[slot] = 0;
        let phase_step = (base_step * oscillator.pitch_ratio).min(0.45);
        let samples = if PRECOMPUTED {
            self.oscillator_bank.oscillators[slot][0].generate_custom_block::<SAMPLES>(
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
            std::array::from_fn(|_| {
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
            })
        };
        for frame in 0..SAMPLES {
            left[frame] += f32x8::splat(samples[frame] * oscillator.left_gain * 0.125);
            right[frame] += f32x8::splat(samples[frame] * oscillator.right_gain * 0.125);
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
        let base_step = self.structural_base_step(oscillator, base_step);
        let voices = usize::from(oscillator.render_voices);
        debug_assert!(voices > 1);
        debug_assert!(oscillator.jitter_active());
        let packs = voices / 8;
        let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
        let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
        let oscillator_step = base_step * oscillator.pitch_ratio;
        for frame in 0..SAMPLES {
            self.advance_structural_jitter(
                slot,
                slot,
                oscillator,
                oscillator.unison_jitter,
                oscillator.jitter_rate_hz,
                sample_rate,
            );
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
        if oscillator.custom_active()
            && oscillator.custom_mix < 1.0
            && (shape == 2.0 || shape == 3.0)
        {
            self.oscillators[oscillator_index][0].generate_custom_block::<SAMPLES>(
                shape,
                phase_step,
                oscillator.pulse_width,
                antialiasing,
                oscillator.phase_warp.mode,
                oscillator.phase_warp.amount,
                oscillator.custom_curve,
                oscillator.custom_mix,
            )
        } else if oscillator.custom_active() {
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
            if shapes.is_none() && shape > 2.0 {
                self.oscillators[oscillator_index][0].generate_shape_block_warped(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else {
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
            }
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

    #[cfg(test)]
    pub(super) fn render_single_lane_custom_scalar_blep_probe<
        const SAMPLES: usize,
        const PRECOMPUTED: bool,
    >(
        &mut self,
        oscillator_index: usize,
        oscillator: OscillatorSettings,
        shape: f32,
        phase_step: f32,
        antialiasing: Antialiasing,
    ) -> [f32; SAMPLES] {
        debug_assert!(oscillator.custom_active());
        if PRECOMPUTED {
            self.oscillators[oscillator_index][0].generate_custom_block::<SAMPLES>(
                shape,
                phase_step,
                oscillator.pulse_width,
                antialiasing,
                oscillator.phase_warp.mode,
                oscillator.phase_warp.amount,
                oscillator.custom_curve,
                oscillator.custom_mix,
            )
        } else {
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
                if oscillator.custom_active()
                    && oscillator.custom_mix < 1.0
                    && (shape == 2.0 || shape == 3.0)
                {
                    let samples = self.oscillators[oscillator_index][index]
                        .generate_custom_block::<SAMPLES>(
                            shape,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                            oscillator.phase_warp.mode,
                            oscillator.phase_warp.amount,
                            oscillator.custom_curve,
                            oscillator.custom_mix,
                        );
                    for frame in 0..SAMPLES {
                        left[frame] += f32x8::splat(
                            samples[frame] * self.secondary_unison[secondary].left[index] * 0.125,
                        );
                        right[frame] += f32x8::splat(
                            samples[frame] * self.secondary_unison[secondary].right[index] * 0.125,
                        );
                    }
                    continue;
                }
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
