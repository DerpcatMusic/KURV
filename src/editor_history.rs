//! Bounded whole-plugin state history for the editor thread.

use std::collections::VecDeque;

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF64};

use crate::generators::GeneratorStackSnapshot;
use crate::pan_curve::PanShapeCurveData;
use crate::wave_curve::WaveCurveData;
use crate::{KurvEditorState, KurvParams, P};

const MAX_SNAPSHOTS: usize = 32;
const MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, PartialEq)]
struct EditorSnapshot {
    params: Vec<(u32, u64)>,
    curves: [PanShapeCurveData; 3],
    wave_curves: [WaveCurveData; 11],
    generator_stack: GeneratorStackSnapshot,
    editor: KurvEditorState,
}

impl EditorSnapshot {
    fn capture(state: &PluginContext<KurvParams>, param_ids: &[u32]) -> Self {
        let params_store = state.params();
        Self {
            params: param_ids
                .iter()
                .map(|&id| (id, state.get_param(id).to_bits()))
                .collect(),
            curves: [
                params_store.pan_shape_curve_state.snapshot(),
                params_store.osc2_pan_shape_curve_state.snapshot(),
                params_store.osc3_pan_shape_curve_state.snapshot(),
            ],
            wave_curves: [
                params_store.osc1_wave_curve_state.snapshot(),
                params_store.osc2_wave_curve_state.snapshot(),
                params_store.osc3_wave_curve_state.snapshot(),
                params_store.lfo1_curve_state.snapshot(),
                params_store.lfo2_curve_state.snapshot(),
                params_store.lfo3_curve_state.snapshot(),
                params_store.lfo4_curve_state.snapshot(),
                params_store.lfo5_curve_state.snapshot(),
                params_store.lfo6_curve_state.snapshot(),
                params_store.lfo7_curve_state.snapshot(),
                params_store.lfo8_curve_state.snapshot(),
            ],
            generator_stack: params_store.generator_stack.history_snapshot(),
            editor: params_store
                .editor_state
                .lock()
                .map_or_else(|_| KurvEditorState::default(), |editor| editor.clone()),
        }
    }

    fn apply(&self, state: &PluginContext<KurvParams>) {
        for &(id, bits) in &self.params {
            let normalized = f64::from_bits(bits);
            if state.get_param(id).to_bits() != bits {
                state.set_param(id, normalized);
            }
        }
        state
            .params()
            .pan_shape_curve_state
            .replace(self.curves[0].clone());
        state
            .params()
            .osc2_pan_shape_curve_state
            .replace(self.curves[1].clone());
        state
            .params()
            .osc3_pan_shape_curve_state
            .replace(self.curves[2].clone());
        for (curve, data) in [
            &state.params().osc1_wave_curve_state,
            &state.params().osc2_wave_curve_state,
            &state.params().osc3_wave_curve_state,
            &state.params().lfo1_curve_state,
            &state.params().lfo2_curve_state,
            &state.params().lfo3_curve_state,
            &state.params().lfo4_curve_state,
            &state.params().lfo5_curve_state,
            &state.params().lfo6_curve_state,
            &state.params().lfo7_curve_state,
            &state.params().lfo8_curve_state,
        ]
        .into_iter()
        .zip(&self.wave_curves)
        {
            curve.replace(data.clone());
        }
        state
            .params()
            .generator_stack
            .restore_snapshot(&self.generator_stack);
        if let Ok(mut editor) = state.params().editor_state.lock() {
            *editor = self.editor.clone();
        }
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
    }
}

#[derive(Clone, Default)]
pub(crate) struct EditorHistory {
    param_ids: Vec<u32>,
    undo: VecDeque<EditorSnapshot>,
    current: Option<EditorSnapshot>,
    redo: VecDeque<EditorSnapshot>,
}

impl EditorHistory {
    /// Capture editor-open state once. Repeated calls are harmless.
    pub(crate) fn capture_initial(&mut self, state: &PluginContext<KurvParams>) {
        if self.current.is_none() {
            self.ensure_param_ids(state);
            let snapshot = EditorSnapshot::capture(state, &self.param_ids);
            if snapshot.retained_bytes() + self.param_ids.len() * std::mem::size_of::<u32>()
                <= MAX_RETAINED_BYTES
            {
                self.current = Some(snapshot);
            }
        }
    }

    /// Commit the state at a completed gesture boundary.
    pub(crate) fn commit(&mut self, state: &PluginContext<KurvParams>) -> bool {
        self.ensure_param_ids(state);
        let snapshot = EditorSnapshot::capture(state, &self.param_ids);
        if snapshot.retained_bytes() + self.param_ids.len() * std::mem::size_of::<u32>()
            > MAX_RETAINED_BYTES
        {
            self.clear();
            return false;
        }
        let Some(current) = self.current.replace(snapshot) else {
            return false;
        };
        if self.current.as_ref() == Some(&current) {
            self.current = Some(current);
            return false;
        }
        self.undo.push_back(current);
        self.redo.clear();
        self.trim();
        true
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self, state: &PluginContext<KurvParams>) -> bool {
        let Some(target) = self.undo.pop_back() else {
            return false;
        };
        let Some(current) = self.current.take() else {
            self.undo.push_back(target);
            return false;
        };
        target.apply(state);
        self.current = Some(target);
        self.redo.push_back(current);
        true
    }

    pub(crate) fn redo(&mut self, state: &PluginContext<KurvParams>) -> bool {
        let Some(target) = self.redo.pop_back() else {
            return false;
        };
        let Some(current) = self.current.take() else {
            self.redo.push_back(target);
            return false;
        };
        target.apply(state);
        self.current = Some(target);
        self.undo.push_back(current);
        true
    }

    /// Handle standard undo and redo shortcuts once from the editor root.
    pub(crate) fn handle_shortcuts(
        &mut self,
        ui: &egui::Ui,
        state: &PluginContext<KurvParams>,
    ) -> bool {
        if ui.ctx().egui_wants_keyboard_input() {
            return false;
        }
        let (undo, redo) = ui.input(|input| {
            let command = input.modifiers.command || input.modifiers.ctrl;
            let z = input.key_pressed(egui::Key::Z);
            let y = input.key_pressed(egui::Key::Y);
            (
                command && z && !input.modifiers.shift,
                command && (y || (z && input.modifiers.shift)),
            )
        });
        if redo {
            self.redo(state)
        } else if undo {
            self.undo(state)
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
                .filter(|&id| id != u32::from(P::PitchBend) && id != u32::from(P::SustainPedal))
                .collect();
        }
    }

    fn clear(&mut self) {
        self.undo.clear();
        self.current = None;
        self.redo.clear();
    }

    fn trim(&mut self) {
        while self.snapshot_count() > MAX_SNAPSHOTS || self.retained_bytes() > MAX_RETAINED_BYTES {
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
}
