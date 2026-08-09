//! Persisted editor/state-thread storage for generator stack patches.
//!
//! This module locks and allocates; none of its API belongs on the audio thread.

use std::sync::{
    RwLock,
    atomic::{AtomicBool, Ordering},
};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use super::{ModuleKind, OscillatorSlot, Patch};

const STATE_VERSION: u32 = 1;
const OSCILLATOR_KIND: u8 = 0;
const FILTER_KIND: u8 = 1;

#[derive(State)]
struct StackDocument {
    version: u32,
    next_group_id: u64,
    next_module_id: u64,
    materialized: bool,
    groups: Vec<GroupDocument>,
}

impl Default for StackDocument {
    fn default() -> Self {
        Self {
            version: 0,
            next_group_id: 0,
            next_module_id: 0,
            materialized: false,
            groups: vec![GroupDocument::default()],
        }
    }
}

#[derive(State)]
struct GroupDocument {
    id: u64,
    modules: Vec<ModuleDocument>,
}

impl Default for GroupDocument {
    fn default() -> Self {
        Self {
            id: 0,
            modules: vec![ModuleDocument::default()],
        }
    }
}

#[derive(State)]
struct ModuleDocument {
    id: u64,
    kind: u8,
    oscillator_slot: u8,
}

impl Default for ModuleDocument {
    fn default() -> Self {
        Self {
            id: 0,
            kind: u8::MAX,
            oscillator_slot: u8::MAX,
        }
    }
}

impl StackDocument {
    fn from_patch(patch: &Patch, materialized: bool) -> Self {
        Self {
            version: STATE_VERSION,
            next_group_id: patch.next_group_id(),
            next_module_id: patch.next_module_id(),
            materialized,
            groups: patch
                .groups()
                .iter()
                .map(|group| GroupDocument {
                    id: group.id().get(),
                    modules: group
                        .modules()
                        .iter()
                        .map(|module| ModuleDocument {
                            id: module.id().get(),
                            kind: match module.kind() {
                                ModuleKind::Oscillator(_) => OSCILLATOR_KIND,
                                ModuleKind::Filter => FILTER_KIND,
                            },
                            oscillator_slot: module
                                .oscillator_slot()
                                .map_or(0, OscillatorSlot::encoded),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn into_patch(self) -> Option<(Patch, bool)> {
        if self.version != STATE_VERSION || self.next_group_id == 0 || self.next_module_id == 0 {
            return None;
        }

        let mut groups = Vec::with_capacity(self.groups.len());
        for group in self.groups {
            let mut modules = Vec::with_capacity(group.modules.len());
            for module in group.modules {
                let kind = match module.kind {
                    OSCILLATOR_KIND => ModuleKind::Oscillator(OscillatorSlot::from_index(
                        usize::from(module.oscillator_slot),
                    )?),
                    FILTER_KIND => ModuleKind::Filter,
                    _ => return None,
                };
                modules.push((module.id, kind));
            }
            groups.push((group.id, modules));
        }

        Patch::restore(groups, self.next_group_id, self.next_module_id)
            .ok()
            .map(|patch| (patch, self.materialized))
    }
}

/// Editable generator patch storage for UI/state-thread use only.
pub struct GeneratorStackState {
    patch: RwLock<Patch>,
    materialized: AtomicBool,
}

impl GeneratorStackState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            patch: RwLock::new(Patch::new()),
            materialized: AtomicBool::new(false),
        }
    }

    /// Whether the document has been reconciled with the legacy oscillator
    /// parameters. Old sessions can remain parameter-driven until this flips.
    #[must_use]
    pub fn is_materialized(&self) -> bool {
        self.materialized.load(Ordering::Acquire)
    }

    /// Clones the current editor-side patch.
    #[must_use]
    pub fn snapshot(&self) -> Patch {
        self.patch
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Edits the patch under its UI/state-thread write lock.
    pub fn edit<R>(&self, edit: impl FnOnce(&mut Patch) -> R) -> R {
        let result = edit(
            &mut self
                .patch
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        self.materialized.store(true, Ordering::Release);
        result
    }

    pub(crate) fn reset_legacy(&self) {
        *self
            .patch
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Patch::default();
        self.materialized.store(false, Ordering::Release);
    }
}

impl Default for GeneratorStackState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for GeneratorStackState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        StackDocument::from_patch(&self.snapshot(), self.is_materialized()).write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some((patch, materialized)) =
            StackDocument::read_field(cursor).and_then(StackDocument::into_patch)
        else {
            return;
        };
        *self
            .patch
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = patch;
        self.materialized.store(materialized, Ordering::Release);
    }
}
