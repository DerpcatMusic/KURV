use super::super::declick::GainDeclicker;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeSettings {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub attack_curve: f32,
    pub decay_curve: f32,
    pub release_curve: f32,
    pub attack_curve_time: f32,
    pub decay_curve_time: f32,
    pub release_curve_time: f32,
}

impl Default for EnvelopeSettings {
    fn default() -> Self {
        Self {
            attack: 0.0,
            decay: 0.1,
            sustain: 1.0,
            release: 0.0,
            attack_curve: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            attack_curve_time: 0.0,
            decay_curve_time: 0.0,
            release_curve_time: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy)]
pub(super) struct GroupVoiceEnvelope {
    settings: EnvelopeSettings,
    pub(super) level: f32,
    start: f32,
    progress: f32,
    step: f32,
    stage: EnvelopeStage,
    declicker: GainDeclicker,
}

impl Default for GroupVoiceEnvelope {
    fn default() -> Self {
        Self {
            settings: EnvelopeSettings::default(),
            level: 0.0,
            start: 0.0,
            progress: 0.0,
            step: 1.0,
            stage: EnvelopeStage::Idle,
            declicker: GainDeclicker::default(),
        }
    }
}

impl GroupVoiceEnvelope {
    pub(super) fn configure(&mut self, settings: EnvelopeSettings, sample_rate: f32) {
        if self.settings == settings {
            return;
        }
        let duration_changed = match self.stage {
            EnvelopeStage::Attack => self.settings.attack.to_bits() != settings.attack.to_bits(),
            EnvelopeStage::Decay => self.settings.decay.to_bits() != settings.decay.to_bits(),
            EnvelopeStage::Release => self.settings.release.to_bits() != settings.release.to_bits(),
            EnvelopeStage::Idle | EnvelopeStage::Sustain => false,
        };
        self.settings = settings;
        if duration_changed {
            self.refresh_step(sample_rate);
        }
    }

    pub(super) fn note_on(&mut self, sample_rate: f32) {
        if self.settings.attack <= 0.0 {
            let previous = self.level;
            self.level = 1.0;
            self.declicker.insert(self.level - previous);
            self.begin_decay(sample_rate);
        } else {
            self.begin_stage(EnvelopeStage::Attack, sample_rate);
            let remaining = (1.0 - self.start).max(f32::EPSILON);
            self.step = 1.0 / (self.settings.attack * sample_rate.max(1.0) * remaining).max(1.0);
        }
    }

    pub(super) fn note_off(&mut self, sample_rate: f32) {
        if self.stage == EnvelopeStage::Idle {
            return;
        }
        if self.settings.release <= 0.0 {
            self.declicker.insert(-self.level);
            self.finish_logical();
        } else {
            self.begin_stage(EnvelopeStage::Release, sample_rate);
        }
    }

    pub(super) fn advance(&mut self, sample_rate: f32) {
        match self.stage {
            EnvelopeStage::Idle => self.level = 0.0,
            EnvelopeStage::Attack => {
                self.advance_progress();
                self.level = shaped_progress(
                    self.progress,
                    self.settings.attack_curve_time,
                    self.settings.attack_curve,
                )
                .mul_add(1.0 - self.start, self.start);
                if self.progress >= 1.0 {
                    self.level = 1.0;
                    self.begin_decay(sample_rate);
                }
            }
            EnvelopeStage::Decay => {
                let sustain = self.settings.sustain.clamp(0.0, 1.0);
                if sustain >= self.start {
                    self.level = sustain;
                    self.stage = EnvelopeStage::Sustain;
                } else {
                    self.advance_progress();
                    self.level = shaped_progress(
                        self.progress,
                        self.settings.decay_curve_time,
                        self.settings.decay_curve,
                    )
                    .mul_add(sustain - self.start, self.start);
                    if self.progress >= 1.0 {
                        self.level = sustain;
                        self.stage = EnvelopeStage::Sustain;
                    }
                }
            }
            EnvelopeStage::Sustain => {
                self.level = self.settings.sustain.clamp(0.0, 1.0);
            }
            EnvelopeStage::Release => {
                self.advance_progress();
                self.level =
                    (1.0 - shaped_progress(
                        self.progress,
                        self.settings.release_curve_time,
                        self.settings.release_curve,
                    )) * self.start;
                if self.progress >= 1.0 || self.level <= 1.0e-5 {
                    self.finish();
                }
            }
        }
        self.declicker.advance();
    }

    fn begin_decay(&mut self, sample_rate: f32) {
        let sustain = self.settings.sustain.clamp(0.0, 1.0);
        if sustain >= self.level || self.settings.decay <= 0.0 {
            self.level = sustain;
            self.stage = EnvelopeStage::Sustain;
        } else {
            self.begin_stage(EnvelopeStage::Decay, sample_rate);
        }
    }

    fn begin_stage(&mut self, stage: EnvelopeStage, sample_rate: f32) {
        self.stage = stage;
        self.start = self.level;
        self.progress = 0.0;
        self.refresh_step(sample_rate);
    }

    pub(super) fn refresh_step(&mut self, sample_rate: f32) {
        let seconds = match self.stage {
            EnvelopeStage::Attack => self.settings.attack,
            EnvelopeStage::Decay => self.settings.decay,
            EnvelopeStage::Release => self.settings.release,
            EnvelopeStage::Idle | EnvelopeStage::Sustain => {
                self.step = 1.0;
                return;
            }
        };
        self.step = 1.0 / (seconds.max(f32::EPSILON) * sample_rate.max(1.0)).max(1.0);
    }

    #[inline]
    fn advance_progress(&mut self) {
        self.progress = (self.progress + self.step).min(1.0);
    }

    pub(super) fn finish(&mut self) {
        self.declicker.reset();
        self.finish_logical();
    }

    fn finish_logical(&mut self) {
        self.level = 0.0;
        self.start = 0.0;
        self.progress = 0.0;
        self.step = 1.0;
        self.stage = EnvelopeStage::Idle;
    }

    pub(super) fn active(self) -> bool {
        self.stage != EnvelopeStage::Idle || self.declicker.active()
    }

    pub(super) fn is_sustaining(self) -> bool {
        self.stage == EnvelopeStage::Sustain && !self.declicker.active()
    }

    pub(super) fn gain(self) -> f32 {
        self.level + self.declicker.correction()
    }
}

#[inline]
pub(super) fn shaped_progress(progress: f32, curve_time: f32, curve_level: f32) -> f32 {
    crate::dsp::curve_progress(
        crate::dsp::curve_progress(progress, -curve_time),
        curve_level,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_attack_curve_update_changes_the_next_voice_envelope_sample() {
        let sample_rate = 8.0;
        let settings = EnvelopeSettings {
            attack: 1.0,
            attack_curve: -1.0,
            ..EnvelopeSettings::default()
        };
        let mut unchanged = GroupVoiceEnvelope::default();
        unchanged.configure(settings, sample_rate);
        unchanged.note_on(sample_rate);
        let mut modulated = unchanged;
        for _ in 0..2 {
            unchanged.advance(sample_rate);
            modulated.advance(sample_rate);
        }
        modulated.configure(
            EnvelopeSettings {
                attack_curve: 1.0,
                ..settings
            },
            sample_rate,
        );
        unchanged.advance(sample_rate);
        modulated.advance(sample_rate);
        assert_ne!(unchanged.level.to_bits(), modulated.level.to_bits());
    }
}
