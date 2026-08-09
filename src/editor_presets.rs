//! Editor-thread user preset storage.
//!
//! This module performs filesystem I/O and must only be called from the UI
//! thread. Parameter values are keyed by stable ID; raw plugin state and
//! `Params` persistence remain separate opaque blobs so structural state can
//! evolve without changing host parameter identity.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use truce::params::Params;
use truce_core::editor::PluginContext;

use crate::{KurvParams, P};

const MAGIC: [u8; 8] = *b"KURVPSET";
const VERSION: u16 = 2;
const V1_HEADER_LEN: usize = 20;
const HEADER_LEN: usize = 24;
const PARAM_LEN: usize = 12;
const MAX_NAME_BYTES: usize = 96;
const MAX_PARAMS: usize = 4_096;
const MAX_CUSTOM_STATE_BYTES: usize = 64 * 1024 * 1024;
const EXTENSION: &str = "kurv";
const INIT_NAME: &str = "Init";
const DEFAULT_NAME: &str = "Default";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(crate) struct PresetEntry {
    name: String,
    source: PresetSource,
}

#[derive(Clone, Debug)]
enum PresetSource {
    Init,
    File(PathBuf),
}

#[derive(Clone)]
struct Snapshot {
    params: Vec<(u32, f64)>,
    custom: Vec<u8>,
    persist: Vec<u8>,
}

impl PresetEntry {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) const fn is_init(&self) -> bool {
        matches!(self.source, PresetSource::Init)
    }
}

/// Cached editor-side view of KURV's per-user preset directory.
#[derive(Clone)]
pub(crate) struct PresetStore {
    directory: PathBuf,
    init: Snapshot,
    entries: Vec<PresetEntry>,
    scanned: bool,
}

impl PresetStore {
    /// Creates a store with a canonical default-parameter Init snapshot.
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            directory: preset_directory()?,
            init: snapshot_params(&KurvParams::default(), Vec::new())?,
            entries: vec![init_entry()],
            scanned: false,
        })
    }

    /// Returns the cached list, scanning once on first use.
    pub(crate) fn entries(&mut self) -> io::Result<&[PresetEntry]> {
        if !self.scanned {
            self.refresh()?;
        }
        Ok(&self.entries)
    }

    /// Refreshes the cache. Invalid, oversized, and partial files are ignored.
    pub(crate) fn refresh(&mut self) -> io::Result<()> {
        let mut entries = vec![init_entry()];
        let directory = match fs::read_dir(&self.directory) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.entries = entries;
                self.scanned = true;
                return Ok(());
            }
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
        self.entries = entries;
        self.scanned = true;
        Ok(())
    }

    /// Saves parameters plus opaque runtime and persisted structural state.
    pub(crate) fn save_as(
        &mut self,
        requested_name: &str,
        context: &PluginContext<KurvParams>,
    ) -> io::Result<PresetEntry> {
        let name = sanitize_name(requested_name)?;
        if name.eq_ignore_ascii_case(INIT_NAME) {
            return Err(invalid_input("Init is reserved"));
        }
        let snapshot = capture(context)?;
        fs::create_dir_all(&self.directory)?;
        let path = self.directory.join(format!("{name}.{EXTENSION}"));
        atomic_write(&path, &encode(&name, &snapshot)?)?;
        self.scanned = false;
        self.refresh()?;
        self.entries
            .iter()
            .find(|entry| matches!(&entry.source, PresetSource::File(saved) if saved == &path))
            .cloned()
            .ok_or_else(|| io::Error::other("saved preset was not found during refresh"))
    }

    pub(crate) fn save_default(
        &mut self,
        context: &PluginContext<KurvParams>,
    ) -> io::Result<PresetEntry> {
        self.save_as(DEFAULT_NAME, context)
    }

    /// Applies known parameter IDs and forwards both state channels unchanged.
    pub(crate) fn load(
        &self,
        entry: &PresetEntry,
        context: &PluginContext<KurvParams>,
    ) -> io::Result<()> {
        let snapshot = match &entry.source {
            PresetSource::Init => self.init.clone(),
            PresetSource::File(path) => {
                ensure_owned_path(&self.directory, path)?;
                let (_, snapshot) = read_preset(path)?;
                snapshot
            }
        };
        let Snapshot {
            params,
            custom,
            persist,
        } = snapshot;
        for (id, normalized) in params {
            if is_preset_param(id) && context.params().get_normalized(id).is_some() {
                context.set_param(id, normalized);
            }
        }
        if persist.is_empty() {
            context.params().generator_stack.reset_legacy();
        } else {
            context.params().load_persist(&persist);
        }
        context.set_state(custom);
        Ok(())
    }
}

fn init_entry() -> PresetEntry {
    PresetEntry {
        name: String::from(INIT_NAME),
        source: PresetSource::Init,
    }
}

fn preset_directory() -> io::Result<PathBuf> {
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

pub(crate) fn sanitize_name(requested: &str) -> io::Result<String> {
    let mut name = String::new();
    let mut last_separator = false;
    for character in requested.trim().chars() {
        let allowed = character.is_alphanumeric() || matches!(character, ' ' | '-' | '_');
        let character = if allowed { character } else { '-' };
        let separator = matches!(character, ' ' | '-' | '_');
        if separator && last_separator {
            continue;
        }
        if name.len() + character.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        name.push(character);
        last_separator = separator;
    }
    let trimmed = name.trim_matches(|character| matches!(character, ' ' | '-' | '_'));
    if trimmed.is_empty() {
        return Err(invalid_input("preset name is empty"));
    }
    name = String::from(trimmed);
    if is_windows_reserved(&name) {
        name.insert(0, '_');
        while name.len() > MAX_NAME_BYTES {
            name.pop();
        }
    }
    Ok(name)
}

fn is_windows_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && matches!(&upper[..3], "COM" | "LPT")
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

fn capture(context: &PluginContext<KurvParams>) -> io::Result<Snapshot> {
    let custom = context.get_state();
    let persist = context.params().serialize_persist();
    let mut params = Vec::new();
    for info in context.params().param_infos() {
        if !is_preset_param(info.id) {
            continue;
        }
        let normalized = context
            .params()
            .get_normalized(info.id)
            .ok_or_else(|| invalid_data("parameter metadata has no value"))?;
        if !normalized.is_finite() || !(0.0..=1.0).contains(&normalized) {
            return Err(invalid_data("plugin returned an invalid parameter value"));
        }
        params.push((info.id, normalized));
    }
    validate_snapshot(&params, custom.len(), persist.len())?;
    Ok(Snapshot {
        params,
        custom,
        persist,
    })
}

fn snapshot_params(params: &KurvParams, custom: Vec<u8>) -> io::Result<Snapshot> {
    let persist = params.serialize_persist();
    let mut values = Vec::new();
    for info in params.param_infos() {
        if !is_preset_param(info.id) {
            continue;
        }
        let normalized = params
            .get_normalized(info.id)
            .ok_or_else(|| invalid_data("parameter metadata has no value"))?;
        values.push((info.id, normalized));
    }
    validate_snapshot(&values, custom.len(), persist.len())?;
    Ok(Snapshot {
        params: values,
        custom,
        persist,
    })
}

fn is_preset_param(id: u32) -> bool {
    id != u32::from(P::PitchBend) && id != u32::from(P::SustainPedal)
}

fn encode(name: &str, snapshot: &Snapshot) -> io::Result<Vec<u8>> {
    let name_bytes = name.as_bytes();
    if name_bytes.is_empty() || name_bytes.len() > MAX_NAME_BYTES {
        return Err(invalid_input("preset name is too long"));
    }
    validate_snapshot(
        &snapshot.params,
        snapshot.custom.len(),
        snapshot.persist.len(),
    )?;
    let name_len = u16::try_from(name_bytes.len()).map_err(|_| invalid_input("name overflow"))?;
    let param_count =
        u32::try_from(snapshot.params.len()).map_err(|_| invalid_input("param count overflow"))?;
    let state_len =
        u32::try_from(snapshot.custom.len()).map_err(|_| invalid_input("custom state overflow"))?;
    let persist_len = u32::try_from(snapshot.persist.len())
        .map_err(|_| invalid_input("persist state overflow"))?;
    let param_bytes = snapshot.params.len() * PARAM_LEN;
    let mut encoded = Vec::with_capacity(
        HEADER_LEN
            + name_bytes.len()
            + param_bytes
            + snapshot.custom.len()
            + snapshot.persist.len(),
    );
    encoded.extend_from_slice(&MAGIC);
    encoded.extend_from_slice(&VERSION.to_le_bytes());
    encoded.extend_from_slice(&name_len.to_le_bytes());
    encoded.extend_from_slice(&param_count.to_le_bytes());
    encoded.extend_from_slice(&state_len.to_le_bytes());
    encoded.extend_from_slice(&persist_len.to_le_bytes());
    encoded.extend_from_slice(name_bytes);
    for (id, normalized) in &snapshot.params {
        encoded.extend_from_slice(&id.to_le_bytes());
        encoded.extend_from_slice(&normalized.to_bits().to_le_bytes());
    }
    encoded.extend_from_slice(&snapshot.custom);
    encoded.extend_from_slice(&snapshot.persist);
    Ok(encoded)
}

fn read_name(path: &Path) -> io::Result<String> {
    read_header(path)
}

#[derive(Clone, Copy)]
struct PresetHeader {
    encoded_len: usize,
    name_len: usize,
    param_count: usize,
    custom_len: usize,
    persist_len: usize,
}

fn read_header(path: &Path) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    let file_len =
        usize::try_from(metadata.len()).map_err(|_| invalid_data("file is too large"))?;
    if file_len < V1_HEADER_LEN
        || file_len > HEADER_LEN + MAX_NAME_BYTES + MAX_PARAMS * PARAM_LEN + MAX_CUSTOM_STATE_BYTES
    {
        return Err(invalid_data("invalid preset size"));
    }
    let mut file = File::open(path)?;
    let mut prefix = [0_u8; 10];
    file.read_exact(&mut prefix)?;
    let encoded_len = encoded_header_len(&prefix)?;
    let mut encoded = [0_u8; HEADER_LEN];
    encoded[..prefix.len()].copy_from_slice(&prefix);
    file.read_exact(&mut encoded[prefix.len()..encoded_len])?;
    let header = decode_header(&encoded[..encoded_len])?;
    let expected = preset_length(header)?;
    if expected != file_len {
        return Err(invalid_data("preset length mismatch"));
    }
    let mut name = vec![0_u8; header.name_len];
    file.read_exact(&mut name)?;
    let name = String::from_utf8(name).map_err(|_| invalid_data("preset name is not UTF-8"))?;
    if !sanitize_name(&name).is_ok_and(|sanitized| sanitized == name) {
        return Err(invalid_data("invalid preset name"));
    }
    Ok(name)
}

fn read_preset(path: &Path) -> io::Result<(String, Snapshot)> {
    let bytes = fs::read(path)?;
    if bytes.len() < V1_HEADER_LEN
        || bytes.len()
            > HEADER_LEN + MAX_NAME_BYTES + MAX_PARAMS * PARAM_LEN + MAX_CUSTOM_STATE_BYTES
    {
        return Err(invalid_data("invalid preset size"));
    }
    let encoded_len = encoded_header_len(&bytes)?;
    let encoded = bytes
        .get(..encoded_len)
        .ok_or_else(|| invalid_data("truncated KURV preset header"))?;
    let header = decode_header(encoded)?;
    let params_start = header
        .encoded_len
        .checked_add(header.name_len)
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    let state_start = params_start
        .checked_add(
            header
                .param_count
                .checked_mul(PARAM_LEN)
                .ok_or_else(|| invalid_data("preset length overflow"))?,
        )
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    let persist_start = state_start
        .checked_add(header.custom_len)
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    let expected = persist_start
        .checked_add(header.persist_len)
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    if expected != bytes.len() {
        return Err(invalid_data("preset length mismatch"));
    }
    let name = String::from_utf8(bytes[header.encoded_len..params_start].to_vec())
        .map_err(|_| invalid_data("preset name is not UTF-8"))?;
    if !sanitize_name(&name).is_ok_and(|sanitized| sanitized == name) {
        return Err(invalid_data("invalid preset name"));
    }
    let mut params = Vec::with_capacity(header.param_count);
    for record in bytes[params_start..state_start].chunks_exact(PARAM_LEN) {
        let id = u32::from_le_bytes(record[..4].try_into().map_err(|_| invalid_data("bad ID"))?);
        let normalized = f64::from_bits(u64::from_le_bytes(
            record[4..]
                .try_into()
                .map_err(|_| invalid_data("bad value"))?,
        ));
        params.push((id, normalized));
    }
    validate_snapshot(&params, header.custom_len, header.persist_len)?;
    Ok((
        name,
        Snapshot {
            params,
            custom: bytes[state_start..persist_start].to_vec(),
            persist: bytes[persist_start..].to_vec(),
        },
    ))
}

fn encoded_header_len(prefix: &[u8]) -> io::Result<usize> {
    if prefix.len() < 10 || prefix[..8] != MAGIC {
        return Err(invalid_data("not a KURV preset"));
    }
    match u16::from_le_bytes([prefix[8], prefix[9]]) {
        1 => Ok(V1_HEADER_LEN),
        VERSION => Ok(HEADER_LEN),
        _ => Err(invalid_data("unsupported KURV preset version")),
    }
}

fn decode_header(header: &[u8]) -> io::Result<PresetHeader> {
    let encoded_len = encoded_header_len(header)?;
    if header.len() != encoded_len {
        return Err(invalid_data("invalid KURV preset header"));
    }
    let name_len = usize::from(u16::from_le_bytes([header[10], header[11]]));
    let param_count = usize::try_from(u32::from_le_bytes([
        header[12], header[13], header[14], header[15],
    ]))
    .map_err(|_| invalid_data("parameter count overflow"))?;
    let custom_len = usize::try_from(u32::from_le_bytes([
        header[16], header[17], header[18], header[19],
    ]))
    .map_err(|_| invalid_data("state length overflow"))?;
    let persist_len = if encoded_len == HEADER_LEN {
        usize::try_from(u32::from_le_bytes([
            header[20], header[21], header[22], header[23],
        ]))
        .map_err(|_| invalid_data("persist length overflow"))?
    } else {
        0
    };
    let total_state = custom_len
        .checked_add(persist_len)
        .ok_or_else(|| invalid_data("state length overflow"))?;
    if name_len == 0
        || name_len > MAX_NAME_BYTES
        || param_count > MAX_PARAMS
        || total_state > MAX_CUSTOM_STATE_BYTES
    {
        return Err(invalid_data("preset field exceeds its bound"));
    }
    Ok(PresetHeader {
        encoded_len,
        name_len,
        param_count,
        custom_len,
        persist_len,
    })
}

fn preset_length(header: PresetHeader) -> io::Result<usize> {
    header
        .param_count
        .checked_mul(PARAM_LEN)
        .and_then(|params| {
            header
                .encoded_len
                .checked_add(header.name_len)?
                .checked_add(params)
        })
        .and_then(|length| length.checked_add(header.custom_len))
        .and_then(|length| length.checked_add(header.persist_len))
        .ok_or_else(|| invalid_data("preset length overflow"))
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

fn ensure_owned_path(directory: &Path, path: &Path) -> io::Result<()> {
    if path.parent() == Some(directory) {
        Ok(())
    } else {
        Err(invalid_input("preset is outside the KURV preset directory"))
    }
}

fn validate_snapshot(
    params: &[(u32, f64)],
    custom_len: usize,
    persist_len: usize,
) -> io::Result<()> {
    let state_len = custom_len
        .checked_add(persist_len)
        .ok_or_else(|| invalid_input("preset snapshot exceeds its bound"))?;
    if params.len() > MAX_PARAMS || state_len > MAX_CUSTOM_STATE_BYTES {
        return Err(invalid_input("preset snapshot exceeds its bound"));
    }
    for (index, (id, normalized)) in params.iter().enumerate() {
        if !normalized.is_finite() || !(0.0..=1.0).contains(normalized) {
            return Err(invalid_data("invalid normalized parameter value"));
        }
        if params[..index].iter().any(|(previous, _)| previous == id) {
            return Err(invalid_data("duplicate parameter ID"));
        }
    }
    Ok(())
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
