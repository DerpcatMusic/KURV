use crate::*;

#[derive(Clone, Copy)]
pub(crate) struct WaveCurveTransition {
    previous: WaveCurveRt,
    current: WaveCurveRt,
    progress: f32,
}

impl Default for WaveCurveTransition {
    fn default() -> Self {
        Self::new(WaveCurveRt::default())
    }
}

impl WaveCurveTransition {
    pub(crate) const fn new(curve: WaveCurveRt) -> Self {
        Self {
            previous: curve,
            current: curve,
            progress: 1.0,
        }
    }

    pub(crate) fn retarget(&mut self, curve: WaveCurveRt, audible: bool) {
        if curve != self.current {
            self.previous = WaveCurveRt::interpolate(self.previous, self.current, self.progress);
            self.current = curve;
            self.progress = if audible { 0.0 } else { 1.0 };
        }
    }

    pub(crate) fn value(self, progress: f32) -> WaveCurveRt {
        if progress >= 1.0 {
            self.current
        } else {
            WaveCurveRt::interpolate(self.previous, self.current, progress)
        }
    }

    pub(crate) fn advance(&mut self, step: f32) -> f32 {
        self.progress = (self.progress + step).min(1.0);
        self.progress
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VaTableTransition {
    previous: VaTableRt,
    current: VaTableRt,
    pending: VaTableRt,
    pending_valid: bool,
    progress: f32,
}

impl Default for VaTableTransition {
    fn default() -> Self {
        Self {
            previous: VaTableRt::default(),
            current: VaTableRt::default(),
            pending: VaTableRt::default(),
            pending_valid: false,
            progress: 1.0,
        }
    }
}

impl VaTableTransition {
    pub(crate) fn retarget(&mut self, table: &VaTableRt, audible: bool) {
        if !audible {
            self.previous.clone_from(table);
            self.current.clone_from(table);
            self.pending_valid = false;
            self.progress = 1.0;
        } else if self.progress < 1.0 {
            if table == &self.current {
                self.pending_valid = false;
            } else {
                self.pending.clone_from(table);
                self.pending_valid = true;
            }
        } else if table != &self.current {
            self.previous.clone_from(&self.current);
            self.current.clone_from(table);
            self.progress = 0.0;
        }
    }

    pub(crate) fn advance(&mut self, sample_rate: f32) {
        if self.progress >= 1.0 {
            return;
        }
        self.progress = (self.progress + 1.0 / (sample_rate * 0.004).max(1.0)).min(1.0);
        if self.progress >= 1.0 && self.pending_valid {
            self.previous.clone_from(&self.current);
            self.current.clone_from(&self.pending);
            self.pending_valid = false;
            self.progress = 0.0;
        }
    }

    pub(crate) fn select(
        &self,
        base: WaveCurveRt,
        legacy_position: f32,
        wave_position: f32,
    ) -> (WaveCurveRt, f32, f32) {
        let current = self.current.select(base, legacy_position, wave_position);
        if self.progress >= 1.0 {
            return (current.curve, current.mix, current.shape);
        }
        let previous = self.previous.select(base, legacy_position, wave_position);
        (
            WaveCurveRt::interpolate(previous.curve, current.curve, self.progress),
            (current.mix - previous.mix).mul_add(self.progress, previous.mix),
            (current.shape - previous.shape).mul_add(self.progress, previous.shape),
        )
    }

    pub(crate) fn active(&self) -> bool {
        self.progress < 1.0
    }
}
