use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::format::{self, Snapshot};
use super::{
    EXTENSION, INIT_NAME, PresetEntry, PresetSource, init_entry, invalid_data, invalid_input,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn scan(directory: &Path) -> io::Result<Vec<PresetEntry>> {
    let mut entries = vec![init_entry()];
    let directory = match fs::read_dir(directory) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(entries),
        Err(error) => return Err(error),
    };

    for item in directory {
        let Ok(item) = item else { continue };
        let path = item.path();
        let is_preset = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION));
        if !is_preset || !item.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        if let Ok(name) = read_name(&path)
            && !name.eq_ignore_ascii_case(INIT_NAME)
        {
            entries.push(PresetEntry {
                name,
                source: PresetSource::File(path),
            });
        }
    }
    entries[1..].sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

pub(super) fn preset_directory() -> io::Result<PathBuf> {
    Ok(user_data_directory()?.join("Presets"))
}

pub(crate) fn user_data_directory() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from);

    #[cfg(target_os = "macos")]
    let root = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local").join("share"))
    });

    root.map(|root| root.join("KURV"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no per-user data directory"))
}

pub(super) fn write_preset(
    directory: &Path,
    name: &str,
    snapshot: &Snapshot,
) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("{name}.{EXTENSION}"));
    atomic_write(&path, &format::encode(name, snapshot)?)?;
    Ok(path)
}

fn read_name(path: &Path) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    let file_len =
        usize::try_from(metadata.len()).map_err(|_| invalid_data("file is too large"))?;
    format::validate_file_length(file_len)?;
    let mut file = File::open(path)?;
    format::decode_name(&mut file, file_len)
}

pub(super) fn read_preset(path: &Path) -> io::Result<(String, Snapshot)> {
    format::decode(fs::read(path)?)
}

pub(crate) fn atomic_write(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| invalid_input("preset path has no parent"))?;
    let (temporary, mut file) = create_temporary(parent)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary(directory: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".kurv-{}-{sequence}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary preset file",
    ))
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(temporary, destination) {
        Ok(()) => Ok(()),
        Err(_error) if destination.exists() => {
            let backup = destination.with_extension("kurv.previous");
            let _ = fs::remove_file(&backup);
            fs::rename(destination, &backup)?;
            match fs::rename(temporary, destination) {
                Ok(()) => {
                    let _ = fs::remove_file(backup);
                    Ok(())
                }
                Err(replace_error) => {
                    let _ = fs::rename(backup, destination);
                    Err(replace_error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

pub(super) fn ensure_owned_path(directory: &Path, path: &Path) -> io::Result<()> {
    if path.parent() == Some(directory) {
        Ok(())
    } else {
        Err(invalid_input("preset is outside the KURV preset directory"))
    }
}
