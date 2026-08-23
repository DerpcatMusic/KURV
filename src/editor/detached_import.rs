use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, TryRecvError},
};

pub(crate) enum ImportSource {
    Bytes(Arc<[u8]>),
    Path(PathBuf),
}

impl ImportSource {
    pub(crate) fn read_bounded(
        self,
        name: &str,
        limit: usize,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        let path = match self {
            Self::Bytes(bytes) => {
                if bytes.len() > limit {
                    return Err(format!(
                        "{name}: file is too large ({} bytes; limit is {limit})",
                        bytes.len()
                    ));
                }
                if cancelled.load(Ordering::Acquire) {
                    return Err(format!("{name}: import cancelled"));
                }
                return Ok(bytes.to_vec());
            }
            Self::Path(path) => path,
        };
        let metadata = path
            .metadata()
            .map_err(|error| format!("{name}: cannot inspect file: {error}"))?;
        if !metadata.is_file() {
            return Err(format!("{name}: path is not a regular file"));
        }
        if metadata.len() > limit as u64 {
            return Err(format!(
                "{name}: file is too large ({} bytes; limit is {limit})",
                metadata.len()
            ));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let mut file = options
            .open(path)
            .map_err(|error| format!("{name}: cannot open file: {error}"))?;
        if !file
            .metadata()
            .map_err(|error| format!("{name}: cannot inspect opened file: {error}"))?
            .is_file()
        {
            return Err(format!("{name}: opened path is not a regular file"));
        }
        let capacity = usize::try_from(metadata.len())
            .map_err(|_| format!("{name}: file size cannot be represented on this platform"))?;
        let mut bytes = Vec::with_capacity(capacity.min(limit));
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(format!("{name}: import cancelled"));
            }
            let read_len = limit
                .saturating_add(1)
                .saturating_sub(bytes.len())
                .min(chunk.len());
            let count = std::io::Read::read(&mut file, &mut chunk[..read_len])
                .map_err(|error| format!("{name}: cannot read file: {error}"))?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
            if bytes.len() > limit {
                return Err(format!(
                    "{name}: file grew beyond the {limit}-byte limit while reading"
                ));
            }
        }
        Ok(bytes)
    }
}

pub(crate) struct DetachedJob<T> {
    pub(crate) result: Mutex<Receiver<Result<T, String>>>,
    pub(crate) cancelled: Arc<AtomicBool>,
}

impl<T> DetachedJob<T> {
    pub(crate) fn poll(&self) -> Option<Result<T, String>> {
        let receiver = self.result.try_lock().ok()?;
        match receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("detached import worker stopped".to_owned()))
            }
        }
    }
}

impl<T> Drop for DetachedJob<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(crate) fn spawn_detached_job<T: Send + 'static>(
    thread_name: &'static str,
    work: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static,
) -> Result<Arc<DetachedJob<T>>, String> {
    spawn_detached_job_after_pin(thread_name, detached_work_is_safe(), work)
}

pub(crate) fn detached_work_is_safe() -> bool {
    static IMAGE_PINNED: OnceLock<bool> = OnceLock::new();
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    return *IMAGE_PINNED.get_or_init(truce_egui::pin_current_image_for_detached_work);
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    false
}

pub(crate) fn spawn_detached_job_after_pin<T: Send + 'static>(
    thread_name: &'static str,
    safe_to_detach: bool,
    work: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static,
) -> Result<Arc<DetachedJob<T>>, String> {
    if !safe_to_detach {
        return Err("Cannot safely start a detached import worker".to_owned());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            let result = work(&worker_cancelled);
            if !worker_cancelled.load(Ordering::Acquire) {
                let _ = sender.send(result);
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(Arc::new(DetachedJob {
        result: Mutex::new(receiver),
        cancelled,
    }))
}
