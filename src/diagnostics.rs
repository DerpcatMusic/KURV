//! Tiny crash-safe markers for host-boundary diagnosis.

use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "windows")]
use std::fs::{self, File, OpenOptions};
#[cfg(target_os = "windows")]
use std::io::Write as IoWrite;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::sync::{Mutex, OnceLock};

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

#[cfg(target_os = "windows")]
static LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

#[cfg(target_os = "windows")]
fn log_file() -> Option<&'static Mutex<File>> {
    LOG_FILE
        .get_or_init(|| {
            let mut directory = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            directory.push("KURV");
            directory.push("Logs");
            let _ = fs::create_dir_all(&directory);
            directory.push(format!("KURV-{}.log", std::process::id()));
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory)
                .ok()
                .map(Mutex::new)
        })
        .as_ref()
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetModuleFileNameW(module: *mut std::ffi::c_void, filename: *mut u16, size: u32) -> u32;
    fn GetModuleHandleExW(
        flags: u32,
        module_name: *const u16,
        module: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn OutputDebugStringA(message: *const u8);
}

#[cfg(target_os = "windows")]
fn append_windows_context(line: &mut TraceLine) {
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x0000_0004;
    let mut module = std::ptr::null_mut();
    let address = emit as *const () as *const u16;
    let module_path = if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            address,
            &raw mut module,
        )
    } != 0
    {
        let mut buffer = [0_u16; 512];
        let length =
            unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };
        String::from_utf16_lossy(&buffer[..(length as usize).min(buffer.len())])
    } else {
        String::from("<module-unknown>")
    };
    let _ = write!(
        line,
        " pid={} tid={} module={module_path}",
        std::process::id(),
        unsafe { GetCurrentThreadId() },
    );
}

#[cfg(target_os = "windows")]
fn emit(line: &[u8]) {
    if let Some(file) = log_file()
        && let Ok(mut file) = file.lock()
    {
        let _ = file.write_all(line);
        let _ = file.flush();
        let _ = file.sync_data();
    }

    let mut debug_line = [0_u8; 257];
    let length = line.len().min(debug_line.len() - 1);
    debug_line[..length].copy_from_slice(&line[..length]);
    unsafe { OutputDebugStringA(debug_line.as_ptr()) };
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn append_cpu_context(line: &mut TraceLine) {
    let _ = write!(
        line,
        " arch={} compiled_avx2={} compiled_fma={} runtime_avx2={} runtime_fma={} runtime_sse42={}",
        std::env::consts::ARCH,
        cfg!(target_feature = "avx2"),
        cfg!(target_feature = "fma"),
        std::is_x86_feature_detected!("avx2"),
        std::is_x86_feature_detected!("fma"),
        std::is_x86_feature_detected!("sse4.2"),
    );
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn append_cpu_context(_line: &mut TraceLine) {}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn emit(_line: &[u8]) {}

pub(crate) fn startup() {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut line = TraceLine::new();
    let _ = write!(
        line,
        "KURV_DIAG seq={sequence} control=plugin stage=startup package={}",
        env!("CARGO_PKG_VERSION")
    );
    #[cfg(target_os = "windows")]
    append_windows_context(&mut line);
    append_cpu_context(&mut line);
    let _ = writeln!(line);
    emit(&line.bytes[..line.len]);
}

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
