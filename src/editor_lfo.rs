use truce::params::{FloatParamReadF32, Params};
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{
    metric_enum_readout, metric_param_readout, metric_text_bounds, paint_metric_readout_response,
};
use crate::editor_modulation::{source_color, used_source_mask};
use crate::modulators::lfo::envelope::shaped_progress as envelope_shaped_progress;
use crate::modulators::state::{
    LEGACY_MODULATION_SOURCES, MAX_MODULATION_SOURCES, SourceConfig, SourceKind,
};
use crate::wave_curve::{
    MIN_WAVE_KNOTS, WaveCurveData, WaveCurveRt, WaveCurveState, curve_x_from_handle_progress,
    insert_knot, move_knot, remove_knot, segment_handle_phase, set_segment_bend,
    shape_segment_progress,
};
use crate::{KurvParams, P, editor_theme, editor_widgets};

mod add_menu;
mod controls;
mod envelope_editor;
mod gate_editor;
mod rack_reorder;
mod source;
mod source_card;
mod spline_editor;

use add_menu::draw_add_modulator;
use controls::draw_macro_pack_controls;
use rack_reorder::{
    draw_reorder_insertion, duplicate_source_at_active_insertion, nearest_modulator_insertion,
    place_source_at_active_insertion, place_source_pack_at_active_insertion, reorder_insertion,
};
use source::*;
pub(crate) use source::{
    set_source_bipolar, source_bipolar, source_config, source_is_running, source_value_meter,
};
use source_card::{collapsed_module_height, draw_source_module, expanded_module_height};
pub(crate) use spline_editor::{
    clear_curve_data_edit_state, draw_curve_state_in_rect, edit_curve_data_in_rect,
};
use spline_editor::{meter_is_moving, request_graph_repaint};

const MODES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
const RATE_MODES: [&str; 4] = ["Hz", "ms", "BEAT", "KEY"];
const SHAPES: [&str; 4] = ["CURVE", "RAND HOLD", "RAND SMOOTH", "TRANCE GATE"];
const SYNC_DIVISIONS: [&str; 16] = [
    "1/64", "1/32T", "1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4T", "1/4", "1/2T", "1/2", "1/1T",
    "1/1", "2/1", "4/1", "8/1",
];
const ENVELOPE_CURVE_SEGMENTS: usize = 12;
const ENVELOPE_HOLD_WEIGHT: f32 = 0.32;
const ENVELOPE_TIME_WEIGHT_OFFSET: f32 = 0.002;
const LIVE_METER_REPAINT: std::time::Duration = std::time::Duration::from_millis(100);
const IDLE_METER_REPAINT: std::time::Duration = std::time::Duration::from_millis(300);
const SPLINE_EDIT_PUBLISH_INTERVAL_SECONDS: f64 = 1.0 / 30.0;
const MACRO_PACK_CAPACITY: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AddMenu {
    presentation_insertion: usize,
    insertion: bool,
}

#[derive(Clone, Copy)]
struct ModulationUi {
    selected: usize,
    add_menu: Option<AddMenu>,
    alt_insertion: Option<usize>,
    reorder: Option<ModulatorReorder>,
    editor_pointer_inside: bool,
    rack_active: u64,
    rack_order: [u8; MAX_MODULATION_SOURCES],
}

impl Default for ModulationUi {
    fn default() -> Self {
        Self {
            selected: 0,
            add_menu: None,
            alt_insertion: None,
            reorder: None,
            editor_pointer_inside: false,
            rack_active: 0,
            rack_order: [0; MAX_MODULATION_SOURCES],
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ModulatorReorder {
    source_slot: usize,
    presentation_insertion: usize,
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
    drag_origin: Option<WaveCurveData>,
    snap_phase: Option<f32>,
    snap_value: Option<f32>,
    last_publish_time: f64,
    last_meter: Option<f32>,
    meter_motion_frames: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvelopeDrag {
    Attack,
    AttackCurve,
    DecaySustain,
    DecayCurve,
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
    density: f32,
) {
    let id = ui.id().with("modulation-ui");
    let mut view = ui
        .data(|data| data.get_temp::<ModulationUi>(id))
        .unwrap_or_default();
    let editor_pointer_inside = view.editor_pointer_inside;
    view.editor_pointer_inside = false;
    let mut active = active_source_mask(state);
    let presentation_order = state.params().modulator_rack.presentation_order();
    if view.rack_active != active || view.rack_order != presentation_order {
        view.add_menu = None;
        view.alt_insertion = None;
        view.reorder = None;
        view.rack_active = active;
        view.rack_order = presentation_order;
    }
    if active != 0 && active & (1_u64 << view.selected) == 0 {
        view.selected = first_presented_active_source(state, active).unwrap_or_default();
    }
    let module_height = expanded_module_height(ui, density);
    let collapsed_modulators = collapsed_modulator_mask(state);
    let viewport = egui::Rect::from_min_size(ui.cursor().left_top(), egui::vec2(width, height));
    egui::ScrollArea::vertical()
        .id_salt("modulator-rack")
        .max_height(height)
        .auto_shrink([false, false])
        .content_margin(egui::Margin::ZERO)
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
            let (pack_sources, pack_len) = macro_pack_sources(state, active);
            draw_macro_pack_controls(
                ui,
                state,
                &pack_sources[..pack_len],
                &mut active,
                &mut view.selected,
                width,
                collapsed_module_height(ui) * 9.2,
            );
            let mut visible_source_storage = [0_usize; MAX_MODULATION_SOURCES];
            let mut visible_source_count = 0;
            for source in presentation_order {
                let index = usize::from(source);
                if active & (1_u64 << index) != 0
                    && !source_kind_is_static(source_kind(state, index))
                {
                    visible_source_storage[visible_source_count] = index;
                    visible_source_count += 1;
                }
            }
            let visible_sources = &visible_source_storage[..visible_source_count];
            if view
                .reorder
                .is_some_and(|drag| active & (1_u64 << drag.source_slot) == 0)
            {
                view.reorder = None;
            }
            if let Some(mut drag) = view.reorder {
                let (focused, escape, primary_down, released, copy, pointer) = ui.input(|input| {
                    (
                        input.focused,
                        input.key_pressed(egui::Key::Escape),
                        input.pointer.primary_down(),
                        input.pointer.button_released(egui::PointerButton::Primary),
                        input.modifiers.ctrl,
                        input.pointer.latest_pos(),
                    )
                });
                if escape || !focused {
                    view.reorder = None;
                } else {
                    if let Some(pointer) = pointer {
                        drag.presentation_insertion = reorder_insertion(
                            ui,
                            visible_sources,
                            collapsed_modulators,
                            module_height,
                            drag.presentation_insertion,
                            pointer,
                        );
                    }
                    if primary_down {
                        view.reorder = Some(drag);
                        editor_theme::request_display_repaint(ui);
                    } else if released {
                        if source_kind_is_static(source_kind(state, drag.source_slot)) {
                            place_source_pack_at_active_insertion(
                                state,
                                macro_pack_mask(state, active),
                                active,
                                visible_sources,
                                drag.presentation_insertion,
                            );
                        } else if copy {
                            if let Some(destination) = duplicate_source_at_active_insertion(
                                state,
                                drag.source_slot,
                                &mut active,
                                drag.presentation_insertion,
                            ) {
                                view.selected = destination;
                            }
                        } else {
                            place_source_at_active_insertion(
                                state,
                                drag.source_slot,
                                active,
                                drag.presentation_insertion,
                            );
                        }
                        view.reorder = None;
                    } else {
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
            let insertion_blocked = view.reorder.is_some()
                || ui.ctx().dragged_id().is_some()
                || egui::DragAndDrop::has_payload_of_type::<crate::generators::ModuleId>(ui.ctx())
                || egui::DragAndDrop::has_payload_of_type::<crate::generators::GroupId>(ui.ctx())
                || crate::editor_modulation::source_drag_active(ui);
            if insertion_blocked {
                view.add_menu = None;
                view.alt_insertion = None;
            }
            let visible_insertion = (!insertion_blocked)
                .then(|| {
                    view.add_menu
                        .filter(|menu| menu.insertion)
                        .map(|menu| menu.presentation_insertion)
                        .or_else(|| {
                            (view.add_menu.is_none()
                                && !editor_pointer_inside
                                && active.count_ones() < MAX_MODULATION_SOURCES as u32)
                                .then(|| {
                                    nearest_modulator_insertion(
                                        ui,
                                        visible_sources,
                                        collapsed_modulators,
                                        module_height,
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
            let reorder = view
                .reorder
                .map(|drag| (drag.presentation_insertion, drag.source_slot));
            // Reorder and modulation payloads live in egui state, and the add menu keeps its own
            // row alive. Offscreen source cards do not own popup state and stay culled.
            let mut presentation_insertion = 0;
            for &index in visible_sources {
                if let Some((_, source_slot)) =
                    reorder.filter(|(insertion, _)| *insertion == presentation_insertion)
                {
                    draw_reorder_insertion(ui, width, source_slot);
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
                let dragged = view.reorder.is_some_and(|drag| drag.source_slot == index);
                editor_widgets::with_dragged_layer(
                    ui,
                    egui::Id::new(("modulator-drag-layer", index)),
                    dragged,
                    |ui| {
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
                    },
                );
                presentation_insertion += 1;
            }
            if let Some((_, source_slot)) =
                reorder.filter(|(insertion, _)| *insertion == presentation_insertion)
            {
                draw_reorder_insertion(ui, width, source_slot);
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
        });
    ui.data_mut(|data| data.insert_temp(id, view));
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
