pub(crate) const MIN_RATIO: f32 = 0.0;
pub(crate) const MAX_RATIO: f32 = 1_024.0;
const RATIO_CURVE_OFFSET: f32 = 4.0;

#[must_use]
pub(crate) fn normalized_ratio(value: f32) -> f32 {
    (value.clamp(MIN_RATIO, MAX_RATIO) / RATIO_CURVE_OFFSET).ln_1p()
        / (MAX_RATIO / RATIO_CURVE_OFFSET).ln_1p()
}

#[must_use]
pub(crate) fn denormalized_ratio(normalized: f32) -> f32 {
    RATIO_CURVE_OFFSET
        * (normalized.clamp(0.0, 1.0) * (MAX_RATIO / RATIO_CURVE_OFFSET).ln_1p()).exp_m1()
}

#[must_use]
pub(crate) fn ratio_brickwall_bypassed(cutoff: f32, lowpass: bool) -> bool {
    cutoff <= f32::EPSILON || lowpass && cutoff >= MAX_RATIO
}
