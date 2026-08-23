use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use super::*;

const INITIAL_STATE_VERSION: u32 = 1;
const SECOND_STATE_VERSION: u32 = 2;
const PREVIOUS_STATE_VERSION: u32 = 3;
const PAN_SHAPE_STATE_VERSION: u32 = 4;
const MATERIALIZED_DEFAULT_STATE_VERSION: u32 = 5;
const GROUP_ENVELOPE_STATE_VERSION: u32 = 6;
const GROUP_ENVELOPE_CURVE_STATE_VERSION: u32 = 7;
const MIDI_ROUTING_STATE_VERSION: u32 = 8;
const FILTER_STATE_VERSION: u32 = 9;
const PARALLEL_ROUTING_STATE_VERSION: u32 = 10;
const LEGACY_MATERIALIZATION_STATE_VERSION: u32 = 11;
const LEGACY_AUTOMATION_STATE_VERSION: u32 = 12;
const PRE_RESYNTH_STATE_VERSION: u32 = 13;
const STATE_VERSION: u32 = 14;
const OSCILLATOR_KIND: u8 = 0;
const FILTER_KIND: u8 = 1;

#[derive(State)]
struct StackDocument {
    version: u32,
    next_group_id: u64,
    next_module_id: u64,
    materialized: bool,
    groups: Vec<GroupDocument>,
    oscillators: Vec<OscillatorDocument>,
    va_tables: Vec<VaTableData>,
    pan_shape_curves: Vec<PanShapeCurveData>,
    // Keep new fields at the tail: Truce's legacy State blobs are positional.
    filters: Vec<FilterDocument>,
    // Current documents remember whether historical oscillator parameter IDs
    // should be bridged onto the materialized structural stack.
    legacy_host_automation_bridge: bool,
    legacy_automation_oscillator_masks: Vec<u32>,
    legacy_automation_group_mask: u32,
    legacy_automation_oscillator_released: Vec<u32>,
    legacy_automation_group_released: u32,
    legacy_pan_automation_masks: Vec<u32>,
    legacy_pan_automation_released: Vec<u32>,
}

impl Default for StackDocument {
    fn default() -> Self {
        Self {
            version: 0,
            next_group_id: 0,
            next_module_id: 0,
            materialized: false,
            groups: vec![GroupDocument::default()],
            oscillators: Vec::new(),
            va_tables: Vec::new(),
            pan_shape_curves: Vec::new(),
            filters: Vec::new(),
            legacy_host_automation_bridge: false,
            legacy_automation_oscillator_masks: Vec::new(),
            legacy_automation_group_mask: 0,
            legacy_automation_oscillator_released: Vec::new(),
            legacy_automation_group_released: 0,
            legacy_pan_automation_masks: Vec::new(),
            legacy_pan_automation_released: Vec::new(),
        }
    }
}

#[derive(State)]
struct GroupDocument {
    id: u64,
    modules: Vec<ModuleDocument>,
    output_pair: u8,
    output_gain: f32,
    output_pan: f32,
    // Appended for compatibility with Truce's legacy positional State blobs.
    output_attack: f32,
    output_decay: f32,
    output_sustain: f32,
    output_release: f32,
    // Keep new fields at the tail: Truce's legacy State blobs are positional.
    output_attack_curve: f32,
    output_decay_curve: f32,
    output_release_curve: f32,
    output_receive_midi_channel: u8,
    // Appended parallel routing fields; zero send keeps older documents silent.
    output_dry: f32,
    output_send: f32,
    output_sidechain: f32,
    output_send_pair: u8,
    // Appended after v10 so migrated one-group documents retain their legacy
    // global-envelope topology across the next save.
    output_envelope_enabled: bool,
    // Appended in v12 so pre-modular ADSR bend handles survive materialization.
    output_attack_curve_time: f32,
    output_decay_curve_time: f32,
    output_release_curve_time: f32,
}

impl Default for GroupDocument {
    fn default() -> Self {
        Self {
            id: 0,
            modules: vec![ModuleDocument::default()],
            output_pair: 0,
            output_gain: 1.0,
            output_pan: 0.0,
            output_attack: 0.0,
            output_decay: 0.1,
            output_sustain: 1.0,
            output_release: 0.0,
            output_attack_curve: 0.0,
            output_decay_curve: 0.0,
            output_release_curve: 0.0,
            output_receive_midi_channel: 0,
            output_dry: 1.0,
            output_send: 0.0,
            output_sidechain: 0.0,
            output_send_pair: 0,
            output_envelope_enabled: true,
            output_attack_curve_time: 0.0,
            output_decay_curve_time: 0.0,
            output_release_curve_time: 0.0,
        }
    }
}

#[derive(State)]
struct ModuleDocument {
    id: u64,
    kind: u8,
    oscillator_slot: u8,
    // Keep new fields at the tail: Truce's legacy State blobs are positional.
    filter_slot: u8,
}

impl Default for ModuleDocument {
    fn default() -> Self {
        Self {
            id: 0,
            kind: u8::MAX,
            oscillator_slot: u8::MAX,
            filter_slot: u8::MAX,
        }
    }
}

#[derive(State)]
struct FilterDocument {
    mode: u8,
    cutoff_hz: f32,
    q: f32,
    slope_db_oct: f32,
    morph: f32,
}

impl Default for FilterDocument {
    fn default() -> Self {
        Self::from_config(FilterConfig::default())
    }
}

impl FilterDocument {
    fn from_config(config: FilterConfig) -> Self {
        let config = sanitize_filter_config(config);
        Self {
            mode: filter_mode_encoded(config.mode),
            cutoff_hz: config.cutoff_hz,
            q: config.q,
            slope_db_oct: config.slope_db_oct,
            morph: config.morph,
        }
    }

    fn into_config(self) -> FilterConfig {
        let legacy_morph = match self.mode {
            1 => 0.5,
            2 => 1.0,
            _ => 0.0,
        };
        sanitize_filter_config(FilterConfig {
            mode: filter_mode_from_encoded(self.mode),
            cutoff_hz: self.cutoff_hz,
            q: self.q,
            slope_db_oct: if self.slope_db_oct == 0.0 {
                12.0
            } else {
                self.slope_db_oct
            },
            morph: if self.mode < 8 {
                legacy_morph
            } else {
                self.morph
            },
        })
    }
}

#[derive(State)]
struct OscillatorDocument {
    enabled: bool,
    shape: f32,
    pulse_width: f32,
    transpose: f32,
    cents: f32,
    level: f32,
    pan: f32,
    unison_voices: u8,
    unison_range: f32,
    unison_amount: f32,
    unison_curve: f32,
    unison_jitter: f32,
    unison_rate: f32,
    unison_width: f32,
    // Appended for compatibility with Truce's legacy positional State blobs.
    custom_shape: f32,
    phase_position: f32,
    phase_random: f32,
    unison_alignment: f32,
    unison_alignment_mode: u8,
    unison_pan_curve: f32,
    unison_pan_center_x: f32,
    unison_stereo_x: f32,
    unison_stereo_alternate: f32,
    unison_jitter_mode: u8,
    unison_weight: f32,
    phase_warp_mode: u8,
    phase_warp_amount: f32,
    // v14: renderer discriminator; absent v1-v13 documents default to VA.
    engine: u8,
}

impl Default for OscillatorDocument {
    fn default() -> Self {
        Self::from_config(OscillatorConfig::default())
    }
}

impl OscillatorDocument {
    fn from_config(config: OscillatorConfig) -> Self {
        Self {
            enabled: config.enabled,
            shape: config.shape,
            custom_shape: config.custom_shape,
            pulse_width: config.pulse_width,
            transpose: config.transpose,
            cents: config.cents,
            level: config.level,
            pan: config.pan,
            unison_voices: config.unison_voices,
            unison_range: config.unison_range,
            unison_amount: config.unison_amount,
            unison_curve: config.unison_curve,
            unison_jitter: config.unison_jitter,
            unison_jitter_mode: config.unison_jitter_mode,
            unison_rate: config.unison_rate,
            unison_width: config.unison_width,
            unison_weight: config.unison_weight,
            phase_position: config.phase_position,
            phase_random: config.phase_random,
            phase_warp_mode: config.phase_warp_mode,
            phase_warp_amount: config.phase_warp_amount,
            engine: config.engine as u8,
            unison_alignment: config.unison_alignment,
            unison_alignment_mode: config.unison_alignment_mode,
            unison_pan_curve: config.unison_pan_curve,
            unison_pan_center_x: config.unison_pan_center_x,
            unison_stereo_x: config.unison_stereo_x,
            unison_stereo_alternate: config.unison_stereo_alternate,
        }
    }

    fn into_config(self) -> OscillatorConfig {
        OscillatorConfig {
            enabled: self.enabled,
            engine: OscillatorEngineKind::from_u8(self.engine),
            shape: self.shape,
            custom_shape: self.custom_shape,
            pulse_width: self.pulse_width,
            transpose: self.transpose,
            cents: self.cents,
            level: self.level,
            pan: self.pan,
            unison_voices: self.unison_voices,
            unison_range: self.unison_range,
            unison_amount: self.unison_amount,
            unison_curve: self.unison_curve,
            unison_jitter: self.unison_jitter,
            unison_jitter_mode: self.unison_jitter_mode,
            unison_rate: self.unison_rate,
            unison_width: self.unison_width,
            unison_weight: self.unison_weight,
            phase_position: self.phase_position,
            phase_random: self.phase_random,
            phase_warp_mode: self.phase_warp_mode,
            phase_warp_amount: self.phase_warp_amount,
            unison_alignment: self.unison_alignment,
            unison_alignment_mode: self.unison_alignment_mode,
            unison_pan_curve: self.unison_pan_curve,
            unison_pan_center_x: self.unison_pan_center_x,
            unison_stereo_x: self.unison_stereo_x,
            unison_stereo_alternate: self.unison_stereo_alternate,
        }
        .sanitized()
    }
}

impl StackDocument {
    fn from_document(
        document: &GeneratorDocument,
        va_tables: &[VaTableState; MAX_OSCILLATORS],
        pan_shape_curves: &[PanShapeCurveState; MAX_OSCILLATORS],
        materialized: bool,
        legacy_host_automation_bridge: bool,
        legacy_automation_masks: ([u32; 3], u16, [u32; 3], u16, [u16; 3], [u16; 3]),
    ) -> Self {
        let patch = &document.patch;
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
                                ModuleKind::Filter(_) => FILTER_KIND,
                            },
                            oscillator_slot: module
                                .oscillator_slot()
                                .map_or(0, OscillatorSlot::encoded),
                            filter_slot: module.filter_slot().map_or(0, FilterSlot::encoded),
                        })
                        .collect(),
                    output_pair: group.output().pair,
                    output_receive_midi_channel: group.output().receive_midi_channel,
                    output_gain: group.output().gain,
                    output_pan: group.output().pan,
                    output_dry: group.output().dry,
                    output_send: group.output().send,
                    output_sidechain: group.output().sidechain,
                    output_send_pair: group.output().send_pair,
                    output_envelope_enabled: group.output().envelope_enabled,
                    output_attack: group.output().attack,
                    output_attack_curve: group.output().attack_curve,
                    output_decay: group.output().decay,
                    output_decay_curve: group.output().decay_curve,
                    output_sustain: group.output().sustain,
                    output_release: group.output().release,
                    output_release_curve: group.output().release_curve,
                    output_attack_curve_time: group.output().attack_curve_time,
                    output_decay_curve_time: group.output().decay_curve_time,
                    output_release_curve_time: group.output().release_curve_time,
                })
                .collect(),
            oscillators: document
                .oscillators
                .iter()
                .copied()
                .map(OscillatorDocument::from_config)
                .collect(),
            va_tables: va_tables.iter().map(VaTableState::snapshot).collect(),
            pan_shape_curves: pan_shape_curves
                .iter()
                .map(PanShapeCurveState::snapshot)
                .collect(),
            filters: document
                .filters
                .iter()
                .copied()
                .map(FilterDocument::from_config)
                .collect(),
            legacy_host_automation_bridge,
            legacy_automation_oscillator_masks: legacy_automation_masks.0.to_vec(),
            legacy_automation_group_mask: u32::from(legacy_automation_masks.1),
            legacy_automation_oscillator_released: legacy_automation_masks.2.to_vec(),
            legacy_automation_group_released: u32::from(legacy_automation_masks.3),
            legacy_pan_automation_masks: legacy_automation_masks.4.map(u32::from).to_vec(),
            legacy_pan_automation_released: legacy_automation_masks.5.map(u32::from).to_vec(),
        }
    }

    fn into_document(
        self,
    ) -> Option<(
        GeneratorDocument,
        [VaTableData; MAX_OSCILLATORS],
        [PanShapeCurveData; MAX_OSCILLATORS],
        bool,
        bool,
        [u32; 3],
        u16,
        [u32; 3],
        u16,
        [u16; 3],
        [u16; 3],
    )> {
        let version = self.version;
        if !matches!(
            version,
            INITIAL_STATE_VERSION
                | SECOND_STATE_VERSION
                | PREVIOUS_STATE_VERSION
                | PAN_SHAPE_STATE_VERSION
                | MATERIALIZED_DEFAULT_STATE_VERSION
                | GROUP_ENVELOPE_STATE_VERSION
                | GROUP_ENVELOPE_CURVE_STATE_VERSION
                | MIDI_ROUTING_STATE_VERSION
                | FILTER_STATE_VERSION
                | PARALLEL_ROUTING_STATE_VERSION
                | LEGACY_MATERIALIZATION_STATE_VERSION
                | LEGACY_AUTOMATION_STATE_VERSION
                | PRE_RESYNTH_STATE_VERSION
                | STATE_VERSION
        ) || self.next_group_id == 0
            || self.next_module_id == 0
        {
            return None;
        }

        let legacy_automation_oscillator_masks = if version >= LEGACY_AUTOMATION_STATE_VERSION {
            self.legacy_automation_oscillator_masks.try_into().ok()?
        } else {
            [0; 3]
        };
        let legacy_automation_group_mask = if version >= LEGACY_AUTOMATION_STATE_VERSION {
            u16::try_from(self.legacy_automation_group_mask).ok()?
        } else {
            0
        };
        let legacy_automation_oscillator_released = if version >= LEGACY_AUTOMATION_STATE_VERSION {
            self.legacy_automation_oscillator_released.try_into().ok()?
        } else {
            [0; 3]
        };
        let legacy_automation_group_released = if version >= LEGACY_AUTOMATION_STATE_VERSION {
            u16::try_from(self.legacy_automation_group_released).ok()?
        } else {
            0
        };
        let legacy_pan_automation_masks = if version >= LEGACY_AUTOMATION_STATE_VERSION
            && !self.legacy_pan_automation_masks.is_empty()
        {
            self.legacy_pan_automation_masks
                .into_iter()
                .map(u16::try_from)
                .collect::<Result<Vec<_>, _>>()
                .ok()?
                .try_into()
                .ok()?
        } else {
            [0; 3]
        };
        let legacy_pan_automation_released = if version >= LEGACY_AUTOMATION_STATE_VERSION
            && !self.legacy_pan_automation_released.is_empty()
        {
            self.legacy_pan_automation_released
                .into_iter()
                .map(u16::try_from)
                .collect::<Result<Vec<_>, _>>()
                .ok()?
                .try_into()
                .ok()?
        } else {
            [0; 3]
        };

        let mut groups = Vec::with_capacity(self.groups.len());
        let mut next_legacy_filter_slot = 0;
        for group in self.groups {
            let mut modules = Vec::with_capacity(group.modules.len());
            for module in group.modules {
                let kind = match module.kind {
                    OSCILLATOR_KIND => ModuleKind::Oscillator(OscillatorSlot::from_index(
                        usize::from(module.oscillator_slot),
                    )?),
                    FILTER_KIND => {
                        let slot = if version >= FILTER_STATE_VERSION {
                            usize::from(module.filter_slot)
                        } else {
                            if next_legacy_filter_slot >= MAX_FILTERS {
                                continue;
                            }
                            let slot = next_legacy_filter_slot;
                            next_legacy_filter_slot += 1;
                            slot
                        };
                        ModuleKind::Filter(FilterSlot::from_index(slot)?)
                    }
                    _ => return None,
                };
                modules.push((module.id, kind));
            }
            groups.push((
                group.id,
                GroupOutput {
                    pair: group.output_pair,
                    receive_midi_channel: if version >= MIDI_ROUTING_STATE_VERSION {
                        group.output_receive_midi_channel
                    } else {
                        GroupOutput::default().receive_midi_channel
                    },
                    gain: group.output_gain,
                    pan: group.output_pan,
                    dry: if version >= PARALLEL_ROUTING_STATE_VERSION {
                        group.output_dry
                    } else {
                        GroupOutput::default().dry
                    },
                    send: if version >= PARALLEL_ROUTING_STATE_VERSION {
                        group.output_send
                    } else {
                        GroupOutput::default().send
                    },
                    sidechain: if version >= PARALLEL_ROUTING_STATE_VERSION {
                        group.output_sidechain
                    } else {
                        GroupOutput::default().sidechain
                    },
                    send_pair: if version >= PARALLEL_ROUTING_STATE_VERSION {
                        group.output_send_pair
                    } else {
                        GroupOutput::default().send_pair
                    },
                    attack: if version >= GROUP_ENVELOPE_STATE_VERSION {
                        group.output_attack
                    } else {
                        GroupOutput::default().attack
                    },
                    attack_curve: if version >= GROUP_ENVELOPE_CURVE_STATE_VERSION {
                        group.output_attack_curve
                    } else {
                        GroupOutput::default().attack_curve
                    },
                    attack_curve_time: if version >= LEGACY_AUTOMATION_STATE_VERSION {
                        group.output_attack_curve_time
                    } else {
                        GroupOutput::default().attack_curve_time
                    },
                    decay: if version >= GROUP_ENVELOPE_STATE_VERSION {
                        group.output_decay
                    } else {
                        GroupOutput::default().decay
                    },
                    decay_curve: if version >= GROUP_ENVELOPE_CURVE_STATE_VERSION {
                        group.output_decay_curve
                    } else {
                        GroupOutput::default().decay_curve
                    },
                    decay_curve_time: if version >= LEGACY_AUTOMATION_STATE_VERSION {
                        group.output_decay_curve_time
                    } else {
                        GroupOutput::default().decay_curve_time
                    },
                    sustain: if version >= GROUP_ENVELOPE_STATE_VERSION {
                        group.output_sustain
                    } else {
                        GroupOutput::default().sustain
                    },
                    release: if version >= GROUP_ENVELOPE_STATE_VERSION {
                        group.output_release
                    } else {
                        GroupOutput::default().release
                    },
                    release_curve: if version >= GROUP_ENVELOPE_CURVE_STATE_VERSION {
                        group.output_release_curve
                    } else {
                        GroupOutput::default().release_curve
                    },
                    release_curve_time: if version >= LEGACY_AUTOMATION_STATE_VERSION {
                        group.output_release_curve_time
                    } else {
                        GroupOutput::default().release_curve_time
                    },
                    envelope_enabled: if version >= LEGACY_MATERIALIZATION_STATE_VERSION {
                        group.output_envelope_enabled
                    } else {
                        version >= PARALLEL_ROUTING_STATE_VERSION
                    },
                },
                modules,
            ));
        }

        let patch = Patch::restore(groups, self.next_group_id, self.next_module_id).ok()?;
        let mut oscillators = [OscillatorConfig::default(); MAX_OSCILLATORS];
        let defaults = OscillatorConfig::default();
        for (target, mut stored) in oscillators.iter_mut().zip(self.oscillators) {
            if version == INITIAL_STATE_VERSION {
                stored.phase_position = defaults.phase_position;
                stored.phase_random = defaults.phase_random;
                stored.unison_alignment = defaults.unison_alignment;
                stored.unison_alignment_mode = defaults.unison_alignment_mode;
                stored.unison_pan_curve = defaults.unison_pan_curve;
            }
            if matches!(version, INITIAL_STATE_VERSION | SECOND_STATE_VERSION) {
                // These versions predate the XY fields and rendered with the old shape corner.
                // Preserve their sound while new oscillators use the safer alternating default.
                stored.unison_stereo_x = 1.0;
                stored.unison_stereo_alternate = 0.0;
            }
            if version < PAN_SHAPE_STATE_VERSION {
                stored.unison_pan_center_x = defaults.unison_pan_center_x;
            }
            if version < GROUP_ENVELOPE_STATE_VERSION {
                stored.unison_jitter_mode = defaults.unison_jitter_mode;
                stored.unison_weight = defaults.unison_weight;
                stored.phase_warp_mode = defaults.phase_warp_mode;
                stored.phase_warp_amount = defaults.phase_warp_amount;
            }
            *target = stored.into_config();
        }
        let mut filters = [FilterConfig::default(); MAX_FILTERS];
        if version >= FILTER_STATE_VERSION {
            for (target, stored) in filters.iter_mut().zip(self.filters) {
                *target = stored.into_config();
            }
        }
        let mut va_tables = std::array::from_fn(|_| VaTableData::default());
        for (target, stored) in va_tables.iter_mut().zip(self.va_tables) {
            *target = stored;
        }
        let mut pan_shape_curves = std::array::from_fn(|index| {
            if version >= PAN_SHAPE_STATE_VERSION {
                PanShapeCurveData::default()
            } else {
                PanShapeCurveData::from_legacy(
                    0.0,
                    1.0,
                    1.0,
                    oscillators[index].unison_pan_curve,
                    oscillators[index].unison_pan_curve,
                    0.5,
                    0.5,
                )
            }
        });
        for (target, stored) in pan_shape_curves.iter_mut().zip(self.pan_shape_curves) {
            *target = stored;
        }
        Some((
            GeneratorDocument {
                patch: std::sync::Arc::new(patch),
                oscillators,
                filters,
            },
            va_tables,
            pan_shape_curves,
            self.materialized,
            version >= LEGACY_AUTOMATION_STATE_VERSION && self.legacy_host_automation_bridge,
            legacy_automation_oscillator_masks,
            legacy_automation_group_mask,
            legacy_automation_oscillator_released,
            legacy_automation_group_released,
            legacy_pan_automation_masks,
            legacy_pan_automation_released,
        ))
    }
}

impl PersistField for GeneratorStackState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        StackDocument::from_document(
            &document,
            &self.va_tables,
            &self.pan_shape_curves,
            self.is_materialized(),
            self.legacy_host_automation_bridge_enabled(),
            self.legacy_automation_masks(),
        )
        .write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        if cursor.remaining() == 0 {
            // `Params::load_persist()` uses an empty field read to signal a
            // missing keyed entry. Generator documents did not exist before
            // the modular rack, so `post_load` must copy the host oscillators
            // into the editable stack instead of keeping today's default saw.
            self.reset_legacy();
            return;
        }
        let Some((
            loaded,
            va_tables,
            pan_shape_curves,
            materialized,
            legacy_bridge,
            legacy_oscillator_masks,
            legacy_group_mask,
            legacy_oscillator_released,
            legacy_group_released,
            legacy_pan_masks,
            legacy_pan_released,
        )) = StackDocument::read_field(cursor).and_then(StackDocument::into_document)
        else {
            // A present but malformed/unsupported field is not evidence of a
            // pre-modular state. Leave the current coherent document, tables,
            // and render selection untouched rather than mixing old data with
            // the fixed-oscillator compatibility path.
            return;
        };
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = loaded;
        for (state, data) in self.va_tables.iter().zip(va_tables) {
            state.replace(data);
        }
        for (state, data) in self.pan_shape_curves.iter().zip(pan_shape_curves) {
            state.replace(data);
        }
        // A present generator document with `materialized = false` is an
        // untranslated pre-modular session, not a finished modular patch.
        // Placeholder topology must go through `post_load` so launch copies
        // the hidden host parameters into the editable three-oscillator group.
        let pending = !materialized && Self::is_legacy_placeholder(&document);
        self.legacy_migration_pending
            .store(pending, Ordering::Release);
        self.legacy_host_automation_bridge
            .store(legacy_bridge, Ordering::Release);
        self.set_legacy_automation_masks(
            legacy_oscillator_masks,
            legacy_group_mask,
            legacy_oscillator_released,
            legacy_group_released,
            legacy_pan_masks,
            legacy_pan_released,
        );
        self.legacy_automation_epoch.fetch_add(1, Ordering::AcqRel);
        self.publish_rt(&document, materialized || !pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wave_curve::WaveCurveData;

    fn document(state: &GeneratorStackState) -> StackDocument {
        let stored = state
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        StackDocument::from_document(
            &stored,
            &state.va_tables,
            &state.pan_shape_curves,
            state.is_materialized(),
            state.legacy_host_automation_bridge_enabled(),
            state.legacy_automation_masks(),
        )
    }

    #[test]
    fn version_nine_documents_restore_silent_parallel_defaults() {
        let state = GeneratorStackState::new();
        let mut stored = document(&state);
        stored.version = FILTER_STATE_VERSION;
        stored.groups[0].output_dry = 0.0;
        stored.groups[0].output_send = 1.0;
        stored.groups[0].output_sidechain = 1.0;
        stored.groups[0].output_send_pair = 2;

        let (loaded, _, _, _, _, _, _, _, _, _, _) =
            stored.into_document().expect("version nine document");
        let output = loaded.patch.groups()[0].output();

        assert!(!output.envelope_enabled);
        assert_eq!(output.dry, 1.0);
        assert_eq!(output.send, 0.0);
        assert_eq!(output.sidechain, 0.0);
        assert_eq!(output.send_pair, 0);
    }

    #[test]
    fn version_ten_documents_enable_group_envelopes_and_restore_parallel_routing() {
        let state = GeneratorStackState::new();
        let mut stored = document(&state);
        stored.version = PARALLEL_ROUTING_STATE_VERSION;
        stored.groups[0].output_dry = 0.35;
        stored.groups[0].output_send = 0.7;
        stored.groups[0].output_sidechain = 0.8;
        stored.groups[0].output_send_pair = 3;

        let (loaded, _, _, _, _, _, _, _, _, _, _) =
            stored.into_document().expect("current document");
        let output = loaded.patch.groups()[0].output();

        assert!(output.envelope_enabled);
        assert_eq!(output.dry, 0.35);
        assert_eq!(output.send, 0.7);
        assert_eq!(output.sidechain, 0.8);
        assert_eq!(output.send_pair, 3);
    }
    #[test]
    fn version_five_documents_remain_loadable_and_materialized() {
        let state = GeneratorStackState::new();
        let mut stored = document(&state);
        stored.version = MATERIALIZED_DEFAULT_STATE_VERSION;
        stored.materialized = true;

        let (loaded, _, _, materialized, _, _, _, _, _, _, _) =
            stored.into_document().expect("version five document");

        assert_eq!(loaded.patch.groups().len(), 1);
        assert!(!loaded.patch.groups()[0].output().envelope_enabled);
        assert!(materialized);
    }

    #[test]
    fn malformed_present_document_leaves_the_current_stack_coherent() {
        let state = GeneratorStackState::new();
        assert!(state.is_materialized());
        let before = state.snapshot();
        let mut cursor = StateCursor::new(&[0xff, 0xff, 0xff]);

        state.persist_read(&mut cursor);

        assert!(state.is_materialized());
        assert_eq!(state.snapshot().groups().len(), before.groups().len());
    }

    #[test]
    fn current_documents_persist_legacy_envelope_topology_and_curve_times() {
        let state = GeneratorStackState::new();
        let mut stored = document(&state);
        stored.groups[0].output_envelope_enabled = false;
        stored.groups[0].output_attack_curve_time = -0.4;
        stored.groups[0].output_decay_curve_time = 0.25;
        stored.groups[0].output_release_curve_time = 0.7;

        let (loaded, _, _, _, _, _, _, _, _, _, _) =
            stored.into_document().expect("current document");
        let output = loaded.patch.groups()[0].output();

        assert!(!output.envelope_enabled);
        assert_eq!(output.attack_curve_time, -0.4);
        assert_eq!(output.decay_curve_time, 0.25);
        assert_eq!(output.release_curve_time, 0.7);
    }

    #[test]
    fn version_eleven_documents_keep_the_compatibility_flag_and_default_curve_times() {
        let state = GeneratorStackState::new();
        let mut stored = document(&state);
        stored.version = LEGACY_MATERIALIZATION_STATE_VERSION;
        stored.groups[0].output_envelope_enabled = false;
        stored.groups[0].output_attack_curve_time = 0.8;

        let (loaded, _, _, _, _, _, _, _, _, _, _) =
            stored.into_document().expect("version eleven document");
        let output = loaded.patch.groups()[0].output();

        assert!(!output.envelope_enabled);
        assert_eq!(output.attack_curve_time, 0.0);
        assert_eq!(output.decay_curve_time, 0.0);
        assert_eq!(output.release_curve_time, 0.0);
    }

    #[test]
    fn current_documents_preserve_positioned_va_keyframes() {
        let state = GeneratorStackState::new();
        let mut curve = WaveCurveData::default();
        curve.knots[0].value = 0.47;
        assert_eq!(
            state.va_tables[0].insert_positioned_frame(0.5, curve.clone()),
            Some(0)
        );
        let stored = document(&state);
        assert_eq!(stored.version, STATE_VERSION);

        let (_, tables, _, _, _, _, _, _, _, _, _) =
            stored.into_document().expect("current positioned document");

        assert_eq!(tables[0].positions, vec![0.5]);
        assert_eq!(tables[0].frames, vec![curve]);
    }
}
