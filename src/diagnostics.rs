use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const RETAINED_JOURNALS: usize = 16;

static NEXT_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct DiagnosticSession {
    file: File,
    instance_id: u64,
    frame_clock: u64,
    max_buffer_frames: usize,
    last_channels: usize,
    process_seen: bool,
}

impl DiagnosticSession {
    pub(crate) fn begin() -> Option<Self> {
        let instance_id = NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed);
        let mut session = Self {
            file: open_log(instance_id)?,
            instance_id,
            frame_clock: 0,
            max_buffer_frames: 0,
            last_channels: 0,
            process_seen: false,
        };
        session.write_event("plugin_initialize", "started", &host_detail());
        Some(session)
    }

    pub(crate) fn initialized(&mut self, params: &crate::KurvParams) {
        let machine_id = params
            .activation
            .machine_id_hex()
            .unwrap_or_else(|| "unavailable".into());
        self.write_event(
            "plugin_initialize",
            "completed",
            &format!(
                "version={} dsp_state_bytes={} machine_id={machine_id}",
                env!("CARGO_PKG_VERSION"),
                std::mem::size_of::<crate::KurvDspState>(),
            ),
        );
    }

    #[inline]
    pub(crate) fn record_process(&mut self, frames: usize, channels: usize) {
        self.process_seen = true;
        self.max_buffer_frames = self.max_buffer_frames.max(frames);
        self.last_channels = channels;
        self.frame_clock = self
            .frame_clock
            .saturating_add(u64::try_from(frames).unwrap_or(u64::MAX));
    }

    fn write_event(&mut self, action: &str, phase: &str, detail: &str) {
        let _ = writeln!(
            self.file,
            "{}\t{}\t{}\t{}\t{}",
            now_nanos(),
            self.instance_id,
            action,
            phase,
            detail
        );
        let _ = self.file.flush();
    }
}

impl Drop for DiagnosticSession {
    fn drop(&mut self) {
        self.write_event(
            "plugin_destroy",
            "completed",
            &format!(
                "processed_frames={} max_buffer_frames={} last_channels={} process_seen={}",
                self.frame_clock, self.max_buffer_frames, self.last_channels, self.process_seen
            ),
        );
    }
}

fn open_log(instance_id: u64) -> Option<File> {
    let directory = diagnostics_dir();
    let _ = std::fs::create_dir_all(&directory);
    cleanup_old_journals(&directory);
    let filename = format!(
        "kurv-{}-{}-{instance_id}.log",
        std::process::id(),
        now_nanos()
    );
    OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(directory.join(&filename))
        .or_else(|_| {
            OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(std::env::temp_dir().join(filename))
        })
        .ok()
}

fn diagnostics_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Derpcat")
            .join("KURV")
            .join("Diagnostics");
    }
    if cfg!(target_os = "macos") {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Logs")
            .join("KURV");
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("kurv")
}

fn cleanup_old_journals(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut journals = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.extension().is_some_and(|extension| extension == "log") {
                return None;
            }
            Some((entry.metadata().ok()?.modified().ok()?, path))
        })
        .collect::<Vec<_>>();
    journals.sort_unstable_by_key(|(modified, _)| *modified);
    let remove = journals.len().saturating_sub(RETAINED_JOURNALS - 1);
    for (_, path) in journals.into_iter().take(remove) {
        let _ = std::fs::remove_file(path);
    }
}

fn host_detail() -> String {
    let host = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    format!(
        "version={} host={} pid={} os={} arch={}",
        env!("CARGO_PKG_VERSION"),
        host,
        std::process::id(),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
