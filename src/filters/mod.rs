//! Fixed-size realtime filter cores.

mod engine;
mod spectral;

pub(crate) use engine::FilterCoefficients;
pub(crate) use engine::prepare;
pub use engine::{FilterConfig, FilterDomain, FilterMode, StereoTptSvf};
pub(crate) use engine::{
    MAX_Q, MAX_SLOPE_DB, MIN_Q, MIN_SLOPE_DB, OBJECT_MAX_DECAY, OBJECT_MIN_DECAY,
};
pub(crate) use spectral::{
    MAX_RATIO, MIN_RATIO, denormalized_ratio, normalized_ratio, ratio_brickwall_bypassed,
};
