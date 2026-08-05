//! Tiny crash-safe markers for host-boundary diagnosis.

use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TraceLine {
    bytes: [u8; 256],
    len: usize,
}

impl TraceLine {
    const fn new() -> Self {
        Self {
            bytes: [0; 256],
            len: 0,
        }
    }
}

impl fmt::Write for TraceLine {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let remaining = self.bytes.len().saturating_sub(self.len);
        let count = remaining.min(value.len());
        self.bytes[self.len..self.len + count].copy_from_slice(&value.as_bytes()[..count]);
        self.len += count;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn emit(line: &[u8]) {
    // SAFETY: `line` is a valid immutable byte slice for the duration of this
    // single diagnostic write. No Rust lock or allocation is involved.
    unsafe {
        let _ = libc::write(libc::STDERR_FILENO, line.as_ptr().cast(), line.len());
    }
}

#[cfg(not(target_os = "linux"))]
fn emit(_line: &[u8]) {}

pub(crate) fn trace(control: &str, stage: &str, x: f32, y: f32) {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut line = TraceLine::new();
    let _ = writeln!(
        line,
        "KURV_DIAG seq={sequence} control={control} stage={stage} x={x:.6} y={y:.6}"
    );
    emit(&line.bytes[..line.len]);
}

pub(crate) fn lifecycle(stage: &str) {
    trace("lifecycle", stage, 0.0, 0.0);
}
