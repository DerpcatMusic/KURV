//! Fixed, allocation-free evaluator for a compiled waveform expression.

use truce_simd::simd::{f32x4, f32x8};

const MAX_USER_INSTRUCTIONS: usize = 32;
const MAX_INSTRUCTIONS: usize = MAX_USER_INSTRUCTIONS * 2 + 5;
const MAX_STACK: usize = 16;
pub const FUNCTION_RT_VALUES: usize = 2 + MAX_INSTRUCTIONS * 2;

const PUSH_X: u8 = 1;
const PUSH_W: u8 = 2;
const PUSH_CONSTANT: u8 = 3;
const ADD: u8 = 4;
const SUBTRACT: u8 = 5;
const MULTIPLY: u8 = 6;
const DIVIDE: u8 = 7;
const NEGATE: u8 = 8;
const SIN: u8 = 9;
const COS: u8 = 10;
const ABS: u8 = 11;
const FLOOR: u8 = 12;
const FRACT: u8 = 13;
const SQRT: u8 = 14;
const MIN: u8 = 15;
const MAX: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VaFunctionRt {
    words: [f32; FUNCTION_RT_VALUES],
}

impl VaFunctionRt {
    pub const fn disabled() -> Self {
        Self {
            words: [0.0; FUNCTION_RT_VALUES],
        }
    }

    pub const fn words(self) -> [f32; FUNCTION_RT_VALUES] {
        self.words
    }

    pub const fn from_words(words: [f32; FUNCTION_RT_VALUES]) -> Self {
        Self { words }
    }

    pub fn at(self, position: f32) -> Self {
        let mut result = self;
        let position = finite_or_zero(position).clamp(0.0, 1.0);
        for index in 0..self.len() {
            let base = 2 + index * 2;
            if self.words[base] as u8 == PUSH_W {
                result.words[base] = f32::from(PUSH_CONSTANT);
                result.words[base + 1] = position;
            }
        }
        result
    }

    pub fn interpolate(previous: Self, current: Self, mix: f32) -> Self {
        if !previous.enabled() {
            return current;
        }
        if !current.enabled() {
            return previous;
        }
        let mix = finite_or_zero(mix).clamp(0.0, 1.0);
        let mut result = Self::disabled();
        result.words[0] = 1.0;
        let mut target = 0;
        target = result.append(&previous, target);
        result.write(target, PUSH_CONSTANT, 1.0 - mix);
        result.write(target + 1, MULTIPLY, 0.0);
        target += 2;
        target = result.append(&current, target);
        result.write(target, PUSH_CONSTANT, mix);
        result.write(target + 1, MULTIPLY, 0.0);
        result.write(target + 2, ADD, 0.0);
        result.words[1] = (target + 3) as f32;
        result
    }

    pub fn enabled(&self) -> bool {
        self.words[0] >= 0.5
    }

    fn len(&self) -> usize {
        (self.words[1] as usize).min(MAX_INSTRUCTIONS)
    }

    fn append(&mut self, source: &Self, mut target: usize) -> usize {
        for index in 0..source.len() {
            let source_base = 2 + index * 2;
            self.write(
                target,
                source.words[source_base] as u8,
                source.words[source_base + 1],
            );
            target += 1;
        }
        target
    }

    fn write(&mut self, index: usize, opcode: u8, value: f32) {
        let base = 2 + index * 2;
        self.words[base] = f32::from(opcode);
        self.words[base + 1] = value;
    }

    #[inline]
    pub fn eval(&self, phase: f32) -> f32 {
        let mut stack = [0.0; MAX_STACK];
        let mut depth = 0;
        for index in 0..self.len() {
            let base = 2 + index * 2;
            match self.words[base] as u8 {
                PUSH_X => push(&mut stack, &mut depth, phase.rem_euclid(1.0)),
                PUSH_CONSTANT => push(&mut stack, &mut depth, self.words[base + 1]),
                ADD => binary(&mut stack, &mut depth, |a, b| a + b),
                SUBTRACT => binary(&mut stack, &mut depth, |a, b| a - b),
                MULTIPLY => binary(&mut stack, &mut depth, |a, b| a * b),
                DIVIDE => binary(&mut stack, &mut depth, |a, b| a / b),
                NEGATE => unary(&mut stack, depth, |value| -value),
                SIN => unary(&mut stack, depth, f32::sin),
                COS => unary(&mut stack, depth, f32::cos),
                ABS => unary(&mut stack, depth, f32::abs),
                FLOOR => unary(&mut stack, depth, f32::floor),
                FRACT => unary(&mut stack, depth, f32::fract),
                SQRT => unary(&mut stack, depth, f32::sqrt),
                MIN => binary(&mut stack, &mut depth, f32::min),
                MAX => binary(&mut stack, &mut depth, f32::max),
                _ => {}
            }
        }
        finite_or_zero(stack[depth.saturating_sub(1)]).clamp(-1.0, 1.0)
    }

    #[inline]
    pub fn eval4(&self, phase: f32x4) -> f32x4 {
        let mut stack = [f32x4::ZERO; MAX_STACK];
        let mut depth = 0;
        for index in 0..self.len() {
            let base = 2 + index * 2;
            match self.words[base] as u8 {
                PUSH_X => push(&mut stack, &mut depth, phase - phase.floor()),
                PUSH_CONSTANT => push(&mut stack, &mut depth, f32x4::splat(self.words[base + 1])),
                ADD => binary(&mut stack, &mut depth, |a, b| a + b),
                SUBTRACT => binary(&mut stack, &mut depth, |a, b| a - b),
                MULTIPLY => binary(&mut stack, &mut depth, |a, b| a * b),
                DIVIDE => binary(&mut stack, &mut depth, |a, b| a / b),
                NEGATE => unary(&mut stack, depth, |value| -value),
                SIN => unary(&mut stack, depth, f32x4::sin),
                COS => unary(&mut stack, depth, f32x4::cos),
                ABS => unary(&mut stack, depth, f32x4::abs),
                FLOOR => unary(&mut stack, depth, f32x4::floor),
                FRACT => unary(&mut stack, depth, |value| value - value.floor()),
                SQRT => unary(&mut stack, depth, f32x4::sqrt),
                MIN => binary(&mut stack, &mut depth, f32x4::fast_min),
                MAX => binary(&mut stack, &mut depth, f32x4::fast_max),
                _ => {}
            }
        }
        sanitize4(stack[depth.saturating_sub(1)])
    }

    #[inline]
    pub fn eval8(&self, phase: f32x8) -> f32x8 {
        let mut stack = [f32x8::ZERO; MAX_STACK];
        let mut depth = 0;
        for index in 0..self.len() {
            let base = 2 + index * 2;
            match self.words[base] as u8 {
                PUSH_X => push(&mut stack, &mut depth, phase - phase.floor()),
                PUSH_CONSTANT => push(&mut stack, &mut depth, f32x8::splat(self.words[base + 1])),
                ADD => binary(&mut stack, &mut depth, |a, b| a + b),
                SUBTRACT => binary(&mut stack, &mut depth, |a, b| a - b),
                MULTIPLY => binary(&mut stack, &mut depth, |a, b| a * b),
                DIVIDE => binary(&mut stack, &mut depth, |a, b| a / b),
                NEGATE => unary(&mut stack, depth, |value| -value),
                SIN => unary(&mut stack, depth, f32x8::sin),
                COS => unary(&mut stack, depth, f32x8::cos),
                ABS => unary(&mut stack, depth, f32x8::abs),
                FLOOR => unary(&mut stack, depth, f32x8::floor),
                FRACT => unary(&mut stack, depth, |value| value - value.floor()),
                SQRT => unary(&mut stack, depth, f32x8::sqrt),
                MIN => binary(&mut stack, &mut depth, f32x8::fast_min),
                MAX => binary(&mut stack, &mut depth, f32x8::fast_max),
                _ => {}
            }
        }
        sanitize8(stack[depth.saturating_sub(1)])
    }
}

impl Default for VaFunctionRt {
    fn default() -> Self {
        Self::disabled()
    }
}

pub fn compile_expression(source: &str, enabled: bool) -> Result<VaFunctionRt, String> {
    let mut parser = Parser {
        source: source.as_bytes(),
        cursor: 0,
        instructions: Vec::new(),
        depth: 0,
        peak_depth: 0,
    };
    parser.expression()?;
    parser.whitespace();
    if parser.cursor != parser.source.len() {
        return Err(format!("unexpected input at column {}", parser.cursor + 1));
    }
    if parser.depth != 1 {
        return Err("function must produce one value".to_owned());
    }
    let mut function = VaFunctionRt::disabled();
    function.words[0] = if enabled { 1.0 } else { 0.0 };
    function.words[1] = parser.instructions.len() as f32;
    for (index, instruction) in parser.instructions.into_iter().enumerate() {
        let base = 2 + index * 2;
        function.words[base] = f32::from(instruction.opcode);
        function.words[base + 1] = instruction.value;
    }
    Ok(function)
}

#[derive(Clone, Copy)]
struct Instruction {
    opcode: u8,
    value: f32,
}

struct Parser<'a> {
    source: &'a [u8],
    cursor: usize,
    instructions: Vec<Instruction>,
    depth: usize,
    peak_depth: usize,
}

impl Parser<'_> {
    fn expression(&mut self) -> Result<(), String> {
        self.sum()
    }

    fn sum(&mut self) -> Result<(), String> {
        self.product()?;
        loop {
            if self.consume(b'+') {
                self.product()?;
                self.emit_binary(ADD)?;
            } else if self.consume(b'-') {
                self.product()?;
                self.emit_binary(SUBTRACT)?;
            } else {
                return Ok(());
            }
        }
    }

    fn product(&mut self) -> Result<(), String> {
        self.unary()?;
        loop {
            if self.consume(b'*') {
                self.unary()?;
                self.emit_binary(MULTIPLY)?;
            } else if self.consume(b'/') {
                self.unary()?;
                self.emit_binary(DIVIDE)?;
            } else {
                return Ok(());
            }
        }
    }

    fn unary(&mut self) -> Result<(), String> {
        if self.consume(b'-') {
            self.unary()?;
            return self.emit(NEGATE, 0.0, 0);
        }
        if self.consume(b'+') {
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<(), String> {
        self.whitespace();
        if self.consume(b'(') {
            self.expression()?;
            self.expect(b')')?;
            return Ok(());
        }
        if self
            .peek()
            .is_some_and(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            return self.number();
        }
        let start = self.cursor;
        while self.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
            self.cursor += 1;
        }
        if start == self.cursor {
            return Err(format!("expected a value at column {}", self.cursor + 1));
        }
        let name = std::str::from_utf8(&self.source[start..self.cursor])
            .unwrap_or_default()
            .to_owned();
        match name.as_str() {
            "x" => self.emit(PUSH_X, 0.0, 1),
            "w" => self.emit(PUSH_W, 0.0, 1),
            "pi" => self.emit(PUSH_CONSTANT, std::f32::consts::PI, 1),
            "tau" => self.emit(PUSH_CONSTANT, std::f32::consts::TAU, 1),
            "sin" | "cos" | "abs" | "floor" | "fract" | "sqrt" => {
                self.expect(b'(')?;
                self.expression()?;
                self.expect(b')')?;
                let opcode = match name.as_str() {
                    "sin" => SIN,
                    "cos" => COS,
                    "abs" => ABS,
                    "floor" => FLOOR,
                    "fract" => FRACT,
                    _ => SQRT,
                };
                self.emit(opcode, 0.0, 0)
            }
            "min" | "max" => {
                self.expect(b'(')?;
                self.expression()?;
                self.expect(b',')?;
                self.expression()?;
                self.expect(b')')?;
                self.emit_binary(if name == "min" { MIN } else { MAX })
            }
            _ => Err(format!("unknown name `{name}`")),
        }
    }

    fn number(&mut self) -> Result<(), String> {
        self.whitespace();
        let start = self.cursor;
        while self.peek().is_some_and(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-')
        }) {
            if self.cursor > start
                && matches!(self.source[self.cursor], b'+' | b'-')
                && !matches!(self.source[self.cursor - 1], b'e' | b'E')
            {
                break;
            }
            self.cursor += 1;
        }
        let value = std::str::from_utf8(&self.source[start..self.cursor])
            .ok()
            .and_then(|text| text.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("invalid number at column {}", start + 1))?;
        self.emit(PUSH_CONSTANT, value, 1)
    }

    fn emit_binary(&mut self, opcode: u8) -> Result<(), String> {
        if self.depth < 2 {
            return Err("operator is missing a value".to_owned());
        }
        self.emit(opcode, 0.0, -1)
    }

    fn emit(&mut self, opcode: u8, value: f32, depth_change: isize) -> Result<(), String> {
        if self.instructions.len() == MAX_USER_INSTRUCTIONS {
            return Err(format!(
                "function is limited to {MAX_USER_INSTRUCTIONS} operations"
            ));
        }
        self.depth = self.depth.saturating_add_signed(depth_change);
        self.peak_depth = self.peak_depth.max(self.depth);
        if self.peak_depth > MAX_STACK {
            return Err(format!("function stack is limited to {MAX_STACK} values"));
        }
        self.instructions.push(Instruction { opcode, value });
        Ok(())
    }

    fn consume(&mut self, expected: u8) -> bool {
        self.whitespace();
        if self.peek() == Some(expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(format!(
                "expected `{}` at column {}",
                expected as char,
                self.cursor + 1
            ))
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.cursor += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.cursor).copied()
    }
}

fn push<T: Copy>(stack: &mut [T; MAX_STACK], depth: &mut usize, value: T) {
    stack[*depth] = value;
    *depth += 1;
}

fn unary<T: Copy>(stack: &mut [T; MAX_STACK], depth: usize, operation: impl FnOnce(T) -> T) {
    stack[depth - 1] = operation(stack[depth - 1]);
}

fn binary<T: Copy>(
    stack: &mut [T; MAX_STACK],
    depth: &mut usize,
    operation: impl FnOnce(T, T) -> T,
) {
    let right = stack[*depth - 1];
    *depth -= 1;
    stack[*depth - 1] = operation(stack[*depth - 1], right);
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn sanitize4(value: f32x4) -> f32x4 {
    f32x4::from(<[f32; 4]>::from(value).map(|value| finite_or_zero(value).clamp(-1.0, 1.0)))
}

fn sanitize8(value: f32x8) -> f32x8 {
    f32x8::from(<[f32; 8]>::from(value).map(|value| finite_or_zero(value).clamp(-1.0, 1.0)))
}
