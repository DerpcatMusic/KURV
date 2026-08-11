use truce::params::FloatParamReadF32;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{
    metric_enum_readout, metric_param_readout, paint_metric_readout_response,
};
use crate::editor_modulation::{source_color, used_source_mask};
use crate::modulators::lfo::envelope::shaped_progress as envelope_shaped_progress;
use crate::modulators::state::{LEGACY_MODULATION_SOURCES, MAX_MODULATION_SOURCES, SourceKind};
use crate::wave_curve::{
    MIN_WAVE_KNOTS, WaveCurveData, WaveCurveRt, WaveCurveState, insert_knot, move_knot,
    remove_knot, set_segment_curve,
};
use crate::{KurvParams, P, editor_theme, editor_widgets};

mod add_menu;
mod controls;
mod envelope_editor;
mod rack_reorder;
mod source;
mod source_card;
mod spline_editor;

use add_menu::draw_add_modulator;
use rack_reorder::{
    draw_reorder_insertion, nearest_modulator_insertion, place_source_at_active_insertion,
    reorder_insertion,
};
use source::*;
use source_card::{
    collapsed_module_height, draw_source_module, expanded_module_height, paint_modulator_drag_ghost,
};
use spline_editor::{meter_is_moving, request_graph_repaint};

const MODES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
const RATE_MODES: [&str; 4] = ["Hz", "ms", "BEAT", "KEY"];
const SYNC_DIVISIONS: [&str; 16] = [
    "1/64", "1/32T", "1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4T", "1/4", "1/2T", "1/2", "1/1T",
    "1/1", "2/1", "4/1", "8/1",
];
const ENVELOPE_CURVE_SEGMENTS: usize = 12;
const ENVELOPE_HOLD_WEIGHT: f32 = 0.32;
const ENVELOPE_TIME_WEIGHT_OFFSET: f32 = 0.002;
const LIVE_METER_REPAINT: std::time::Duration = std::time::Duration::from_millis(100);
const IDLE_METER_REPAINT: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AddMenu {
    presentation_insertion: usize,
    insertion: bool,
}

#[derive(Clone, Copy, Default)]
struct ModulationUi {
    selected: usize,
    add_menu: Option<AddMenu>,
    alt_insertion: Option<usize>,
    reorder: Option<ModulatorReorder>,
}

#[derive(Clone, Copy, Debug)]
struct ModulatorReorder {
    source_slot: usize,
    presentation_insertion: usize,
    card_size: egui::Vec2,
    header_height: f32,
    collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplineDrag {
    Point(usize),
    Tension(usize),
}

#[derive(Clone, Default)]
struct SplineEditorUi {
    selected: Option<SplineDrag>,
    drag: Option<SplineDrag>,
    context_target: Option<SplineDrag>,
    draft: Option<WaveCurveData>,
    snap_phase: Option<f32>,
    snap_value: Option<f32>,
    last_meter: Option<f32>,
    meter_motion_frames: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvelopeDrag {
    Attack,
    AttackCurve,
    DecaySustain,
    DecayCurve,
    Sustain,
    Release,
    ReleaseCurve,
}

#[derive(Clone, Copy, Default)]
struct EnvelopeEditorUi {
    drag: Option<EnvelopeDrag>,
    selected: Option<EnvelopeDrag>,
    context_target: Option<EnvelopeDrag>,
    drag_pointer_origin: Option<egui::Pos2>,
    drag_handle_origin: Option<egui::Pos2>,
    drag_precision: f32,
    last_meter: Option<f32>,
    meter_motion_frames: u8,
}

pub(crate) fn modulation_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let id = ui.id().with("modulation-ui");
    let mut view = ui
        .data(|data| data.get_temp::<ModulationUi>(id))
        .unwrap_or_default();
    let mut active = active_source_mask(state);
    if active != 0 && active & (1_u64 << view.selected) == 0 {
        view.selected = first_presented_active_source(state, active).unwrap_or_default();
    }
    let module_height = expanded_module_height(ui);
    let collapsed_modulators = collapsed_modulator_mask(state);
    let viewport = egui::Rect::from_min_size(ui.cursor().left_top(), egui::vec2(width, height));
    egui::ScrollArea::vertical()
        .id_salt("modulator-rack")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
            let presentation_order = state.params().modulator_rack.presentation_order();
            let visible_sources: Vec<_> = presentation_order
                .into_iter()
                .map(usize::from)
                .filter(|index| active & (1_u64 << index) != 0)
                .collect();
            if view
                .reorder
                .is_some_and(|drag| active & (1_u64 << drag.source_slot) == 0)
            {
                view.reorder = None;
            }
            if let Some(mut drag) = view.reorder {
                let (escape, primary_down, pointer) = ui.input(|input| {
                    (
                        input.key_pressed(egui::Key::Escape),
                        input.pointer.primary_down(),
                        input.pointer.latest_pos(),
                    )
                });
                if escape {
                    view.reorder = None;
                } else {
                    if let Some(pointer) = pointer {
                        drag.presentation_insertion =
                            reorder_insertion(ui, &visible_sources, collapsed_modulators, pointer);
                    }
                    if primary_down {
                        view.reorder = Some(drag);
                        editor_theme::request_display_repaint(ui);
                    } else {
                        place_source_at_active_insertion(
                            state,
                            drag.source_slot,
                            active,
                            drag.presentation_insertion,
                        );
                        view.reorder = None;
                    }
                }
            }
            editor_widgets::drag_edge_scroll(ui, viewport, view.reorder.is_some());
            view.add_menu = view.add_menu.filter(|menu| {
                if menu.insertion {
                    menu.presentation_insertion < visible_sources.len()
                } else {
                    menu.presentation_insertion == visible_sources.len()
                }
            });
            let visible_insertion = view
                .reorder
                .is_none()
                .then(|| {
                    view.add_menu
                        .filter(|menu| menu.insertion)
                        .map(|menu| menu.presentation_insertion)
                        .or_else(|| {
                            view.add_menu
                                .is_none()
                                .then(|| {
                                    nearest_modulator_insertion(
                                        ui,
                                        &visible_sources,
                                        collapsed_modulators,
                                        view.alt_insertion,
                                    )
                                })
                                .flatten()
                        })
                })
                .flatten();
            view.alt_insertion = if view.reorder.is_none()
                && view.add_menu.is_none()
                && ui.input(|input| input.modifiers.alt)
            {
                visible_insertion
            } else {
                None
            };
            let reorder_insertion = view.reorder.map(|drag| drag.presentation_insertion);
            // Reorder and modulation payloads live in egui state, and the add menu keeps its own
            // row alive. Offscreen source cards do not own popup state and stay culled.
            let mut presentation_insertion = 0;
            for &index in &visible_sources {
                if reorder_insertion == Some(presentation_insertion) {
                    draw_reorder_insertion(ui, width, view.reorder.unwrap().source_slot);
                }
                if visible_insertion == Some(presentation_insertion) {
                    draw_add_modulator(
                        ui,
                        state,
                        &mut view,
                        &mut active,
                        width,
                        presentation_insertion,
                        true,
                    );
                }
                draw_source_module(
                    ui,
                    state,
                    &mut view,
                    &mut active,
                    index,
                    presentation_insertion,
                    collapsed_modulators & (1_u64 << index) != 0,
                    width,
                    module_height,
                    false,
                );
                presentation_insertion += 1;
            }
            if reorder_insertion == Some(presentation_insertion) {
                draw_reorder_insertion(ui, width, view.reorder.unwrap().source_slot);
            }
            draw_add_modulator(
                ui,
                state,
                &mut view,
                &mut active,
                width,
                presentation_insertion,
                false,
            );
            if let Some(drag) = view.reorder {
                paint_modulator_drag_ghost(ui, state, drag);
            }
        });
    ui.data_mut(|data| data.insert_temp(id, view));
}

pub(crate) fn preferred_height(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    maximum: f32,
) -> f32 {
    let active = active_source_mask(state);
    let collapsed = collapsed_modulator_mask(state);
    let mut height = editor_theme::title_height(ui);
    let mut visible = 0;
    for source_slot in state.params().modulator_rack.presentation_order() {
        let index = usize::from(source_slot);
        if active & (1_u64 << index) == 0 {
            continue;
        }
        height += if collapsed & (1_u64 << index) != 0 {
            collapsed_module_height(ui)
        } else {
            expanded_module_height(ui)
        };
        visible += 1;
    }
    (height + editor_theme::compact_gap(ui) * visible as f32).min(maximum.max(1.0))
}

fn first_presented_active_source(state: &PluginContext<KurvParams>, active: u64) -> Option<usize> {
    state
        .params()
        .modulator_rack
        .presentation_order()
        .into_iter()
        .map(usize::from)
        .find(|index| active & (1_u64 << index) != 0)
}

fn rack_item_visible(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.is_rect_visible(rect) && rect.intersect(ui.clip_rect()).is_positive()
}

fn collapsed_modulator_mask(state: &PluginContext<KurvParams>) -> u64 {
    state
        .params()
        .editor_state
        .lock()
        .map_or(0, |editor| editor.collapsed_modulators)
}
