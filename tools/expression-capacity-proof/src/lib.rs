#![allow(dead_code)]
// The production truce_simd module reexports these wide vector types.
extern crate self as truce_simd;
pub mod simd {
    pub use wide::{f32x4, f32x8};
}
#[path = "../../../src/wave_curve/function.rs"]
mod function;
