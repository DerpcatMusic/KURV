//! Compiler boundary for a user-written VA waveform function.

use crate::wave_curve::function::{VaFunctionRt, compile_expression};

pub const DEFAULT_VA_FUNCTION: &str = "sin(tau*x)";
const MAX_FUNCTION_CHARS: usize = 256;

pub(crate) fn compile_va_function(expression: &str) -> Result<VaFunctionRt, String> {
    if expression.chars().count() > MAX_FUNCTION_CHARS {
        return Err(format!("function limit is {MAX_FUNCTION_CHARS} characters"));
    }
    compile_expression(expression, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_compiles_once_and_uses_phase_and_wave_position() {
        let function =
            compile_va_function("sin(tau*x)*(1-w)+cos(tau*x)*w").expect("valid expression");

        assert!((function.at(0.0).eval(0.25) - 1.0).abs() < 1.0e-5);
        assert!(function.at(1.0).eval(0.25).abs() < 1.0e-5);
        let phases =
            truce_simd::simd::f32x8::from([0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875]);
        let vector: [f32; 8] = function.at(0.0).eval8(phases).into();
        for (phase, sample) in <[f32; 8]>::from(phases).into_iter().zip(vector) {
            assert!((sample - function.at(0.0).eval(phase)).abs() < 1.0e-5);
        }
        assert!(compile_va_function("sin(").is_err());
    }
}
