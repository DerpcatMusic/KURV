//! Editor-thread user preset storage.
//!
//! This module performs filesystem I/O and must only be called from the UI
//! thread. Parameter values are keyed by stable ID; raw plugin state and
//! `Params` persistence remain separate opaque blobs so structural state can
//! evolve without changing host parameter identity.

mod context;
mod format;
mod storage;

use std::io;
use std::path::PathBuf;

use truce_core::editor::PluginContext;

use crate::KurvParams;

pub(crate) use format::sanitize_name;
#[cfg(test)]
pub(crate) use storage::create_atomic_temp;
pub(crate) use storage::{atomic_write, atomic_write_with, user_data_directory};

const EXTENSION: &str = "kurv";
const INIT_NAME: &str = "Init";
const DEFAULT_NAME: &str = "Default";

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
    init: format::Snapshot,
    entries: Vec<PresetEntry>,
    scanned: bool,
}

impl PresetStore {
    /// Creates a store with a canonical default-parameter Init snapshot.
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            directory: storage::preset_directory()?,
            init: context::init_snapshot()?,
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
        let entries = storage::scan(&self.directory)?;
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
        let snapshot = context::capture(context)?;
        let path = storage::write_preset(&self.directory, &name, &snapshot)?;
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
                storage::ensure_owned_path(&self.directory, path)?;
                let (_, snapshot) = storage::read_preset(path)?;
                snapshot
            }
        };
        context::apply(snapshot, context);
        Ok(())
    }
}

fn init_entry() -> PresetEntry {
    PresetEntry {
        name: String::from(INIT_NAME),
        source: PresetSource::Init,
    }
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
