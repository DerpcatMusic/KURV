use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use super::*;

const INITIAL_STATE_VERSION: u32 = 1;
const SECOND_STATE_VERSION: u32 = 2;
const PREVIOUS_STATE_VERSION: u32 = 3;
const PAN_SHAPE_STATE_VERSION: u32 = 4;
const GROUP_ENVELOPE_STATE_VERSION: u32 = 6;
const GROUP_ENVELOPE_CURVE_STATE_VERSION: u32 = 7;
const MIDI_ROUTING_STATE_VERSION: u32 = 8;
const STATE_VERSION: u32 = 9;
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
        }
    }

    fn into_config(self) -> FilterConfig {
        sanitize_filter_config(FilterConfig {
            mode: filter_mode_from_encoded(self.mode),
            cutoff_hz: self.cutoff_hz,
            q: self.q,
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
                    output_attack: group.output().attack,
                    output_attack_curve: group.output().attack_curve,
                    output_decay: group.output().decay,
                    output_decay_curve: group.output().decay_curve,
                    output_sustain: group.output().sustain,
                    output_release: group.output().release,
                    output_release_curve: group.output().release_curve,
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
        }
    }

    fn into_document(
        self,
    ) -> Option<(
        GeneratorDocument,
        [VaTableData; MAX_OSCILLATORS],
        [PanShapeCurveData; MAX_OSCILLATORS],
        bool,
    )> {
        let version = self.version;
        if !matches!(
            version,
            INITIAL_STATE_VERSION
                | SECOND_STATE_VERSION
                | PREVIOUS_STATE_VERSION
                | PAN_SHAPE_STATE_VERSION
                | GROUP_ENVELOPE_STATE_VERSION
                | GROUP_ENVELOPE_CURVE_STATE_VERSION
                | MIDI_ROUTING_STATE_VERSION
                | STATE_VERSION
        ) || self.next_group_id == 0
            || self.next_module_id == 0
        {
            return None;
        }

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
                        let slot = if version >= STATE_VERSION {
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
                stored.unison_stereo_x = defaults.unison_stereo_x;
                stored.unison_stereo_alternate = defaults.unison_stereo_alternate;
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
        if version >= STATE_VERSION {
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
                patch,
                oscillators,
                filters,
            },
            va_tables,
            pan_shape_curves,
            self.materialized,
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
        )
        .write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some((loaded, va_tables, pan_shape_curves, materialized)) =
            StackDocument::read_field(cursor).and_then(StackDocument::into_document)
        else {
            let document = self
                .document
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.publish_rt(&document, false);
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
        self.publish_rt(&document, materialized);
    }
}
