//! Scalar and control-rate voice rendering kernels.

use super::*;

impl VaVoice {
    pub(super) fn render_controlled_frame<const DYNAMIC_UNISON: bool>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        force_gate: bool,
        unison_control: &UnisonFrameControl,
    ) -> LegacyScalarFrame {
        if force_gate && self.stage == EnvelopeStage::Idle {
            let seed = 0x4452_4f4e_452d_4b56;
            self.randomize_oscillators(seed);
            self.velocity = 1.0;
            self.pressure = 0.0;
            self.timbre = 0.5;
            self.begin_attack();
        }
        self.advance_envelope(sample_rate, force_gate);
        self.advance_unison_transitions();
        if DYNAMIC_UNISON {
            self.prepare_dynamic_unison_spatial(unison_control);
        }

        let primary = settings.oscillator(0);
        if primary.enabled && !force_gate && self.phase_steps_dirty {
            self.refresh_phase_steps();
        }
        if !force_gate {
            self.advance_glide();
        }
        if primary.enabled && self.unison.settings.motion_active() {
            self.advance_swarm();
        }
        let dynamic_base_step = force_gate
            .then(|| self.base_phase_step(settings.frequency_hz * self.pitch_ratio, sample_rate));
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = self.envelope_level * velocity_gain * pressure_gain;
        let shape = self.effective_shape(settings);
        let mut left;
        let mut right;
        let voice_count = if primary.enabled {
            usize::from(self.unison.render_voices)
        } else {
            0
        };
        let mut index = 0;
        let mut left8 = f32x8::ZERO;
        let mut right8 = f32x8::ZERO;
        if shape <= f32::EPSILON {
            while index + 8 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step::<DYNAMIC_UNISON>(
                        index + lane,
                        primary.pitch_ratio,
                        dynamic_base_step,
                        unison_control,
                    )
                });
                let samples = if primary.custom_active() {
                    generate_custom8(
                        &mut self.oscillators[0][index..index + 8],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                        primary.custom_curve,
                        primary.custom_mix,
                    )
                } else if primary.phase_warp_active() {
                    generate_shape8_warped(
                        &mut self.oscillators[0][index..index + 8],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else {
                    generate_sine8(&mut self.oscillators[0][index..index + 8], phase_steps)
                };
                let (left_gains, right_gains) =
                    self.unison_gains8::<DYNAMIC_UNISON>(0, index, unison_control);
                left8 = samples.mul_add(left_gains, left8);
                right8 = samples.mul_add(right_gains, right8);
                index += 8;
            }
            left = left8.reduce_add();
            right = right8.reduce_add();

            let mut left4 = f32x4::ZERO;
            let mut right4 = f32x4::ZERO;
            while index + 4 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step::<DYNAMIC_UNISON>(
                        index + lane,
                        primary.pitch_ratio,
                        dynamic_base_step,
                        unison_control,
                    )
                });
                let samples4 = if primary.custom_active() {
                    generate_custom4(
                        &mut self.oscillators[0][index..index + 4],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                        primary.custom_curve,
                        primary.custom_mix,
                    )
                } else if primary.phase_warp_active() {
                    generate_shape4_warped(
                        &mut self.oscillators[0][index..index + 4],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else {
                    generate_sine4(&mut self.oscillators[0][index..index + 4], phase_steps)
                };
                let (left_gains, right_gains) =
                    self.unison_gains4::<DYNAMIC_UNISON>(0, index, unison_control);
                left4 = samples4.mul_add(left_gains, left4);
                right4 = samples4.mul_add(right_gains, right4);
                index += 4;
            }
            left += left4.reduce_add();
            right += right4.reduce_add();
        } else {
            while index + 8 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step::<DYNAMIC_UNISON>(
                        index + lane,
                        primary.pitch_ratio,
                        dynamic_base_step,
                        unison_control,
                    )
                });
                let oscillators = &mut self.oscillators[0][index..index + 8];
                let samples = if primary.custom_active() {
                    generate_custom8(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                        primary.custom_curve,
                        primary.custom_mix,
                    )
                } else if primary.phase_warp_active() {
                    generate_shape8_warped(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else if (shape - 1.0).abs() <= f32::EPSILON {
                    generate_triangle8(oscillators, phase_steps, settings.antialiasing)
                } else if (shape - 2.0).abs() <= f32::EPSILON {
                    generate_saw8(oscillators, phase_steps, settings.antialiasing)
                } else if shape >= 3.0 - f32::EPSILON {
                    generate_pulse8(
                        oscillators,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                } else {
                    generate_shape8(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                };
                let (left_gains, right_gains) =
                    self.unison_gains8::<DYNAMIC_UNISON>(0, index, unison_control);
                left8 = samples.mul_add(left_gains, left8);
                right8 = samples.mul_add(right_gains, right8);
                index += 8;
            }
            left = left8.reduce_add();
            right = right8.reduce_add();

            let mut left4 = f32x4::ZERO;
            let mut right4 = f32x4::ZERO;
            while index + 4 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step::<DYNAMIC_UNISON>(
                        index + lane,
                        primary.pitch_ratio,
                        dynamic_base_step,
                        unison_control,
                    )
                });
                let oscillators = &mut self.oscillators[0][index..index + 4];
                let samples = if primary.custom_active() {
                    generate_custom4(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                        primary.custom_curve,
                        primary.custom_mix,
                    )
                } else if primary.phase_warp_active() {
                    generate_shape4_warped(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else if (shape - 1.0).abs() <= f32::EPSILON {
                    generate_triangle4(oscillators, phase_steps, settings.antialiasing)
                } else if (shape - 2.0).abs() <= f32::EPSILON {
                    generate_saw4(oscillators, phase_steps, settings.antialiasing)
                } else if shape >= 3.0 - f32::EPSILON {
                    generate_pulse4(
                        oscillators,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                } else {
                    generate_shape4(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                };
                let (left_gains, right_gains) =
                    self.unison_gains4::<DYNAMIC_UNISON>(0, index, unison_control);
                left4 = samples.mul_add(left_gains, left4);
                right4 = samples.mul_add(right_gains, right4);
                index += 4;
            }
            left += left4.reduce_add();
            right += right4.reduce_add();
        }
        while index < voice_count {
            let phase_step = self.oscillator_phase_step::<DYNAMIC_UNISON>(
                index,
                primary.pitch_ratio,
                dynamic_base_step,
                unison_control,
            );
            let sample = if primary.custom_active() {
                self.oscillators[0][index].generate_custom_step(
                    shape,
                    phase_step,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                    primary.custom_curve,
                    primary.custom_mix,
                )
            } else if primary.phase_warp_active() {
                self.oscillators[0][index].generate_shape_step_warped(
                    shape,
                    phase_step,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                self.oscillators[0][index].generate_shape_step(
                    shape,
                    phase_step,
                    primary.pulse_width,
                    settings.antialiasing,
                )
            };
            left = sample.mul_add(
                self.unison_left_gain::<DYNAMIC_UNISON>(0, index, unison_control),
                left,
            );
            right = sample.mul_add(
                self.unison_right_gain::<DYNAMIC_UNISON>(0, index, unison_control),
                right,
            );
            index += 1;
        }
        let (primary_left, primary_right) = if primary.enabled {
            primary.channel_gains()
        } else {
            (0.0, 0.0)
        };
        left *= primary_left;
        right *= primary_right;
        let gain = amplitude * self.unison_layout_gain::<DYNAMIC_UNISON>(0, unison_control);
        let has_secondary = settings.oscillator(1).enabled || settings.oscillator(2).enabled;
        let mut secondary = [(0.0, 0.0); LEGACY_OSCILLATOR_COUNT - 1];
        if has_secondary {
            for oscillator in 1..LEGACY_OSCILLATOR_COUNT {
                secondary[oscillator - 1] = self.render_secondary_oscillator::<DYNAMIC_UNISON>(
                    settings,
                    oscillator,
                    dynamic_base_step,
                    unison_control,
                );
            }
        }
        LegacyScalarFrame {
            primary: (left * gain, right * gain),
            secondary,
            amplitude,
            has_secondary,
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "two internal samples share oscillator, gain, and Swarm lane setup"
    )]
    pub fn render_pair(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [f32; 2],
    ) -> ([(f32, f32); 2], bool) {
        debug_assert!(self.active());
        debug_assert!(self.unison_transitions_steady());
        self.advance_envelope(sample_rate, false);
        let envelope0 = self.envelope_level;
        let render_second = self.active();
        if render_second {
            self.advance_envelope(sample_rate, false);
        }
        if self.phase_steps_dirty {
            self.refresh_phase_steps();
        }
        let swarm = self.unison.settings.motion_active();
        debug_assert!(!swarm || !self.is_gliding());
        let glide_phase_steps = if self.is_gliding() {
            self.advance_glide();
            let first = self.phase_steps;
            if render_second {
                self.advance_glide();
            }
            [first, self.phase_steps]
        } else {
            [self.phase_steps; 2]
        };
        let mut first_swarm_frame_advanced = false;
        if swarm {
            if render_second {
                first_swarm_frame_advanced = self.prepare_swarm_pair(swarm_clocks);
            } else {
                self.set_swarm_clock(swarm_clocks[0]);
                self.advance_swarm();
            }
        }
        let envelope1 = if render_second {
            self.envelope_level
        } else {
            0.0
        };

        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = [
            envelope0 * velocity_gain * pressure_gain,
            envelope1 * velocity_gain * pressure_gain,
        ];
        let shape = self.effective_shape(settings);
        let primary = settings.oscillator(0);
        let voice_count = usize::from(self.unison.render_voices);
        let mut index = 0;
        let mut left8 = [f32x8::ZERO; 2];
        let mut right8 = [f32x8::ZERO; 2];
        while index + 8 <= voice_count {
            let phase_steps = if swarm {
                self.advance_jitter_phase_steps8_pair(
                    index,
                    render_second,
                    first_swarm_frame_advanced,
                )
            } else {
                [
                    std::array::from_fn(|lane| glide_phase_steps[0][index + lane]),
                    if render_second {
                        std::array::from_fn(|lane| glide_phase_steps[1][index + lane])
                    } else {
                        [0.0; 8]
                    },
                ]
            };
            let samples = if primary.phase_warp_active() {
                generate_shape8_pair_warped(
                    &mut self.oscillators[0][index..index + 8],
                    shape,
                    phase_steps,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                generate_shape8_pair(
                    &mut self.oscillators[0][index..index + 8],
                    shape,
                    phase_steps,
                    settings.pulse_width,
                    settings.antialiasing,
                )
            };
            let left0 = f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
            let right0 = f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
            left8[0] = samples[0].mul_add(left0, left8[0]);
            right8[0] = samples[0].mul_add(right0, right8[0]);
            left8[1] = samples[1].mul_add(left0, left8[1]);
            right8[1] = samples[1].mul_add(right0, right8[1]);
            index += 8;
        }
        let mut left = [left8[0].reduce_add(), left8[1].reduce_add()];
        let mut right = [right8[0].reduce_add(), right8[1].reduce_add()];

        let mut left4 = [f32x4::ZERO; 2];
        let mut right4 = [f32x4::ZERO; 2];
        while index + 4 <= voice_count {
            let phase_steps = if swarm {
                self.advance_jitter_phase_steps4_pair(
                    index,
                    render_second,
                    first_swarm_frame_advanced,
                )
            } else {
                [
                    std::array::from_fn(|lane| glide_phase_steps[0][index + lane]),
                    if render_second {
                        std::array::from_fn(|lane| glide_phase_steps[1][index + lane])
                    } else {
                        [0.0; 4]
                    },
                ]
            };
            let samples = if primary.phase_warp_active() {
                generate_shape4_pair_warped(
                    &mut self.oscillators[0][index..index + 4],
                    shape,
                    phase_steps,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                generate_shape4_pair(
                    &mut self.oscillators[0][index..index + 4],
                    shape,
                    phase_steps,
                    settings.pulse_width,
                    settings.antialiasing,
                )
            };
            let left0 = f32x4::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
            let right0 = f32x4::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
            left4[0] = samples[0].mul_add(left0, left4[0]);
            right4[0] = samples[0].mul_add(right0, right4[0]);
            left4[1] = samples[1].mul_add(left0, left4[1]);
            right4[1] = samples[1].mul_add(right0, right4[1]);
            index += 4;
        }
        for frame in 0..2 {
            left[frame] += left4[frame].reduce_add();
            right[frame] += right4[frame].reduce_add();
        }
        while index < voice_count {
            let phase_steps = if swarm {
                let phase_steps = self.advance_jitter_phase_steps_pair::<1>(
                    index,
                    render_second,
                    first_swarm_frame_advanced,
                );
                [phase_steps[0][0], phase_steps[1][0]]
            } else {
                [
                    glide_phase_steps[0][index],
                    if render_second {
                        glide_phase_steps[1][index]
                    } else {
                        0.0
                    },
                ]
            };
            let samples = if primary.phase_warp_active() {
                self.oscillators[0][index].generate_shape_step_pair_warped(
                    shape,
                    phase_steps,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                self.oscillators[0][index].generate_shape_step_pair(
                    shape,
                    phase_steps,
                    settings.pulse_width,
                    settings.antialiasing,
                )
            };
            for frame in 0..2 {
                left[frame] = samples[frame].mul_add(self.unison.left[index], left[frame]);
                right[frame] = samples[frame].mul_add(self.unison.right[index], right[frame]);
            }
            index += 1;
        }
        let gains = [
            amplitude[0] * self.unison.gain,
            amplitude[1] * self.unison.gain,
        ];
        let output = [
            (left[0] * gains[0], right[0] * gains[0]),
            (left[1] * gains[1], right[1] * gains[1]),
        ];
        (output, render_second)
    }

    pub fn render_saw_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        self.render_shape_block(settings, sample_rate, swarm_clocks, None, None, 0)
    }

    pub fn render_morph_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        shapes: &[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        self.render_shape_block(settings, sample_rate, swarm_clocks, Some(shapes), None, 0)
    }

    pub(crate) fn render_motion_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        motion: &[[UnisonMotionFrame; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        motion_mask: OscillatorMask,
    ) -> [(f32, f32); SAMPLES] {
        self.render_shape_block(
            settings,
            sample_rate,
            swarm_clocks,
            None,
            Some(motion),
            motion_mask,
        )
    }

    #[inline]
    pub(super) fn configure_unison_motion_frame(&mut self, motion: UnisonMotionFrame) {
        let mut settings = self.unison.settings;
        settings.phase_random = motion.phase_random;
        settings.swarm_amount = motion.swarm_amount;
        settings.swarm_rate = motion.swarm_rate;
        self.configure_unison_motion(settings);
    }

    #[inline]
    pub(super) fn configure_secondary_unison_motion_frame(
        &mut self,
        secondary: usize,
        motion: UnisonMotionFrame,
    ) {
        let mut settings = self.secondary_unison[secondary].settings;
        settings.phase_random = motion.phase_random;
        settings.swarm_amount = motion.swarm_amount;
        settings.swarm_rate = motion.swarm_rate;
        self.configure_secondary_unison_motion(secondary + 1, settings);
    }

    pub(crate) fn render_pitch_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        controls: &[PitchModulationFrame],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert_eq!(controls.len(), SAMPLES);
        debug_assert!(self.active());
        debug_assert!(self.held);
        debug_assert!(!self.is_gliding());
        debug_assert!(self.pitch_block_eligible());

        if self.phase_steps_dirty {
            self.refresh_phase_steps();
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if self.secondary_phase_steps_dirty[secondary] {
                self.refresh_secondary_phase_steps(secondary);
            }
        }

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
            *value = self.envelope_level * velocity_gain * pressure_gain;
        }

        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            self.accumulate_pitch_oscillator_block(
                oscillator,
                settings,
                controls,
                &amplitude,
                &mut output,
            );
        }
        output
    }

    pub(crate) fn render_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: &[VoiceSettings; SAMPLES],
        envelopes: &[EnvelopeSettings; SAMPLES],
        sample_rate: f32,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(SAMPLES <= BLOCK_INTERNAL_SAMPLES);
        debug_assert!(self.active());
        debug_assert!(!self.is_gliding());
        debug_assert!(self.control_block_eligible());

        if self.phase_steps_dirty {
            self.refresh_phase_steps();
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if self.secondary_phase_steps_dirty[secondary] {
                self.refresh_secondary_phase_steps(secondary);
            }
        }

        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for frame in 0..SAMPLES {
            if !self.active() {
                break;
            }
            self.configure(envelopes[frame]);
            self.advance_envelope(sample_rate, false);
            let frame_settings = settings[frame];
            let velocity_gain = frame_settings
                .velocity_amount
                .clamp(0.0, 1.0)
                .mul_add(self.velocity - 1.0, 1.0);
            let pressure_gain = frame_settings
                .pressure_amount
                .clamp(0.0, 1.0)
                .mul_add(self.pressure, 1.0);
            let amplitude = self.envelope_level * velocity_gain * pressure_gain;
            for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                self.accumulate_control_oscillator_frame(
                    frame_settings,
                    oscillator,
                    amplitude,
                    &mut output[frame],
                );
            }
        }
        output
    }

    pub(in crate::voices) fn render_oscillator_bank_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: &[VoiceSettings; SAMPLES],
        envelopes: &[EnvelopeSettings; SAMPLES],
        sample_rate: f32,
        oscillator_bank: &ActiveOscillatorRenderSet,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.control_block_eligible());
        debug_assert!(!oscillator_bank.transitioning());
        std::array::from_fn(|frame| {
            if !self.active() {
                return (0.0, 0.0);
            }
            self.configure(envelopes[frame]);
            self.advance_envelope(sample_rate, false);
            self.render_oscillator_bank(
                oscillator_bank,
                settings[frame],
                sample_rate,
                &StructuralOscillatorFrameControl::NEUTRAL,
            )
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the frame kernel keeps the waveform, lane, and stereo state local"
    )]
    pub(super) fn accumulate_control_oscillator_frame(
        &mut self,
        settings: VoiceSettings,
        oscillator_index: usize,
        amplitude: f32,
        output: &mut (f32, f32),
    ) {
        let oscillator = settings.oscillator(oscillator_index);
        if !oscillator.enabled {
            return;
        }
        let (base_steps, left_values, right_values, render_voices, unison_gain) =
            if oscillator_index == 0 {
                (
                    self.phase_steps,
                    self.unison.left,
                    self.unison.right,
                    self.unison.render_voices,
                    self.unison.gain,
                )
            } else {
                let layout = &self.secondary_unison[oscillator_index - 1];
                (
                    self.secondary_phase_steps[oscillator_index - 1],
                    layout.left,
                    layout.right,
                    layout.render_voices,
                    layout.gain,
                )
            };
        let phase_ratio = oscillator.pitch_ratio;
        let shape = self.effective_oscillator_shape(settings, oscillator_index);
        let mut left8 = f32x8::ZERO;
        let mut right8 = f32x8::ZERO;
        let mut index = 0;
        let voice_count = usize::from(render_voices);
        while index + 8 <= voice_count {
            let phase_steps =
                std::array::from_fn(|lane| (base_steps[index + lane] * phase_ratio).min(0.45));
            let oscillators = &mut self.oscillators[oscillator_index][index..index + 8];
            let samples = if oscillator.custom_active() {
                generate_custom8(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp_active() {
                generate_shape8_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else if shape <= f32::EPSILON {
                generate_sine8(oscillators, phase_steps)
            } else if (shape - 1.0).abs() <= f32::EPSILON {
                generate_triangle8(oscillators, phase_steps, settings.antialiasing)
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                generate_saw8(oscillators, phase_steps, settings.antialiasing)
            } else if shape >= 3.0 - f32::EPSILON {
                generate_pulse8(
                    oscillators,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            } else {
                generate_shape8(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            let left_gain = f32x8::from(std::array::from_fn(|lane| left_values[index + lane]));
            let right_gain = f32x8::from(std::array::from_fn(|lane| right_values[index + lane]));
            left8 = samples.mul_add(left_gain, left8);
            right8 = samples.mul_add(right_gain, right8);
            index += 8;
        }
        let mut left = left8.reduce_add();
        let mut right = right8.reduce_add();
        let mut left4 = f32x4::ZERO;
        let mut right4 = f32x4::ZERO;
        while index + 4 <= voice_count {
            let phase_steps =
                std::array::from_fn(|lane| (base_steps[index + lane] * phase_ratio).min(0.45));
            let oscillators = &mut self.oscillators[oscillator_index][index..index + 4];
            let samples = if oscillator.custom_active() {
                generate_custom4(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp_active() {
                generate_shape4_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else if shape <= f32::EPSILON {
                generate_sine4(oscillators, phase_steps)
            } else if (shape - 1.0).abs() <= f32::EPSILON {
                generate_triangle4(oscillators, phase_steps, settings.antialiasing)
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                generate_saw4(oscillators, phase_steps, settings.antialiasing)
            } else if shape >= 3.0 - f32::EPSILON {
                generate_pulse4(
                    oscillators,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            } else {
                generate_shape4(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            let left_gain = f32x4::from(std::array::from_fn(|lane| left_values[index + lane]));
            let right_gain = f32x4::from(std::array::from_fn(|lane| right_values[index + lane]));
            left4 = samples.mul_add(left_gain, left4);
            right4 = samples.mul_add(right_gain, right4);
            index += 4;
        }
        left += left4.reduce_add();
        right += right4.reduce_add();
        while index < voice_count {
            let phase_step = (base_steps[index] * phase_ratio).min(0.45);
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
            } else if oscillator.phase_warp_active() {
                self.oscillators[oscillator_index][index].generate_shape_step_warped(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else {
                self.oscillators[oscillator_index][index].generate_shape_step(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            left = sample.mul_add(left_values[index], left);
            right = sample.mul_add(right_values[index], right);
            index += 1;
        }
        let (channel_left, channel_right) = oscillator.channel_gains();
        let gain = amplitude * unison_gain;
        output.0 += left * channel_left * gain;
        output.1 += right * channel_right * gain;
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the bounded block kernel keeps one SIMD accumulator per oscillator"
    )]
    pub(super) fn accumulate_pitch_oscillator_block<const SAMPLES: usize>(
        &mut self,
        oscillator_index: usize,
        settings: VoiceSettings,
        controls: &[PitchModulationFrame],
        amplitude: &[f32; SAMPLES],
        output: &mut [(f32, f32); SAMPLES],
    ) {
        let oscillator = settings.oscillator(oscillator_index);
        if !oscillator.enabled {
            return;
        }
        let bit = 1 << oscillator_index;
        let (base_steps, left_values, right_values, render_voices, static_unison_gain) =
            if oscillator_index == 0 {
                (
                    self.phase_steps,
                    self.unison.left,
                    self.unison.right,
                    self.unison.render_voices,
                    self.unison.gain,
                )
            } else {
                let layout = &self.secondary_unison[oscillator_index - 1];
                (
                    self.secondary_phase_steps[oscillator_index - 1],
                    layout.left,
                    layout.right,
                    layout.render_voices,
                    layout.gain,
                )
            };
        let voice_count = usize::from(render_voices);
        let dynamic_spatial = controls
            .iter()
            .any(|control| control.unison_spatial_active_mask & bit != 0);
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let packs = voice_count / 8;
        let oscillators = &mut self.oscillators[oscillator_index];

        for pack in 0..packs {
            let index = pack * 8;
            let phase_steps = std::array::from_fn(|frame| {
                f32x8::from(std::array::from_fn(|lane| {
                    let lane = index + lane;
                    let correction = if controls[frame].unison_active_mask & bit != 0 {
                        controls[frame].unison_pitch_correction[oscillator_index][lane]
                    } else {
                        1.0
                    };
                    (base_steps[lane]
                        * controls[frame].oscillator_pitch_ratios[oscillator_index]
                        * correction)
                        .min(0.45)
                }))
            });
            if dynamic_spatial {
                let left_gains = std::array::from_fn(|frame| {
                    f32x8::from(std::array::from_fn(|lane| {
                        if controls[frame].unison_spatial_active_mask & bit != 0 {
                            controls[frame].unison_spatial_left[oscillator_index][index + lane]
                        } else {
                            left_values[index + lane]
                        }
                    }))
                });
                let right_gains = std::array::from_fn(|frame| {
                    f32x8::from(std::array::from_fn(|lane| {
                        if controls[frame].unison_spatial_active_mask & bit != 0 {
                            controls[frame].unison_spatial_right[oscillator_index][index + lane]
                        } else {
                            right_values[index + lane]
                        }
                    }))
                });
                accumulate_saw8_block_dynamic_gains(
                    &mut oscillators[index..index + 8],
                    phase_steps,
                    left_gains,
                    right_gains,
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
            } else {
                let left_gain = f32x8::from(std::array::from_fn(|lane| left_values[index + lane]));
                let right_gain =
                    f32x8::from(std::array::from_fn(|lane| right_values[index + lane]));
                accumulate_saw8_block(
                    &mut oscillators[index..index + 8],
                    phase_steps,
                    left_gain,
                    right_gain,
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
            }
        }

        let mut tail_start = packs * 8;
        if voice_count - tail_start >= 4 {
            let index = tail_start;
            let phase_steps = std::array::from_fn(|frame| {
                f32x4::from(std::array::from_fn(|lane| {
                    let lane = index + lane;
                    let correction = if controls[frame].unison_active_mask & bit != 0 {
                        controls[frame].unison_pitch_correction[oscillator_index][lane]
                    } else {
                        1.0
                    };
                    (base_steps[lane]
                        * controls[frame].oscillator_pitch_ratios[oscillator_index]
                        * correction)
                        .min(0.45)
                }))
            });
            if dynamic_spatial {
                let left_gains = std::array::from_fn(|frame| {
                    f32x4::from(std::array::from_fn(|lane| {
                        if controls[frame].unison_spatial_active_mask & bit != 0 {
                            controls[frame].unison_spatial_left[oscillator_index][index + lane]
                        } else {
                            left_values[index + lane]
                        }
                    }))
                });
                let right_gains = std::array::from_fn(|frame| {
                    f32x4::from(std::array::from_fn(|lane| {
                        if controls[frame].unison_spatial_active_mask & bit != 0 {
                            controls[frame].unison_spatial_right[oscillator_index][index + lane]
                        } else {
                            right_values[index + lane]
                        }
                    }))
                });
                accumulate_saw4_block_dynamic_gains(
                    &mut oscillators[index..index + 4],
                    phase_steps,
                    left_gains,
                    right_gains,
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
            } else {
                let left_gain = f32x4::from(std::array::from_fn(|lane| left_values[index + lane]));
                let right_gain =
                    f32x4::from(std::array::from_fn(|lane| right_values[index + lane]));
                accumulate_saw4_block(
                    &mut oscillators[index..index + 4],
                    phase_steps,
                    left_gain,
                    right_gain,
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
            }
            tail_start += 4;
        }

        for index in tail_start..voice_count {
            for frame in 0..SAMPLES {
                let correction = if controls[frame].unison_active_mask & bit != 0 {
                    controls[frame].unison_pitch_correction[oscillator_index][index]
                } else {
                    1.0
                };
                let phase_step = (base_steps[index]
                    * controls[frame].oscillator_pitch_ratios[oscillator_index]
                    * correction)
                    .min(0.45);
                let sample = oscillators[index].generate_shape_step(
                    2.0,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                );
                let (left_gain, right_gain) =
                    if controls[frame].unison_spatial_active_mask & bit != 0 {
                        (
                            controls[frame].unison_spatial_left[oscillator_index][index],
                            controls[frame].unison_spatial_right[oscillator_index][index],
                        )
                    } else {
                        (left_values[index], right_values[index])
                    };
                left[frame] += f32x8::splat(sample * left_gain * 0.125);
                right[frame] += f32x8::splat(sample * right_gain * 0.125);
            }
        }

        let (channel_left, channel_right) = oscillator.channel_gains();
        for frame in 0..SAMPLES {
            let unison_gain = if controls[frame].unison_spatial_active_mask & bit != 0 {
                controls[frame].unison_spatial_gain[oscillator_index]
            } else {
                static_unison_gain
            };
            let gain = amplitude[frame] * unison_gain;
            output[frame].0 += left[frame].reduce_add() * (channel_left * gain);
            output[frame].1 += right[frame].reduce_add() * (channel_right * gain);
        }
    }

    pub(in crate::voices) fn render_generic_morph_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        shapes: &[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        std::array::from_fn(|frame| {
            let mut frame_settings = settings;
            frame_settings.shape = shapes[0][frame];
            for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                if frame_settings.oscillators[oscillator].enabled {
                    frame_settings.oscillators[oscillator].shape = shapes[oscillator][frame];
                    if oscillator == 0 {
                        self.set_swarm_clock(swarm_clocks[0][frame]);
                    } else {
                        self.set_secondary_swarm_clock(oscillator, swarm_clocks[oscillator][frame]);
                    }
                }
            }
            self.render(frame_settings, sample_rate, false)
        })
    }

    pub(super) fn render_shape_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        shapes: Option<&[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion: Option<&[[UnisonMotionFrame; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        motion_mask: OscillatorMask,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.active());
        debug_assert!(self.held);
        debug_assert!(!self.is_gliding());
        let primary = settings.oscillator(0);
        let primary_shape = self.effective_shape(settings);
        let primary_shapes = shapes.map(|shapes| {
            std::array::from_fn(|frame| {
                self.effective_oscillator_shape_value(settings, 0, shapes[0][frame])
            })
        });
        debug_assert!((8..=BLOCK_INTERNAL_SAMPLES).contains(&SAMPLES));
        if primary.enabled && self.phase_steps_dirty {
            self.refresh_phase_steps();
        }

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
            *value = self.envelope_level * velocity_gain * pressure_gain;
        }

        let voice_count = if primary.enabled {
            usize::from(self.unison.render_voices)
        } else {
            0
        };
        if voice_count == 1 {
            if motion_mask & 1 != 0
                && let Some(motion) = motion
            {
                self.configure_unison_motion_frame(motion[0][SAMPLES - 1]);
            }
            return self.render_single_lane_primary_block(
                settings,
                primary,
                primary_shape,
                primary_shapes.as_ref(),
                &amplitude,
                &swarm_clocks,
                shapes,
                motion,
                motion_mask,
            );
        }
        let primary_morph_gains = primary_shapes
            .as_ref()
            .map(|shapes| std::array::from_fn(|frame| shape_morph_gain(shapes[frame])));
        let packs = voice_count / 8;
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let primary_motion_dynamic = motion_mask & 1 != 0;
        if !primary.enabled || !primary_motion_dynamic && !self.unison.settings.motion_active() {
            for pack in 0..packs {
                let index = pack * 8;
                let steps = f32x8::from(std::array::from_fn(|lane| {
                    tuned_phase_step(self.phase_steps[index + lane], primary.pitch_ratio)
                }));
                let left_gain =
                    f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                let right_gain =
                    f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                if let (Some(shapes), Some(morph_gains)) = (&primary_shapes, &primary_morph_gains) {
                    accumulate_shape8_block_morphing(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shapes,
                        morph_gains,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                } else if primary.custom_active() {
                    accumulate_custom8_block_constant(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary.custom_curve,
                        primary.custom_mix,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    );
                } else if primary.phase_warp_active() {
                    accumulate_shape8_block_constant_warped(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    );
                } else if (primary_shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw8_block_constant(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape8_block_constant(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                }
            }
            let mut tail_start = packs * 8;
            if voice_count - tail_start >= 4 {
                let steps = f32x4::from(std::array::from_fn(|lane| {
                    tuned_phase_step(self.phase_steps[tail_start + lane], primary.pitch_ratio)
                }));
                let left_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.left[tail_start + lane]
                }));
                let right_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.right[tail_start + lane]
                }));
                if let (Some(shapes), Some(morph_gains)) = (&primary_shapes, &primary_morph_gains) {
                    accumulate_shape4_block_morphing(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shapes,
                        morph_gains,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                } else if primary.custom_active() {
                    accumulate_custom4_block_constant(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary.custom_curve,
                        primary.custom_mix,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    );
                } else if primary.phase_warp_active() {
                    accumulate_shape4_block_constant_warped(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    );
                } else if (primary_shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw4_block_constant(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape4_block_constant(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                }
                tail_start += 4;
            }
            let tail_lanes = voice_count - tail_start;
            if tail_lanes >= 2 {
                self.accumulate_short_static(
                    tail_start,
                    tail_lanes,
                    primary,
                    primary_shape,
                    primary_shapes.as_ref(),
                    primary_morph_gains.as_ref(),
                    settings.antialiasing,
                    &mut left,
                    &mut right,
                );
                tail_start += tail_lanes;
            }
            for index in tail_start..voice_count {
                let phase_step = tuned_phase_step(self.phase_steps[index], primary.pitch_ratio);
                if primary.custom_active()
                    && primary.custom_mix < 1.0
                    && (primary_shape == 2.0 || primary_shape == 3.0)
                {
                    let samples = self.oscillators[0][index].generate_custom_block::<SAMPLES>(
                        primary_shape,
                        phase_step,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                        primary.custom_curve,
                        primary.custom_mix,
                    );
                    for frame in 0..SAMPLES {
                        left[frame] +=
                            f32x8::splat(samples[frame] * self.unison.left[index] * 0.125);
                        right[frame] +=
                            f32x8::splat(samples[frame] * self.unison.right[index] * 0.125);
                    }
                    continue;
                }
                for frame in 0..SAMPLES {
                    let sample = if primary.custom_active() {
                        self.oscillators[0][index].generate_custom_step(
                            primary_shape,
                            phase_step,
                            primary.pulse_width,
                            settings.antialiasing,
                            primary.phase_warp.mode,
                            primary.phase_warp.amount,
                            primary.custom_curve,
                            primary.custom_mix,
                        )
                    } else {
                        self.oscillators[0][index].generate_shape_step(
                            primary_shapes
                                .as_ref()
                                .map_or(primary_shape, |shapes| shapes[frame]),
                            phase_step,
                            primary.pulse_width,
                            settings.antialiasing,
                        )
                    };
                    left[frame] += f32x8::splat(sample * self.unison.left[index] * 0.125);
                    right[frame] += f32x8::splat(sample * self.unison.right[index] * 0.125);
                }
            }
        } else {
            self.set_swarm_clock(swarm_clocks[0][0]);
            if !primary_motion_dynamic && self.swarm_update_remaining == 0 {
                let update_interval = self.swarm_update_interval();
                self.prepare_swarm_jitter_target(update_interval);
                self.swarm_update_remaining = update_interval;
            }
            let neutral_tune = primary.pitch_ratio.to_bits() == 1.0_f32.to_bits();
            let constant_ramp =
                !primary_motion_dynamic && usize::from(self.swarm_update_remaining) >= SAMPLES;
            let tuned_final_steps = if neutral_tune || !constant_ramp {
                None
            } else {
                constant_jitter_ramp_final::<SAMPLES>(
                    &self.phase_steps,
                    &self.swarm_pitch_step,
                    voice_count,
                    primary.pitch_ratio,
                )
            };
            if constant_ramp
                && !primary.custom_active()
                && (neutral_tune || tuned_final_steps.is_some())
            {
                for pack in 0..packs {
                    let index = pack * 8;
                    let dynamic_step = f32x8::from(std::array::from_fn(|lane| {
                        tuned_phase_step(self.phase_steps[index + lane], primary.pitch_ratio)
                    }));
                    let delta = f32x8::from(std::array::from_fn(|lane| {
                        self.swarm_pitch_step[index + lane] * primary.pitch_ratio
                    }));
                    let left_gain =
                        f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                    let right_gain =
                        f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                    let final_steps: [f32; 8] = if let (Some(shapes), Some(morph_gains)) =
                        (&primary_shapes, &primary_morph_gains)
                    {
                        let steps = std::array::from_fn(|frame| {
                            dynamic_step + delta * f32x8::splat((frame + 1) as f32)
                        });
                        accumulate_shape8_block_dynamic(
                            &mut self.oscillators[0][index..index + 8],
                            steps,
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            shapes,
                            morph_gains,
                            primary.pulse_width,
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
                                &mut self.oscillators[0][index..index + 8],
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
                                &mut self.oscillators[0][index..index + 8],
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
                        self.phase_steps[index..index + 8].copy_from_slice(&final_steps);
                    }
                }
                let mut tail_start = packs * 8;
                if voice_count - tail_start >= 4 {
                    let dynamic_step = f32x4::from(std::array::from_fn(|lane| {
                        tuned_phase_step(self.phase_steps[tail_start + lane], primary.pitch_ratio)
                    }));
                    let delta = f32x4::from(std::array::from_fn(|lane| {
                        self.swarm_pitch_step[tail_start + lane] * primary.pitch_ratio
                    }));
                    let left_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.unison.left[tail_start + lane]
                    }));
                    let right_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.unison.right[tail_start + lane]
                    }));
                    let final_steps: [f32; 4] = if let (Some(shapes), Some(morph_gains)) =
                        (&primary_shapes, &primary_morph_gains)
                    {
                        let steps = std::array::from_fn(|frame| {
                            dynamic_step + delta * f32x4::splat((frame + 1) as f32)
                        });
                        accumulate_shape4_block_dynamic(
                            &mut self.oscillators[0][tail_start..tail_start + 4],
                            steps,
                            left_gain,
                            right_gain,
                            &mut left,
                            &mut right,
                            shapes,
                            morph_gains,
                            primary.pulse_width,
                            settings.antialiasing,
                        );
                        steps[SAMPLES - 1].into()
                    } else {
                        accumulate_saw4_block_static_gains(
                            &mut self.oscillators[0][tail_start..tail_start + 4],
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
                        self.phase_steps[tail_start..tail_start + 4].copy_from_slice(&final_steps);
                    }
                    tail_start += 4;
                }
                let tail_lanes = voice_count - tail_start;
                if tail_lanes >= 2 {
                    let final_steps = self.accumulate_short_ramp(
                        tail_start,
                        tail_lanes,
                        primary,
                        primary_shapes.as_ref(),
                        primary_morph_gains.as_ref(),
                        settings.antialiasing,
                        &mut left,
                        &mut right,
                    );
                    if neutral_tune {
                        self.phase_steps[tail_start..voice_count]
                            .copy_from_slice(&final_steps[..tail_lanes]);
                    }
                    tail_start += tail_lanes;
                }
                for index in tail_start..voice_count {
                    let mut phase_step =
                        tuned_phase_step(self.phase_steps[index], primary.pitch_ratio);
                    let phase_step_delta = self.swarm_pitch_step[index] * primary.pitch_ratio;
                    for frame in 0..SAMPLES {
                        phase_step += phase_step_delta;
                        let sample = self.oscillators[0][index].generate_shape_step(
                            primary_shapes.as_ref().map_or(2.0, |shapes| shapes[frame]),
                            phase_step,
                            primary.pulse_width,
                            settings.antialiasing,
                        );
                        left[frame] += f32x8::splat(sample * self.unison.left[index] * 0.125);
                        right[frame] += f32x8::splat(sample * self.unison.right[index] * 0.125);
                    }
                    if neutral_tune {
                        self.phase_steps[index] = phase_step;
                    }
                }
                if let Some(final_steps) = tuned_final_steps {
                    self.phase_steps[..voice_count].copy_from_slice(&final_steps[..voice_count]);
                }
                self.swarm_update_remaining -= SAMPLES as u16;
                self.set_swarm_clock(swarm_clocks[0][SAMPLES - 2]);
                let output = std::array::from_fn(|frame| {
                    let gain = amplitude[frame] * self.unison.gain;
                    (
                        left[frame].reduce_add() * gain,
                        right[frame].reduce_add() * gain,
                    )
                });
                return self.finish_saw_block(
                    output,
                    &amplitude,
                    settings,
                    &swarm_clocks,
                    shapes,
                    motion,
                    motion_mask,
                );
            }

            let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
            let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
            for frame in 0..SAMPLES {
                if primary_motion_dynamic && let Some(motion) = motion {
                    self.configure_unison_motion_frame(motion[0][frame]);
                }
                self.set_swarm_clock(swarm_clocks[0][frame]);
                if self.unison.settings.motion_active() {
                    self.advance_swarm();
                }
                for pack in 0..packs {
                    let index = pack * 8;
                    steps[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                        tuned_phase_step(
                            self.lane_phase_step(index + lane, None),
                            primary.pitch_ratio,
                        )
                    }));
                }
                for (tail, index) in (packs * 8..voice_count).enumerate() {
                    tail_steps[tail][frame] =
                        tuned_phase_step(self.lane_phase_step(index, None), primary.pitch_ratio);
                }
            }
            for pack in 0..packs {
                let index = pack * 8;
                let left_gain =
                    f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                let right_gain =
                    f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                if primary.custom_active() {
                    accumulate_custom8_block(
                        &mut self.oscillators[0][index..index + 8],
                        steps[pack],
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary.custom_curve,
                        primary.custom_mix,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    );
                } else if let (Some(shapes), Some(morph_gains)) =
                    (&primary_shapes, &primary_morph_gains)
                {
                    accumulate_shape8_block_dynamic(
                        &mut self.oscillators[0][index..index + 8],
                        steps[pack],
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shapes,
                        morph_gains,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_saw8_block(
                        &mut self.oscillators[0][index..index + 8],
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
            if voice_count - tail_start >= 4 {
                let steps4 = std::array::from_fn(|frame| {
                    f32x4::from(std::array::from_fn(|lane| tail_steps[lane][frame]))
                });
                let left_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.left[tail_start + lane]
                }));
                let right_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.right[tail_start + lane]
                }));
                if primary.custom_active() {
                    accumulate_custom4_block(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps4,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary.custom_curve,
                        primary.custom_mix,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    );
                } else if let (Some(shapes), Some(morph_gains)) =
                    (&primary_shapes, &primary_morph_gains)
                {
                    accumulate_shape4_block_dynamic(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps4,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shapes,
                        morph_gains,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_saw4_block(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
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
            let tail_lanes = voice_count - tail_start;
            if tail_lanes >= 2 {
                let tail_offset = tail_start - packs * 8;
                self.accumulate_short_dynamic(
                    tail_start,
                    tail_lanes,
                    tail_offset,
                    primary,
                    primary_shape,
                    primary_shapes.as_ref(),
                    primary_morph_gains.as_ref(),
                    settings.antialiasing,
                    &tail_steps,
                    &mut left,
                    &mut right,
                );
                tail_start += tail_lanes;
            }
            for (tail, index) in (tail_start..voice_count).enumerate() {
                let tail = tail + tail_start - packs * 8;
                for frame in 0..SAMPLES {
                    let sample = if primary.custom_active() {
                        self.oscillators[0][index].generate_custom_step(
                            primary_shape,
                            tail_steps[tail][frame],
                            primary.pulse_width,
                            settings.antialiasing,
                            primary.phase_warp.mode,
                            primary.phase_warp.amount,
                            primary.custom_curve,
                            primary.custom_mix,
                        )
                    } else {
                        self.oscillators[0][index].generate_shape_step(
                            primary_shapes.as_ref().map_or(2.0, |shapes| shapes[frame]),
                            tail_steps[tail][frame],
                            primary.pulse_width,
                            settings.antialiasing,
                        )
                    };
                    left[frame] += f32x8::splat(sample * self.unison.left[index] * 0.125);
                    right[frame] += f32x8::splat(sample * self.unison.right[index] * 0.125);
                }
            }
        }

        let output = std::array::from_fn(|frame| {
            let gain = amplitude[frame] * self.unison.gain;
            (
                left[frame].reduce_add() * gain,
                right[frame].reduce_add() * gain,
            )
        });
        self.finish_saw_block(
            output,
            &amplitude,
            settings,
            &swarm_clocks,
            shapes,
            motion,
            motion_mask,
        )
    }

    #[cold]
    #[inline(never)]
    pub(super) fn accumulate_short_static<const SAMPLES: usize>(
        &mut self,
        index: usize,
        lanes: usize,
        oscillator: OscillatorSettings,
        shape: f32,
        shapes: Option<&[f32; SAMPLES]>,
        morph_gains: Option<&[f32; SAMPLES]>,
        antialiasing: Antialiasing,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        let steps = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| tuned_phase_step(self.phase_steps[index + lane], oscillator.pitch_ratio))
                .unwrap_or_default()
        }));
        let left_gain = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.unison.left[index + lane])
                .unwrap_or_default()
        }));
        let right_gain = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.unison.right[index + lane])
                .unwrap_or_default()
        }));
        let oscillators = &mut self.oscillators[0][index..index + 4];
        if let (Some(shapes), Some(morph_gains)) = (shapes, morph_gains) {
            accumulate_shape4_block_morphing(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                shapes,
                morph_gains,
                oscillator.pulse_width,
                antialiasing,
            );
        } else if oscillator.custom_active() {
            accumulate_custom4_block_constant(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                oscillator.custom_curve,
                oscillator.custom_mix,
                shape,
                oscillator.pulse_width,
                antialiasing,
                oscillator.phase_warp.mode,
                oscillator.phase_warp.amount,
            );
        } else if oscillator.phase_warp_active() {
            accumulate_shape4_block_constant_warped(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                shape,
                oscillator.pulse_width,
                antialiasing,
                oscillator.phase_warp.mode,
                oscillator.phase_warp.amount,
            );
        } else if (shape - 2.0).abs() <= f32::EPSILON {
            accumulate_saw4_block_constant(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                antialiasing,
            );
        } else {
            accumulate_shape4_block_constant(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                shape,
                oscillator.pulse_width,
                antialiasing,
            );
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn accumulate_short_ramp<const SAMPLES: usize>(
        &mut self,
        index: usize,
        lanes: usize,
        oscillator: OscillatorSettings,
        shapes: Option<&[f32; SAMPLES]>,
        morph_gains: Option<&[f32; SAMPLES]>,
        antialiasing: Antialiasing,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) -> [f32; 4] {
        let dynamic_step = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| tuned_phase_step(self.phase_steps[index + lane], oscillator.pitch_ratio))
                .unwrap_or_default()
        }));
        let delta = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.swarm_pitch_step[index + lane] * oscillator.pitch_ratio)
                .unwrap_or_default()
        }));
        let left_gain = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.unison.left[index + lane])
                .unwrap_or_default()
        }));
        let right_gain = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.unison.right[index + lane])
                .unwrap_or_default()
        }));
        let oscillators = &mut self.oscillators[0][index..index + 4];
        if let (Some(shapes), Some(morph_gains)) = (shapes, morph_gains) {
            let steps = std::array::from_fn(|frame| {
                dynamic_step + delta * f32x4::splat((frame + 1) as f32)
            });
            accumulate_shape4_block_dynamic(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                shapes,
                morph_gains,
                oscillator.pulse_width,
                antialiasing,
            );
            steps[SAMPLES - 1].into()
        } else {
            accumulate_saw4_block_static_gains(
                oscillators,
                dynamic_step,
                delta,
                left_gain,
                right_gain,
                left,
                right,
                antialiasing,
            )
            .into()
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn accumulate_short_dynamic<const SAMPLES: usize>(
        &mut self,
        index: usize,
        lanes: usize,
        tail_offset: usize,
        oscillator: OscillatorSettings,
        shape: f32,
        shapes: Option<&[f32; SAMPLES]>,
        morph_gains: Option<&[f32; SAMPLES]>,
        antialiasing: Antialiasing,
        tail_steps: &[[f32; SAMPLES]; 7],
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) {
        let steps = std::array::from_fn(|frame| {
            f32x4::from(std::array::from_fn(|lane| {
                (lane < lanes)
                    .then(|| tail_steps[tail_offset + lane][frame])
                    .unwrap_or_default()
            }))
        });
        let left_gain = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.unison.left[index + lane])
                .unwrap_or_default()
        }));
        let right_gain = f32x4::from(std::array::from_fn(|lane| {
            (lane < lanes)
                .then(|| self.unison.right[index + lane])
                .unwrap_or_default()
        }));
        let oscillators = &mut self.oscillators[0][index..index + 4];
        if oscillator.custom_active() {
            accumulate_custom4_block(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                oscillator.custom_curve,
                oscillator.custom_mix,
                shape,
                oscillator.pulse_width,
                antialiasing,
                oscillator.phase_warp.mode,
                oscillator.phase_warp.amount,
            );
        } else if let (Some(shapes), Some(morph_gains)) = (shapes, morph_gains) {
            accumulate_shape4_block_dynamic(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                shapes,
                morph_gains,
                oscillator.pulse_width,
                antialiasing,
            );
        } else {
            accumulate_saw4_block(
                oscillators,
                steps,
                left_gain,
                right_gain,
                left,
                right,
                antialiasing,
            );
        }
    }

    pub(in crate::voices) fn render_generic_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        std::array::from_fn(|frame| {
            if settings.oscillator(0).enabled {
                self.set_swarm_clock(swarm_clocks[0][frame]);
            }
            for oscillator in 1..LEGACY_OSCILLATOR_COUNT {
                if settings.oscillator(oscillator).enabled {
                    self.set_secondary_swarm_clock(oscillator, swarm_clocks[oscillator][frame]);
                }
            }
            self.render(settings, sample_rate, false)
        })
    }

    pub(in crate::voices) fn render_generic_block_with_static_oscillator_bank<
        const SAMPLES: usize,
    >(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
        shapes: Option<&[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        oscillator_bank: &ActiveOscillatorRenderSet,
        legacy_disabled: bool,
        settled_bank_config: bool,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(!oscillator_bank.transitioning());
        if legacy_disabled
            && settled_bank_config
            && self.settled_oscillator_bank_voice_eligible(oscillator_bank)
        {
            let entries = oscillator_bank.entries();
            let jittered = entries.iter().any(|entry| entry.current.jitter_active());
            if entries.len() == 2 && !jittered {
                let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
                if let Some((shape, pulse_width)) =
                    Self::structural_single_lane_pair(entries, timbre)
                {
                    return self.render_settled_two_oscillator_bank_block(
                        settings,
                        sample_rate,
                        entries,
                        shape,
                        pulse_width,
                    );
                }
                if let Some((lanes, shape, pulse_width)) =
                    Self::structural_small_unison_pair(entries, timbre)
                {
                    return self.render_settled_two_small_unison_oscillator_bank_block(
                        settings,
                        sample_rate,
                        entries,
                        lanes,
                        shape,
                        pulse_width,
                    );
                }
            }
            return self.render_settled_oscillator_bank_block(
                settings,
                sample_rate,
                oscillator_bank,
            );
        }
        if legacy_disabled {
            return std::array::from_fn(|_| {
                self.advance_envelope(sample_rate, false);
                self.advance_glide();
                self.render_oscillator_bank(
                    oscillator_bank,
                    settings,
                    sample_rate,
                    &StructuralOscillatorFrameControl::NEUTRAL,
                )
            });
        }
        std::array::from_fn(|frame| {
            if settings.oscillator(0).enabled {
                self.set_swarm_clock(swarm_clocks[0][frame]);
            }
            for oscillator in 1..LEGACY_OSCILLATOR_COUNT {
                if settings.oscillator(oscillator).enabled {
                    self.set_secondary_swarm_clock(oscillator, swarm_clocks[oscillator][frame]);
                }
            }
            let mut frame_settings = settings;
            if let Some(shapes) = shapes {
                for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                    if frame_settings.oscillators[oscillator].enabled {
                        frame_settings.oscillators[oscillator].shape = shapes[oscillator][frame];
                    }
                }
            }
            let (left, right) = self.render(frame_settings, sample_rate, false);
            let (bank_left, bank_right) = self.render_oscillator_bank(
                oscillator_bank,
                frame_settings,
                sample_rate,
                &StructuralOscillatorFrameControl::NEUTRAL,
            );
            (left + bank_left, right + bank_right)
        })
    }

    pub(in crate::voices) fn terminal_filter_block_eligible(
        &self,
        settings: VoiceSettings,
        oscillator_bank: &ActiveOscillatorRenderSet,
        envelope: EnvelopeSettings,
    ) -> bool {
        self.stage == EnvelopeStage::Sustain
            && self.settled_oscillator_bank_voice_eligible(oscillator_bank)
            && self.steady_voice_amplitude(settings, envelope.sustain) > f32::EPSILON
    }

    pub(in crate::voices) fn render_terminal_filter_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        oscillator_bank: &ActiveOscillatorRenderSet,
        group: &GeneratorRtGroup,
        filters: &[FilterCoefficients; MAX_FILTERS],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.terminal_filter_block_eligible(
            settings,
            oscillator_bank,
            self.envelope,
        ));
        let amplitude = self.steady_voice_amplitude(settings, self.envelope.sustain);
        let inverse_amplitude = amplitude.recip();
        let mut samples = self.render_generic_block_with_static_oscillator_bank(
            settings,
            sample_rate,
            [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
            None,
            oscillator_bank,
            true,
            true,
        );
        let modules = group.terminal_filters().unwrap_or_default();
        for module in modules {
            if let GeneratorRtModule::Filter(slot) = *module {
                let slot = slot.index();
                if filters[slot].is_phaser() {
                    self.filters[slot].prepare_phaser(filters[slot]);
                }
            }
        }
        for sample in &mut samples {
            let mut left = sample.0 * inverse_amplitude;
            let mut right = sample.1 * inverse_amplitude;
            for module in modules {
                if let GeneratorRtModule::Filter(slot) = *module {
                    let slot = slot.index();
                    (left, right) = if filters[slot].is_phaser() {
                        self.filters[slot].process_prepared_phaser(filters[slot], left, right)
                    } else {
                        self.filters[slot].process(filters[slot], left, right)
                    };
                }
            }
            sample.0 = left * amplitude;
            sample.1 = right * amplitude;
        }
        samples
    }

    pub(in crate::voices) fn render_terminal_filter_modulated_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        oscillator_bank: &ActiveOscillatorRenderSet,
        group: &GeneratorRtGroup,
        filters: &[[FilterCoefficients; MAX_FILTERS]],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert_eq!(filters.len(), SAMPLES);
        debug_assert!(self.terminal_filter_block_eligible(
            settings,
            oscillator_bank,
            self.envelope
        ));
        let amplitude = self.steady_voice_amplitude(settings, self.envelope.sustain);
        let inverse_amplitude = amplitude.recip();
        let mut samples = self.render_generic_block_with_static_oscillator_bank(
            settings,
            sample_rate,
            [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
            None,
            oscillator_bank,
            true,
            true,
        );
        let modules = group.terminal_filters().unwrap_or_default();
        for (frame, sample) in samples.iter_mut().enumerate() {
            let mut left = sample.0 * inverse_amplitude;
            let mut right = sample.1 * inverse_amplitude;
            for module in modules {
                if let GeneratorRtModule::Filter(slot) = *module {
                    let slot = slot.index();
                    (left, right) = self.filters[slot].process(filters[frame][slot], left, right);
                }
            }
            sample.0 = left * amplitude;
            sample.1 = right * amplitude;
        }
        samples
    }

    pub(in crate::voices) fn render_terminal_filter_voice_job<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        oscillator_bank: &ActiveOscillatorRenderSet,
        group: &GeneratorRtGroup,
        base_filters: &[FilterConfig; MAX_FILTERS],
        shared_filters: &[FilterCoefficients; MAX_FILTERS],
        program: Option<&VoiceLfoProgram>,
        routes: Option<&VoiceStructuralRouteFrame>,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.terminal_filter_block_eligible(
            settings,
            oscillator_bank,
            self.envelope,
        ));
        debug_assert_eq!(program.is_some(), routes.is_some());
        let amplitude = self.steady_voice_amplitude(settings, self.envelope.sustain);
        let inverse_amplitude = amplitude.recip();
        let mut samples = self.render_generic_block_with_static_oscillator_bank(
            settings,
            sample_rate,
            [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
            None,
            oscillator_bank,
            true,
            true,
        );
        let mut structural = crate::StructuralModulationFrame::default();
        let modules = group.terminal_filters().unwrap_or_default();
        if program.is_none() {
            for module in modules {
                if let GeneratorRtModule::Filter(slot) = *module {
                    let slot = slot.index();
                    if shared_filters[slot].is_phaser() {
                        self.filters[slot].prepare_phaser(shared_filters[slot]);
                    }
                }
            }
        }
        if let (Some(program), Some((source, amount, route_slot, control))) = (
            program,
            routes.and_then(VoiceStructuralRouteFrame::single_filter_route),
        ) {
            let amount = amount.clamp(-1.0, 1.0);
            let route_slot_index = usize::from(route_slot);
            let prepared_phaser_resonance = control == crate::FilterControl::Resonance
                && shared_filters[route_slot_index].is_phaser();
            let prepared_phaser_shape = control == crate::FilterControl::Shape
                && shared_filters[route_slot_index].is_phaser();
            let prepared_svf_resonance = control == crate::FilterControl::Resonance
                && shared_filters[route_slot_index].is_svf();
            let direct_scream_resonance = control == crate::FilterControl::Resonance
                && shared_filters[route_slot_index].is_scream();
            let direct_scream_shape = matches!(
                control,
                crate::FilterControl::Slope | crate::FilterControl::Morph
            ) && shared_filters[route_slot_index].is_scream();
            if prepared_phaser_resonance || prepared_phaser_shape {
                self.filters[route_slot_index].prepare_phaser(shared_filters[route_slot_index]);
            }
            if prepared_svf_resonance {
                self.filters[route_slot_index].prepare_svf(shared_filters[route_slot_index]);
            }
            if direct_scream_resonance {
                for sample in &mut samples {
                    let value = self.modulation.next(program)[usize::from(source)] * amount;
                    let mut left = sample.0 * inverse_amplitude;
                    let mut right = sample.1 * inverse_amplitude;
                    for module in modules {
                        if let GeneratorRtModule::Filter(slot) = *module {
                            let slot = slot.index();
                            (left, right) = if route_slot_index == slot {
                                self.filters[slot].process_scream_resonance(
                                    &shared_filters[slot],
                                    value * 4.0,
                                    left,
                                    right,
                                )
                            } else {
                                self.filters[slot].process(shared_filters[slot], left, right)
                            };
                        }
                    }
                    sample.0 = left * amplitude;
                    sample.1 = right * amplitude;
                }
                return samples;
            }
            for sample in &mut samples {
                let value = self.modulation.next(program)[usize::from(source)] * amount;
                let mut left = sample.0 * inverse_amplitude;
                let mut right = sample.1 * inverse_amplitude;
                for module in modules {
                    if let GeneratorRtModule::Filter(slot) = *module {
                        let slot = slot.index();
                        if prepared_phaser_resonance && route_slot_index == slot {
                            (left, right) = self.filters[slot].process_prepared_phaser_resonance(
                                &shared_filters[slot],
                                value * 4.0,
                                left,
                                right,
                            );
                            continue;
                        }
                        if prepared_phaser_shape && route_slot_index == slot {
                            (left, right) = self.filters[slot].process_prepared_phaser_shape(
                                &shared_filters[slot],
                                value,
                                left,
                                right,
                            );
                            continue;
                        }
                        if prepared_svf_resonance && route_slot_index == slot {
                            (left, right) = self.filters[slot].process_prepared_svf_resonance(
                                &shared_filters[slot],
                                value * 4.0,
                                left,
                                right,
                            );
                            continue;
                        }
                        if direct_scream_shape && route_slot_index == slot {
                            (left, right) = match control {
                                crate::FilterControl::Slope => self.filters[slot]
                                    .process_scream_slope(
                                        &shared_filters[slot],
                                        value,
                                        left,
                                        right,
                                    ),
                                crate::FilterControl::Morph => self.filters[slot]
                                    .process_scream_morph(
                                        &shared_filters[slot],
                                        value,
                                        left,
                                        right,
                                    ),
                                crate::FilterControl::Cutoff
                                | crate::FilterControl::Resonance
                                | crate::FilterControl::Shape => {
                                    unreachable!()
                                }
                            };
                            continue;
                        }
                        let coefficients = if route_slot == slot as u8 {
                            match control {
                                crate::FilterControl::Cutoff => {
                                    shared_filters[slot].modulated_cutoff(value * 4.0)
                                }
                                crate::FilterControl::Resonance => {
                                    shared_filters[slot].modulated_resonance(value * 4.0)
                                }
                                crate::FilterControl::Slope => {
                                    shared_filters[slot].modulated_slope(value)
                                }
                                crate::FilterControl::Morph => {
                                    shared_filters[slot].modulated_morph(value)
                                }
                                crate::FilterControl::Shape => {
                                    shared_filters[slot].modulated_shape(value)
                                }
                            }
                        } else {
                            shared_filters[slot]
                        };
                        (left, right) = self.filters[slot].process(coefficients, left, right);
                    }
                }
                sample.0 = left * amplitude;
                sample.1 = right * amplitude;
            }
            return samples;
        }
        for sample in &mut samples {
            if let (Some(program), Some(routes)) = (program, routes) {
                routes.evaluate(self.modulation.next(program), &mut structural);
            }
            let mut left = sample.0 * inverse_amplitude;
            let mut right = sample.1 * inverse_amplitude;
            for module in modules {
                if let GeneratorRtModule::Filter(slot) = *module {
                    let slot = slot.index();
                    let coefficients = if program.is_some() {
                        let delta = if structural.filter_mask & (1 << slot) != 0 {
                            structural.filters[slot]
                        } else {
                            crate::StructuralFilterDelta::default()
                        };
                        voice_filter_coefficient(
                            base_filters[slot],
                            shared_filters[slot],
                            delta,
                            sample_rate,
                        )
                    } else {
                        shared_filters[slot]
                    };
                    (left, right) = if program.is_none() && coefficients.is_phaser() {
                        self.filters[slot].process_prepared_phaser(coefficients, left, right)
                    } else {
                        self.filters[slot].process(coefficients, left, right)
                    };
                }
            }
            sample.0 = left * amplitude;
            sample.1 = right * amplitude;
        }
        samples
    }

    pub(in crate::voices) fn copy_terminal_filter_state_from(
        &mut self,
        source: &Self,
        group: &GeneratorRtGroup,
        coefficients: Option<&[FilterCoefficients; MAX_FILTERS]>,
    ) {
        for module in group.terminal_filters().unwrap_or_default() {
            if let GeneratorRtModule::Filter(slot) = *module {
                let slot = slot.index();
                if let Some(coefficients) = coefficients {
                    self.filters[slot]
                        .copy_static_state_from(&source.filters[slot], coefficients[slot]);
                } else {
                    self.filters[slot] = source.filters[slot];
                }
            }
        }
    }
}
