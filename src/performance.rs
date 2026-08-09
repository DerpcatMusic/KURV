//! Machine-local SIMD selection and off-thread calibration.

use std::fs;
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::Once;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SplineBackend {
    Baseline = 0,
    Avx2Fma = 1,
}

const PROFILE_SCHEMA: u64 = 1;
const STATUS_IDLE: u8 = 0;
const STATUS_RUNNING: u8 = 1;
const STATUS_READY: u8 = 2;
const STATUS_FAILED: u8 = 3;

#[cfg(not(test))]
static INITIALIZE: Once = Once::new();
static BACKEND: AtomicU8 = AtomicU8::new(SplineBackend::Baseline as u8);
static STATUS: AtomicU8 = AtomicU8::new(STATUS_IDLE);
static BASELINE_NS: AtomicU64 = AtomicU64::new(0);
static AVX2_NS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn initialize() {
    #[cfg(test)]
    BACKEND.store(SplineBackend::Baseline as u8, Ordering::Release);

    #[cfg(not(test))]
    INITIALIZE.call_once(|| {
        let detected = detected_backend();
        let requested = match std::env::var("KURV_SIMD").as_deref() {
            Ok("baseline") => Some(SplineBackend::Baseline),
            Ok("avx2" | "avx2-fma") => Some(SplineBackend::Avx2Fma),
            _ => None,
        };
        let selected = requested
            .or_else(load_profile)
            .filter(|backend| backend_supported(*backend));
        BACKEND.store(selected.unwrap_or(detected) as u8, Ordering::Release);
    });
}

#[inline]
pub(crate) fn spline_backend() -> SplineBackend {
    match BACKEND.load(Ordering::Relaxed) {
        1 => SplineBackend::Avx2Fma,
        _ => SplineBackend::Baseline,
    }
}

pub(crate) fn backend_supported(backend: SplineBackend) -> bool {
    match backend {
        SplineBackend::Baseline => true,
        SplineBackend::Avx2Fma => avx2_fma_available(),
    }
}

pub(crate) fn start_calibration() -> bool {
    initialize();
    loop {
        let status = STATUS.load(Ordering::Acquire);
        if status == STATUS_RUNNING {
            return false;
        }
        if STATUS
            .compare_exchange(status, STATUS_RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            break;
        }
    }

    let spawn = std::thread::Builder::new()
        .name("kurv-cpu-calibration".to_owned())
        .spawn(|| match crate::oscillators::calibrate_spline_backends() {
            Ok((baseline_ns, avx2_ns, selected)) => {
                BASELINE_NS.store(baseline_ns, Ordering::Release);
                AVX2_NS.store(avx2_ns.unwrap_or(0), Ordering::Release);
                BACKEND.store(selected as u8, Ordering::Release);
                if write_profile(selected).is_ok() {
                    STATUS.store(STATUS_READY, Ordering::Release);
                } else {
                    STATUS.store(STATUS_FAILED, Ordering::Release);
                }
            }
            Err(()) => STATUS.store(STATUS_FAILED, Ordering::Release),
        });
    if spawn.is_err() {
        STATUS.store(STATUS_FAILED, Ordering::Release);
        return false;
    }
    true
}

pub(crate) fn reset_auto() {
    initialize();
    BACKEND.store(detected_backend() as u8, Ordering::Release);
    BASELINE_NS.store(0, Ordering::Release);
    AVX2_NS.store(0, Ordering::Release);
    STATUS.store(STATUS_IDLE, Ordering::Release);
    if let Ok(path) = profile_path() {
        let _ = fs::remove_file(path);
    }
}

pub(crate) fn status_text() -> String {
    initialize();
    let backend = match spline_backend() {
        SplineBackend::Baseline => "BASELINE",
        SplineBackend::Avx2Fma => "AVX2 + FMA",
    };
    match STATUS.load(Ordering::Acquire) {
        STATUS_RUNNING => format!("{backend} · CALIBRATING…"),
        STATUS_READY => {
            let baseline = BASELINE_NS.load(Ordering::Acquire);
            let avx2 = AVX2_NS.load(Ordering::Acquire);
            if avx2 == 0 {
                format!("{backend} · CALIBRATED")
            } else {
                format!("{backend} · {baseline} / {avx2} ns")
            }
        }
        STATUS_FAILED => format!("{backend} · CALIBRATION FAILED"),
        _ => format!("{backend} · AUTO"),
    }
}

pub(crate) fn calibration_running() -> bool {
    STATUS.load(Ordering::Acquire) == STATUS_RUNNING
}

fn detected_backend() -> SplineBackend {
    if avx2_fma_available() {
        SplineBackend::Avx2Fma
    } else {
        SplineBackend::Baseline
    }
}

#[cfg(target_arch = "x86_64")]
fn avx2_fma_available() -> bool {
    std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")
}

#[cfg(not(target_arch = "x86_64"))]
const fn avx2_fma_available() -> bool {
    false
}

fn load_profile() -> Option<SplineBackend> {
    let root: serde_json::Value =
        serde_json::from_slice(&fs::read(profile_path().ok()?).ok()?).ok()?;
    if root.get("schema")?.as_u64()? != PROFILE_SCHEMA
        || root.get("fingerprint")?.as_str()? != runtime_fingerprint()
    {
        return None;
    }
    match root.get("backend")?.as_str()? {
        "avx2-fma" => Some(SplineBackend::Avx2Fma),
        "baseline" => Some(SplineBackend::Baseline),
        _ => None,
    }
}

fn write_profile(backend: SplineBackend) -> std::io::Result<()> {
    let path = profile_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let backend = match backend {
        SplineBackend::Baseline => "baseline",
        SplineBackend::Avx2Fma => "avx2-fma",
    };
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": PROFILE_SCHEMA,
        "fingerprint": runtime_fingerprint(),
        "backend": backend,
    }))?;
    fs::write(path, bytes)
}

fn runtime_fingerprint() -> String {
    format!(
        "{}:{}:{}:avx2={}:fma={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION"),
        avx2_fma_available(),
        cfg!(target_feature = "fma")
    )
}

fn profile_path() -> std::io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library").join("Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".config"))
        });
    root.map(|path| path.join("KURV").join("performance.json"))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "user data directory unavailable",
            )
        })
}
