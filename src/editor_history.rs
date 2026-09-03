//! Bounded whole-plugin and per-parameter history for the editor thread.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use truce::params::Params;
use truce_core::editor::PluginContext;

use crate::generators::{GeneratorHistoryStamp, GeneratorStackSnapshot};
use crate::modulators::routing::{
    ExtraModulationRouteSnapshot, HostAutomationTargetSnapshot, ModulationRouteTargetSnapshot,
};
use crate::modulators::state::ModulatorRackHistorySnapshot;
use crate::pan_curve::PanShapeCurveData;
use crate::resynth_state::{
    MAX_RESYNTH_HISTORY_BYTES, ResynthHistoryReceipt, ResynthHistoryRestore,
};
use crate::wave_curve::WaveCurveData;
use crate::{KurvEditorState, KurvParams, P};

const MAX_SNAPSHOTS: usize = 32;
const MAX_PARAMETER_SNAPSHOTS: usize = 32;
const MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, PartialEq)]
struct EditorSnapshot {
    params: Vec<(u32, u64)>,
    curves: [Arc<PanShapeCurveData>; 3],
    wave_curves: [Arc<WaveCurveData>; 11],
    modulation_route_targets: ModulationRouteTargetSnapshot,
    modulation_route_overflow: ExtraModulationRouteSnapshot,
    mod_wheel_route_mask: u64,
    xy_source_x_route_mask: u64,
    xy_source_y_route_mask: u64,
    modulator_rack: Arc<ModulatorRackHistorySnapshot>,
    host_automation_targets: HostAutomationTargetSnapshot,
    generator_stack: Arc<GeneratorStackSnapshot>,
    generator_stamp: GeneratorHistoryStamp,
    resynth: ResynthHistoryReceipt,
    pan_curve_generations: [u32; 3],
    wave_curve_generations: [u32; 11],
    editor: KurvEditorState,
}

impl EditorSnapshot {
    fn capture(
        state: &PluginContext<KurvParams>,
        param_ids: &[u32],
        previous: Option<&Self>,
    ) -> Option<Self> {
        let params_store = state.params();
        let pan_curve_states = [
            &params_store.pan_shape_curve_state,
            &params_store.osc2_pan_shape_curve_state,
            &params_store.osc3_pan_shape_curve_state,
        ];
        let wave_curve_states = [
            &params_store.osc1_wave_curve_state,
            &params_store.osc2_wave_curve_state,
            &params_store.osc3_wave_curve_state,
            &params_store.lfo1_curve_state,
            &params_store.lfo2_curve_state,
            &params_store.lfo3_curve_state,
            &params_store.lfo4_curve_state,
            &params_store.lfo5_curve_state,
            &params_store.lfo6_curve_state,
            &params_store.lfo7_curve_state,
            &params_store.lfo8_curve_state,
        ];
        let generator_stamp = params_store.generator_stack.history_stamp();
        let pan_curve_generations =
            std::array::from_fn(|index| pan_curve_states[index].history_generation());
        let wave_curve_generations =
            std::array::from_fn(|index| wave_curve_states[index].history_generation());
        let curves = std::array::from_fn(|index| {
            previous
                .filter(|snapshot| {
                    snapshot.pan_curve_generations[index] == pan_curve_generations[index]
                })
                .map_or_else(
                    || Arc::new(pan_curve_states[index].snapshot()),
                    |snapshot| Arc::clone(&snapshot.curves[index]),
                )
        });
        let wave_curves = std::array::from_fn(|index| {
            previous
                .filter(|snapshot| {
                    snapshot.wave_curve_generations[index] == wave_curve_generations[index]
                })
                .map_or_else(
                    || Arc::new(wave_curve_states[index].snapshot()),
                    |snapshot| Arc::clone(&snapshot.wave_curves[index]),
                )
        });
        let modulator_rack = previous
            .filter(|snapshot| {
                params_store
                    .modulator_rack
                    .matches_history_snapshot(&snapshot.modulator_rack)
            })
            .map_or_else(
                || Arc::new(params_store.modulator_rack.history_snapshot()),
                |snapshot| Arc::clone(&snapshot.modulator_rack),
            );
        let generator_stack = previous
            .filter(|snapshot| snapshot.generator_stamp == generator_stamp)
            .map_or_else(
                || Arc::new(params_store.generator_stack.history_snapshot()),
                |snapshot| Arc::clone(&snapshot.generator_stack),
            );
        let resynth = params_store
            .resynth_assets
            .history_receipt(previous.map(|snapshot| &snapshot.resynth))?;
        Some(Self {
            params: param_ids
                .iter()
                .map(|&id| {
                    (
                        id,
                        params_store
                            .get_normalized(id)
                            .unwrap_or_default()
                            .to_bits(),
                    )
                })
                .collect(),
            curves,
            wave_curves,
            modulation_route_targets: params_store.modulation_route_targets.snapshot(),
            modulation_route_overflow: params_store.modulation_route_overflow.snapshot(),
            mod_wheel_route_mask: params_store.mod_wheel_route_mask.load(),
            xy_source_x_route_mask: params_store.xy_source_x_route_mask.load(),
            xy_source_y_route_mask: params_store.xy_source_y_route_mask.load(),
            modulator_rack,
            host_automation_targets: params_store.host_automation_targets.snapshot(),
            generator_stack,
            generator_stamp,
            resynth,
            pan_curve_generations,
            wave_curve_generations,
            editor: params_store
                .editor_state
                .lock()
                .map_or_else(|_| KurvEditorState::default(), |editor| editor.clone()),
        })
    }

    fn matches_live(&self, state: &PluginContext<KurvParams>, param_ids: &[u32]) -> bool {
        let params = state.params();
        if self.generator_stamp != params.generator_stack.history_stamp()
            || !params.resynth_assets.matches_history(&self.resynth)
            || self.pan_curve_generations
                != [
                    params.pan_shape_curve_state.history_generation(),
                    params.osc2_pan_shape_curve_state.history_generation(),
                    params.osc3_pan_shape_curve_state.history_generation(),
                ]
            || self.wave_curve_generations
                != [
                    params.osc1_wave_curve_state.history_generation(),
                    params.osc2_wave_curve_state.history_generation(),
                    params.osc3_wave_curve_state.history_generation(),
                    params.lfo1_curve_state.history_generation(),
                    params.lfo2_curve_state.history_generation(),
                    params.lfo3_curve_state.history_generation(),
                    params.lfo4_curve_state.history_generation(),
                    params.lfo5_curve_state.history_generation(),
                    params.lfo6_curve_state.history_generation(),
                    params.lfo7_curve_state.history_generation(),
                    params.lfo8_curve_state.history_generation(),
                ]
        {
            return false;
        }
        if self.params.len() != param_ids.len()
            || self
                .params
                .iter()
                .zip(param_ids)
                .any(|(&(stored_id, bits), &id)| {
                    stored_id != id
                        || params
                            .get_normalized(id)
                            .is_none_or(|value| value.to_bits() != bits)
                })
        {
            return false;
        }
        if self.modulation_route_targets != params.modulation_route_targets.snapshot()
            || self.modulation_route_overflow != params.modulation_route_overflow.snapshot()
            || self.mod_wheel_route_mask != params.mod_wheel_route_mask.load()
            || self.xy_source_x_route_mask != params.xy_source_x_route_mask.load()
            || self.xy_source_y_route_mask != params.xy_source_y_route_mask.load()
            || !params
                .modulator_rack
                .matches_history_snapshot(&self.modulator_rack)
            || self.host_automation_targets != params.host_automation_targets.snapshot()
        {
            return false;
        }
        params
            .editor_state
            .lock()
            .is_ok_and(|editor| *editor == self.editor)
    }

    fn apply(&self, state: &PluginContext<KurvParams>) -> bool {
        let params = state.params();
        if params.resynth_assets.try_restore_history(&self.resynth) == ResynthHistoryRestore::Busy {
            return false;
        }
        params.modulation_route_targets.clear_all();
        params.host_automation_targets.clear_all();
        for &(id, bits) in &self.params {
            let normalized = f64::from_bits(bits);
            if params
                .get_normalized(id)
                .is_none_or(|value| value.to_bits() != bits)
            {
                state.set_param(id, normalized);
            }
        }
        params
            .pan_shape_curve_state
            .replace(self.curves[0].as_ref().clone());
        params
            .osc2_pan_shape_curve_state
            .replace(self.curves[1].as_ref().clone());
        params
            .osc3_pan_shape_curve_state
            .replace(self.curves[2].as_ref().clone());
        for (curve, data) in [
            &params.osc1_wave_curve_state,
            &params.osc2_wave_curve_state,
            &params.osc3_wave_curve_state,
            &params.lfo1_curve_state,
            &params.lfo2_curve_state,
            &params.lfo3_curve_state,
            &params.lfo4_curve_state,
            &params.lfo5_curve_state,
            &params.lfo6_curve_state,
            &params.lfo7_curve_state,
            &params.lfo8_curve_state,
        ]
        .into_iter()
        .zip(&self.wave_curves)
        {
            curve.replace(data.as_ref().clone());
        }
        params
            .modulator_rack
            .restore_history_snapshot(&self.modulator_rack);
        params
            .modulation_route_overflow
            .restore_snapshot(self.modulation_route_overflow);
        params.mod_wheel_route_mask.store(self.mod_wheel_route_mask);
        params
            .xy_source_x_route_mask
            .store(self.xy_source_x_route_mask);
        params
            .xy_source_y_route_mask
            .store(self.xy_source_y_route_mask);
        if let Ok(mut editor) = params.editor_state.lock() {
            *editor = self.editor.clone();
        }
        params.set_fast_audio_rate_modulation(self.editor.fast_audio_rate_modulation);
        params
            .modulation_route_targets
            .restore_snapshot(self.modulation_route_targets);
        params
            .host_automation_targets
            .restore_snapshot(self.host_automation_targets);
        // Publish generator topology only after every RESYNTH slot is ready.
        params
            .generator_stack
            .restore_snapshot(&self.generator_stack);
        true
    }

    fn retained_bytes(&self) -> usize {
        let generator_bytes = self.generator_stack.patch().groups().len()
            * std::mem::size_of::<crate::generators::Group>()
            + self
                .generator_stack
                .patch()
                .groups()
                .iter()
                .map(|group| {
                    group.modules().len() * std::mem::size_of::<crate::generators::Module>()
                })
                .sum::<usize>();
        self.params.len() * std::mem::size_of::<(u32, u64)>()
            + std::mem::size_of::<Self>()
            + generator_bytes
            + self.modulator_rack.retained_bytes()
    }

    fn set_param_bits(&mut self, id: u32, bits: u64) {
        if let Some((_, stored_bits)) = self
            .params
            .iter_mut()
            .find(|(stored_id, _)| *stored_id == id)
        {
            *stored_bits = bits;
        }
    }
}

#[derive(Clone, Copy)]
struct ParameterTransition {
    before: u64,
    after: u64,
}

#[derive(Clone, Default)]
struct ParameterLane {
    undo: VecDeque<ParameterTransition>,
    redo: VecDeque<ParameterTransition>,
}

#[derive(Clone, Default)]
pub(crate) struct EditorHistory {
    param_ids: Vec<u32>,
    undo: VecDeque<EditorSnapshot>,
    current: Option<EditorSnapshot>,
    redo: VecDeque<EditorSnapshot>,
    parameter_lanes: HashMap<u32, ParameterLane>,
    deferred_commit: bool,
}

impl EditorHistory {
    /// Capture editor-open state once. Repeated calls are harmless.
    pub(crate) fn capture_initial(&mut self, state: &PluginContext<KurvParams>) {
        if self.current.is_none() {
            self.ensure_param_ids(state);
            let Some(snapshot) = EditorSnapshot::capture(state, &self.param_ids, None) else {
                return;
            };
            if snapshot.retained_bytes() + self.param_ids.len() * std::mem::size_of::<u32>()
                <= MAX_RETAINED_BYTES
                && snapshot.resynth.retained_bytes() <= MAX_RESYNTH_HISTORY_BYTES
            {
                self.current = Some(snapshot);
            }
        }
    }

    /// Commit the state at a completed gesture boundary.
    pub(crate) fn commit(&mut self, state: &PluginContext<KurvParams>) -> bool {
        self.ensure_param_ids(state);
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.matches_live(state, &self.param_ids))
        {
            return false;
        }
        let Some(snapshot) = EditorSnapshot::capture(state, &self.param_ids, self.current.as_ref())
        else {
            return false;
        };
        if snapshot.retained_bytes() + self.param_ids.len() * std::mem::size_of::<u32>()
            > MAX_RETAINED_BYTES
            || snapshot.resynth.retained_bytes() > MAX_RESYNTH_HISTORY_BYTES
        {
            return false;
        }
        let Some(current) = self.current.take() else {
            return false;
        };
        if current == snapshot {
            self.current = Some(current);
            return false;
        }
        self.record_parameter_changes(&current, &snapshot);
        self.current = Some(snapshot);
        self.undo.push_back(current);
        self.redo.clear();
        self.trim();
        true
    }

    /// Move snapshot comparison/capture out of the pointer-release callback.
    /// This keeps host gesture completion and structural route publication from
    /// sharing one editor frame with whole-plugin undo bookkeeping.
    pub(crate) fn defer_commit(&mut self) {
        self.deferred_commit = true;
    }

    pub(crate) fn flush_deferred(&mut self, state: &PluginContext<KurvParams>) -> bool {
        if std::mem::take(&mut self.deferred_commit) {
            self.commit(state)
        } else {
            false
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self, state: &PluginContext<KurvParams>) -> bool {
        if self.current.is_none() || !self.undo.back().is_some_and(|target| target.apply(state)) {
            return false;
        }
        let target = self.undo.pop_back().expect("peeked undo target");
        let current = self
            .current
            .replace(target)
            .expect("checked current snapshot");
        self.redo.push_back(current);
        self.parameter_lanes.clear();
        self.trim();
        true
    }

    pub(crate) fn redo(&mut self, state: &PluginContext<KurvParams>) -> bool {
        if self.current.is_none() || !self.redo.back().is_some_and(|target| target.apply(state)) {
            return false;
        }
        let target = self.redo.pop_back().expect("peeked redo target");
        let current = self
            .current
            .replace(target)
            .expect("checked current snapshot");
        self.undo.push_back(current);
        self.parameter_lanes.clear();
        self.trim();
        true
    }

    pub(crate) fn parameter_undo(&mut self, id: u32, state: &PluginContext<KurvParams>) -> bool {
        self.parameter_transition(id, state, false)
    }

    pub(crate) fn parameter_redo(&mut self, id: u32, state: &PluginContext<KurvParams>) -> bool {
        self.parameter_transition(id, state, true)
    }

    pub(crate) fn clear_parameter_history(&mut self) {
        self.parameter_lanes.clear();
    }

    /// Handle standard undo and redo shortcuts once from the editor root.
    pub(crate) fn handle_shortcuts(
        &mut self,
        ui: &egui::Ui,
        state: &PluginContext<KurvParams>,
        hovered_parameter: Option<u32>,
    ) -> bool {
        if ui.ctx().text_edit_focused() {
            return false;
        }
        let (command, z, y, shift, alt) = ui.input(|input| {
            let command = input.modifiers.command || input.modifiers.ctrl;
            (
                command,
                input.key_pressed(egui::Key::Z),
                input.key_pressed(egui::Key::Y),
                input.modifiers.shift,
                input.modifiers.alt,
            )
        });
        if z && command && shift {
            match hovered_parameter {
                Some(id) => self.parameter_undo(id, state),
                None => self.redo(state),
            }
        } else if z && alt {
            match (command, hovered_parameter) {
                (true, Some(id)) => self.parameter_redo(id, state),
                _ => self.redo(state),
            }
        } else if z && command {
            self.undo(state)
        } else if y && command {
            self.redo(state)
        } else {
            false
        }
    }

    fn ensure_param_ids(&mut self, state: &PluginContext<KurvParams>) {
        if self.param_ids.is_empty() {
            self.param_ids = state
                .params()
                .param_infos()
                .into_iter()
                .map(|info| info.id)
                .filter(|&id| {
                    id != u32::from(P::PitchBend)
                        && id != u32::from(P::SustainPedal)
                        && id != u32::from(P::StateRevision)
                })
                .collect();
        }
    }

    fn trim(&mut self) {
        while self.snapshot_count() > MAX_SNAPSHOTS
            || self.retained_bytes() > MAX_RETAINED_BYTES
            || self.resynth_retained_bytes() > MAX_RESYNTH_HISTORY_BYTES
        {
            if self.undo.pop_front().is_none() && self.redo.pop_front().is_none() {
                break;
            }
        }
    }

    fn snapshot_count(&self) -> usize {
        self.undo.len() + usize::from(self.current.is_some()) + self.redo.len()
    }

    fn retained_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(EditorSnapshot::retained_bytes)
            .sum::<usize>()
            + self
                .current
                .as_ref()
                .map_or(0, EditorSnapshot::retained_bytes)
            + self.param_ids.len() * std::mem::size_of::<u32>()
    }

    fn resynth_retained_bytes(&self) -> usize {
        let mut allocations = HashSet::new();
        self.undo
            .iter()
            .chain(self.current.iter())
            .chain(&self.redo)
            .map(|snapshot| snapshot.resynth.accumulate_retained_bytes(&mut allocations))
            .sum()
    }

    fn record_parameter_changes(&mut self, before: &EditorSnapshot, after: &EditorSnapshot) {
        for (&(before_id, before_bits), &(after_id, after_bits)) in
            before.params.iter().zip(&after.params)
        {
            if before_id != after_id || before_bits == after_bits {
                continue;
            }
            let lane = self.parameter_lanes.entry(after_id).or_default();
            lane.undo.push_back(ParameterTransition {
                before: before_bits,
                after: after_bits,
            });
            lane.redo.clear();
            while lane.undo.len() > MAX_PARAMETER_SNAPSHOTS {
                lane.undo.pop_front();
            }
        }
    }

    fn parameter_transition(
        &mut self,
        id: u32,
        state: &PluginContext<KurvParams>,
        redo: bool,
    ) -> bool {
        let transition = {
            let Some(lane) = self.parameter_lanes.get_mut(&id) else {
                return false;
            };
            if redo {
                lane.redo.pop_back()
            } else {
                lane.undo.pop_back()
            }
        };
        let Some(transition) = transition else {
            return false;
        };
        let expected = if redo {
            transition.before
        } else {
            transition.after
        };
        if state
            .params()
            .get_normalized(id)
            .is_none_or(|value| value.to_bits() != expected)
        {
            let lane = self.parameter_lanes.entry(id).or_default();
            if redo {
                lane.redo.push_back(transition);
            } else {
                lane.undo.push_back(transition);
            }
            return false;
        }
        state.automate(
            id,
            f64::from_bits(if redo {
                transition.after
            } else {
                transition.before
            }),
        );
        if let Some(current) = self.current.as_mut() {
            current.set_param_bits(
                id,
                if redo {
                    transition.after
                } else {
                    transition.before
                },
            );
        }
        let lane = self.parameter_lanes.entry(id).or_default();
        if redo {
            lane.undo.push_back(transition);
        } else {
            lane.redo.push_back(transition);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use truce::params::Params;
    use truce_core::editor::{ClosureBridge, PluginContext};

    use super::EditorHistory;
    use crate::{
        KurvParams,
        generators::{OscillatorEngineKind, OscillatorSlot},
        oscillators::{ResynthAlgorithm, ResynthControls},
        resynth_state::{ResynthPublicationIdentity, ResynthRtPlanAck},
    };

    fn test_context() -> PluginContext<KurvParams> {
        let params = Arc::new(KurvParams::default());
        let params_for_set = Arc::clone(&params);
        let params_for_get = Arc::clone(&params);
        let params_for_plain = Arc::clone(&params);
        let params_for_format = Arc::clone(&params);
        PluginContext::new(
            Arc::new(ClosureBridge {
                begin_edit: Box::new(|_| {}),
                set_param: Box::new(move |id, normalized| {
                    params_for_set.set_normalized(id, normalized);
                }),
                end_edit: Box::new(|_| {}),
                request_resize: Box::new(|_, _| false),
                get_param: Box::new(move |id| {
                    params_for_get.get_normalized(id).unwrap_or_default()
                }),
                get_param_plain: Box::new(move |id| {
                    params_for_plain.get_plain(id).unwrap_or_default()
                }),
                format_param: Box::new(move |id| {
                    let plain = params_for_format.get_plain(id).unwrap_or_default();
                    params_for_format
                        .format_value(id, plain)
                        .unwrap_or_default()
                }),
                get_meter: Box::new(|_| 0.0),
                get_state: Box::new(Vec::new),
                set_state: Box::new(|_| {}),
                transport: Box::new(|| None),
            }),
            params,
        )
    }

    fn tone() -> Vec<u8> {
        crate::wav_test::wav_i16(
            1,
            48_000,
            (0..3_840).map(|index| {
                let sample = (std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin();
                (sample * 24_000.0) as i16
            }),
        )
    }

    fn acknowledge_current(state: &PluginContext<KurvParams>, index: usize) {
        let slot = state.params().resynth_assets.slot(index).expect("slot");
        let view = slot.try_rt_view_after(0).expect("current publication");
        let identity = view.publication_identity();
        assert!(identity.is_present());
        slot.acknowledge_rt(
            identity.generation,
            ResynthRtPlanAck {
                live_generations: [identity.generation, 0],
                accepted: ResynthPublicationIdentity {
                    generation: identity.generation,
                    revision: identity.revision,
                },
            },
        );
    }

    #[test]
    fn undo_restores_resynth_source_with_generator_engine() {
        let state = test_context();
        let mut history = EditorHistory::default();
        history.capture_initial(&state);

        let slot = OscillatorSlot::from_index(0).expect("slot zero");
        let mut config = state.params().generator_stack.oscillator_config(slot);
        config.engine = OscillatorEngineKind::Resynth;
        state
            .params()
            .generator_stack
            .set_oscillator_config(slot, config);
        let source = tone();
        let controls = ResynthControls::default();
        let model = crate::oscillators::analyze_wav("history-source.wav", source.clone(), controls)
            .expect("analyze");
        state
            .params()
            .resynth_assets
            .slot(0)
            .expect("slot")
            .replace(model, ResynthAlgorithm::Grain, controls)
            .expect("install source");
        acknowledge_current(&state, 0);
        assert!(history.commit(&state));

        state.params().resynth_assets.clear();
        acknowledge_current(&state, 0);
        state.params().generator_stack.reset_default();
        assert!(history.commit(&state));
        assert!(
            !state
                .params()
                .resynth_assets
                .slot(0)
                .expect("slot")
                .has_source()
        );

        assert!(history.undo(&state));
        let restored = state
            .params()
            .resynth_assets
            .slot(0)
            .expect("slot")
            .source_export_snapshot()
            .expect("restored Source Master");
        assert_eq!(restored.original_bytes, source);
        assert_eq!(
            state
                .params()
                .generator_stack
                .oscillator_config(slot)
                .engine,
            OscillatorEngineKind::Resynth
        );

        acknowledge_current(&state, 0);
        assert!(history.redo(&state));
        assert!(
            !state
                .params()
                .resynth_assets
                .slot(0)
                .expect("slot")
                .has_source()
        );
        assert_eq!(
            state
                .params()
                .generator_stack
                .oscillator_config(slot)
                .engine,
            OscillatorEngineKind::Va
        );
    }
}
