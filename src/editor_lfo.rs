use truce::params::{FloatParamReadF32, Params};
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{
    metric_enum_readout, metric_param_readout, paint_metric_readout_response,
};
use crate::editor_modulation::{clear_source, source_color, source_handle, used_source_mask};
use crate::modulators::lfo::envelope::shaped_progress as envelope_shaped_progress;
use crate::modulators::state::{LEGACY_MODULATION_SOURCES, MAX_MODULATION_SOURCES, SourceKind};
use crate::wave_curve::{
    MIN_WAVE_KNOTS, WaveCurveData, WaveCurveRt, WaveCurveState, insert_knot, move_knot,
    remove_knot, set_segment_curve,
};
use crate::{KurvParams, P, editor_theme, editor_widgets};

mod controls;
mod envelope_editor;
mod source;
mod spline_editor;

use controls::{collapsed_source_summary, draw_controls, draw_envelope_controls};
use envelope_editor::{draw_envelope_curve, envelope_path};
use source::*;
use spline_editor::{draw_curve, draw_in_rect, meter_is_moving, request_graph_repaint};

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
            // Reorder and modulation payloads live in egui state; offscreen module painting is
            // not required to keep them alive. Culling here prevents one drag from repainting
            // every modulator and all of its route destinations.
            let keep_rack_interactions_alive = ui.ctx().any_popup_open();
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
                    keep_rack_interactions_alive,
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

fn draw_source_module(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    view: &mut ModulationUi,
    active: &mut u64,
    index: usize,
    presentation_index: usize,
    collapsed: bool,
    width: f32,
    height: f32,
    keep_interaction_alive: bool,
) {
    let mut collapsed = collapsed;
    let header_height = collapsed_module_height(ui);
    let shown_height = if collapsed { header_height } else { height };
    let (_, rect) = ui.allocate_space(egui::vec2(width, shown_height));
    if !rack_item_visible(ui, rect) && !keep_interaction_alive {
        return;
    }
    let palette = editor_theme::semantic();
    let color = source_color(index);
    let mut selected = view.selected == index;
    let envelope = source_is_envelope(state, index);
    let source_label = format!("{} {}", if envelope { "ENV" } else { "LFO" }, index + 1);
    let card_hovered = ui.rect_contains_pointer(rect);
    if card_hovered && ui.input(|input| input.pointer.primary_clicked()) {
        view.selected = index;
        selected = true;
    }
    ui.painter().rect_filled(rect, 0.0, palette.well);
    if selected || card_hovered {
        ui.painter().line_segment(
            [rect.left_top(), rect.left_bottom()],
            egui::Stroke::new(
                if selected {
                    editor_theme::shape::FOCUS_STROKE
                } else {
                    editor_theme::shape::STROKE
                },
                color.gamma_multiply(if selected { 0.78 } else { 0.46 }),
            ),
        );
    }

    let controls_height = if collapsed {
        0.0
    } else {
        editor_theme::title_height(ui)
    };
    let controls_bottom_inset = if collapsed {
        0.0
    } else {
        editor_theme::space::XS.min(rect.height() * 0.06)
    };
    let graph = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(
            rect.right(),
            rect.bottom() - controls_height - controls_bottom_inset,
        ),
    );
    let controls = egui::Rect::from_min_max(
        egui::pos2(rect.left(), graph.bottom()),
        egui::pos2(rect.right(), rect.bottom() - controls_bottom_inset),
    );
    let header = egui::Rect::from_min_size(graph.min, egui::vec2(graph.width(), header_height));
    let action_size = header.height();
    let collapse_rect = egui::Rect::from_center_size(
        header.left_center() + egui::vec2(action_size * 0.5, 0.0),
        egui::Vec2::splat(action_size),
    );
    let remove_rect = egui::Rect::from_center_size(
        header.right_center() - egui::vec2(action_size * 0.5, 0.0),
        egui::Vec2::splat(action_size),
    );
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), header.top()),
        egui::pos2(remove_rect.left(), header.bottom()),
    );
    let grip_width =
        (editor_theme::space::XS + editor_theme::space::SM).min(drag_rect.width() * 0.34);
    let grip_rect = egui::Rect::from_min_max(
        drag_rect.min,
        egui::pos2(drag_rect.left() + grip_width, drag_rect.bottom()),
    );
    let source_label_width = ui
        .painter()
        .layout_no_wrap(source_label.clone(), editor_theme::font::label(), color)
        .size()
        .x;
    let source_width = (source_label_width + action_size * 0.72 + editor_theme::space::XS * 2.0)
        .min((drag_rect.width() - grip_width).max(0.0));
    let source_rect = egui::Rect::from_min_max(
        egui::pos2(grip_rect.right(), drag_rect.top()),
        egui::pos2(grip_rect.right() + source_width, drag_rect.bottom()),
    );
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(source_rect.right(), drag_rect.top()),
        drag_rect.max,
    );
    let header_id = ui.id().with(("lfo-module", index));
    let header_response = ui
        .interact(title_rect, header_id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Select this modulator; double-click to collapse");
    let source_response = ui.interact(
        source_rect,
        header_id.with("source"),
        egui::Sense::click_and_drag(),
    );
    let grip_response = ui
        .interact(
            grip_rect,
            header_id.with("reorder"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to reorder this modulator card");
    let collapse = ui
        .interact(
            collapse_rect,
            header_id.with("collapse"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Expand modulator"
        } else {
            "Collapse modulator"
        });
    let remove = ui
        .interact(remove_rect, header_id.with("remove"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove modulator and its routes");
    if grip_response.drag_started() {
        view.selected = index;
        selected = true;
        view.reorder = Some(ModulatorReorder {
            source_slot: index,
            presentation_insertion: presentation_index,
            card_size: rect.size(),
            header_height,
            collapsed,
        });
    }
    if grip_response.dragged()
        && let Some(drag) = view
            .reorder
            .as_mut()
            .filter(|drag| drag.source_slot == index)
    {
        drag.card_size = rect.size();
        drag.header_height = header_height;
        drag.collapsed = collapsed;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        editor_theme::request_display_repaint(ui);
    }
    if source_response.is_pointer_button_down_on()
        || source_response.drag_started()
        || source_response.clicked()
    {
        view.selected = index;
        selected = true;
    }
    let source_active = source_response.dragged() || source_response.is_pointer_button_down_on();
    let reorder_active = view.reorder.is_some_and(|drag| drag.source_slot == index);
    if selected
        || header_response.hovered()
        || source_response.hovered()
        || grip_response.hovered()
        || collapse.hovered()
        || remove.hovered()
    {
        let visuals = editor_theme::control_visuals(
            true,
            header_response.hovered()
                || source_response.hovered()
                || grip_response.hovered()
                || collapse.hovered()
                || remove.hovered(),
            selected || source_active || reorder_active,
            header_response.has_focus()
                || source_response.has_focus()
                || grip_response.has_focus()
                || collapse.has_focus()
                || remove.has_focus(),
            color,
        );
        ui.painter().rect_filled(
            header.shrink2(egui::vec2(editor_theme::shape::STROKE, 0.0)),
            1.0,
            visuals.fill,
        );
    }
    if source_active || reorder_active {
        ui.painter().rect_stroke(
            rect.shrink(editor_theme::shape::STROKE),
            1.0,
            egui::Stroke::new(
                editor_theme::shape::FOCUS_STROKE,
                color.gamma_multiply(0.84),
            ),
            egui::StrokeKind::Inside,
        );
    }
    let dot_radius = editor_theme::shape::STROKE;
    let grip_spacing = editor_theme::space::XXS;
    let origin = grip_rect.center() - egui::vec2(grip_spacing * 0.5, grip_spacing);
    let grip_color = if reorder_active {
        palette.text
    } else if grip_response.hovered() {
        color
    } else {
        palette.text_muted.gamma_multiply(0.56)
    };
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                origin + egui::vec2(column as f32 * grip_spacing, row as f32 * grip_spacing),
                dot_radius,
                grip_color,
            );
        }
    }
    source_handle(ui, state, index, &source_label, &source_response)
        .on_hover_text("Drag this source onto a highlighted parameter");
    let keyboard_activate = ui
        .input(|input| input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space));
    if collapse.clicked()
        || (collapse.has_focus() && keyboard_activate)
        || header_response.double_clicked()
    {
        collapsed = !collapsed;
        set_modulator_collapsed(state, index, collapsed);
        editor_theme::request_display_repaint(ui);
    }
    if remove.clicked()
        || (remove.has_focus()
            && (keyboard_activate || ui.input(|input| input.key_pressed(egui::Key::Delete))))
    {
        clear_source(state, (index + 1) as u8);
        *active &= !(1_u64 << index);
        set_source_active(state, index, false, SourceKind::Lfo);
        view.selected = if *active == 0 {
            0
        } else {
            first_presented_active_source(state, *active).unwrap_or_default()
        };
        return;
    }
    if header_response.clicked() {
        view.selected = index;
    }
    let marker_center = collapse_rect.center();
    let marker_size = collapse_rect.height() * 0.30;
    let marker_points = if collapsed {
        vec![
            marker_center + egui::vec2(-marker_size * 0.36, -marker_size * 0.56),
            marker_center + egui::vec2(-marker_size * 0.36, marker_size * 0.56),
            marker_center + egui::vec2(marker_size * 0.52, 0.0),
        ]
    } else {
        vec![
            marker_center + egui::vec2(-marker_size * 0.56, -marker_size * 0.36),
            marker_center + egui::vec2(marker_size * 0.56, -marker_size * 0.36),
            marker_center + egui::vec2(0.0, marker_size * 0.52),
        ]
    };
    if collapse.hovered() || collapse.is_pointer_button_down_on() || collapse.has_focus() {
        let visuals = editor_theme::control_visuals(
            true,
            collapse.hovered(),
            collapse.is_pointer_button_down_on(),
            collapse.has_focus(),
            color,
        );
        ui.painter().rect(
            collapse_rect,
            1.0,
            visuals.fill,
            visuals.stroke,
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().add(egui::Shape::convex_polygon(
        marker_points,
        if collapse.hovered() || collapse.is_pointer_button_down_on() || collapse.has_focus() {
            color
        } else {
            palette.text_muted
        },
        egui::Stroke::NONE,
    ));
    if title_rect.width() > header.height() * 5.0 {
        let text = if source_active {
            if drag_rect.width() > header.height() * 8.0 {
                "DROP ON CONTROL".to_owned()
            } else {
                "DRAG".to_owned()
            }
        } else if source_response.hovered() {
            "DRAG TO MODULATE".to_owned()
        } else if collapsed {
            collapsed_source_summary(state, index, envelope)
        } else if envelope {
            format!(
                "{:.0}%",
                source_value_meter(state, index).clamp(0.0, 1.0) * 100.0
            )
        } else {
            format!("{:+.2}", source_value_meter(state, index).clamp(-1.0, 1.0))
        };
        let text_font = if source_active || source_response.hovered() || collapsed {
            editor_theme::font::caption()
        } else {
            editor_theme::font::value()
        };
        let text_width = ui
            .painter()
            .layout_no_wrap(text.clone(), text_font.clone(), palette.text_muted)
            .size()
            .x;
        if text_width + editor_theme::space::XS * 2.0 < title_rect.width() {
            ui.painter().text(
                title_rect.right_center() - egui::vec2(editor_theme::space::XS, 0.0),
                egui::Align2::RIGHT_CENTER,
                text,
                text_font,
                if source_active {
                    palette.text
                } else if source_response.hovered() {
                    color
                } else {
                    palette.text_muted
                },
            );
        }
    }
    if remove.hovered() || remove.is_pointer_button_down_on() || remove.has_focus() {
        let visuals = editor_theme::control_visuals(
            true,
            remove.hovered(),
            remove.is_pointer_button_down_on(),
            remove.has_focus(),
            palette.danger,
        );
        ui.painter().rect(
            remove_rect,
            1.0,
            visuals.fill,
            visuals.stroke,
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        remove_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::label(),
        if remove.hovered() || remove.is_pointer_button_down_on() || remove.has_focus() {
            palette.danger
        } else if selected || card_hovered {
            palette.text_muted
        } else {
            palette.text_muted.gamma_multiply(0.44)
        },
    );
    if collapsed {
        paint_reorder_origin(ui, rect, reorder_active, color);
        return;
    }

    let graph_body = egui::Rect::from_min_max(
        egui::pos2(graph.left(), header.bottom()),
        graph.right_bottom(),
    );
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.42),
        ),
    );
    let graph_inner = graph_body.shrink2(egui::vec2(
        editor_theme::space::XXS,
        editor_theme::compact_gap(ui),
    ));
    draw_in_rect(ui, graph_inner, ("source-graph", index), |ui| {
        if envelope {
            draw_envelope_curve(ui, state, index, graph_inner.width(), graph_inner.height());
        } else {
            draw_curve(ui, state, index, graph_inner.width(), graph_inner.height());
        }
    });
    ui.painter().line_segment(
        [controls.left_top(), controls.right_top()],
        egui::Stroke::new(1.0_f32, palette.grid.gamma_multiply(0.58)),
    );
    draw_in_rect(ui, controls, ("source-controls", index), |ui| {
        if envelope {
            draw_envelope_controls(ui, state, index, controls.width(), controls.height());
        } else {
            draw_controls(ui, state, index, controls.width(), controls.height());
        }
    });
    paint_reorder_origin(ui, rect, reorder_active, color);
}

fn expanded_module_height(ui: &egui::Ui) -> f32 {
    editor_theme::title_height(ui) * 4.05
}

fn collapsed_module_height(ui: &egui::Ui) -> f32 {
    editor_theme::title_height(ui) * 0.72
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

fn set_modulator_collapsed(state: &PluginContext<KurvParams>, index: usize, collapsed: bool) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        let bit = 1_u64 << index;
        if collapsed {
            editor.collapsed_modulators |= bit;
        } else {
            editor.collapsed_modulators &= !bit;
        }
    }
}

fn reorder_insertion(
    ui: &egui::Ui,
    visible_sources: &[usize],
    collapsed_modulators: u64,
    pointer: egui::Pos2,
) -> usize {
    let gap = ui.spacing().item_spacing.y;
    let mut top = ui.cursor().top();
    for (insertion, index) in visible_sources.iter().enumerate() {
        let height = if collapsed_modulators & (1_u64 << *index) != 0 {
            collapsed_module_height(ui)
        } else {
            expanded_module_height(ui)
        };
        if pointer.y < top + height * 0.5 {
            return insertion;
        }
        top += height + gap;
    }
    visible_sources.len()
}

fn draw_reorder_insertion(ui: &mut egui::Ui, width: f32, source_slot: usize) {
    let color = source_color(source_slot);
    let height = (editor_theme::compact_gap(ui) * 2.0).max(editor_theme::title_height(ui) * 0.28);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let inset = editor_theme::space::XS.min(rect.width() * 0.04);
    let line = [
        egui::pos2(rect.left() + inset, rect.center().y),
        egui::pos2(rect.right() - inset, rect.center().y),
    ];
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 18),
    );
    ui.painter().line_segment(
        line,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    );
    for point in line {
        ui.painter()
            .circle_filled(point, editor_theme::shape::FOCUS_STROKE * 1.25, color);
    }
}

fn paint_reorder_origin(ui: &egui::Ui, rect: egui::Rect, active: bool, color: egui::Color32) {
    if !active {
        return;
    }
    ui.painter().rect_filled(
        rect.shrink(editor_theme::shape::STROKE),
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_black_alpha(104),
    );
    ui.painter().rect_stroke(
        rect.shrink(editor_theme::shape::STROKE),
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            color.gamma_multiply(0.72),
        ),
        egui::StrokeKind::Inside,
    );
}

fn paint_modulator_drag_ghost(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    drag: ModulatorReorder,
) {
    let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) else {
        return;
    };
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
    let scale = 0.62_f32
        .min(screen.width() * 0.56 / drag.card_size.x.max(1.0))
        .min(screen.height() * 0.42 / drag.card_size.y.max(1.0));
    let size = drag.card_size * scale;
    let offset = egui::vec2(drag.header_height * scale, drag.header_height * scale);
    let mut rect = egui::Rect::from_min_size(pointer + offset, size);
    if rect.right() > screen.right() {
        rect = egui::Rect::from_min_size(pointer - egui::vec2(size.x + offset.x, -offset.y), size);
    }
    rect = rect.translate(egui::vec2(
        (screen.left() - rect.left()).max(0.0) - (rect.right() - screen.right()).max(0.0),
        (screen.top() - rect.top()).max(0.0) - (rect.bottom() - screen.bottom()).max(0.0),
    ));

    let palette = editor_theme::semantic();
    let color = source_color(drag.source_slot);
    let envelope = source_is_envelope(state, drag.source_slot);
    let label = format!(
        "{} {}",
        if envelope { "ENV" } else { "LFO" },
        drag.source_slot + 1
    );
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new(("modulator-reorder-ghost", drag.source_slot)),
    ));
    painter.rect_filled(rect, editor_theme::shape::CONTROL_RADIUS, palette.surface);
    painter.rect_stroke(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
        egui::StrokeKind::Inside,
    );

    let header_height = (drag.header_height * scale).min(rect.height());
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
    painter.rect_filled(
        header,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 26),
    );
    painter.line_segment(
        [header.left_bottom(), header.right_bottom()],
        egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.56)),
    );
    let text_inset = header.height() * 0.46;
    painter.text(
        egui::pos2(header.left() + text_inset, header.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        palette.text,
    );
    painter.text(
        egui::pos2(header.right() - text_inset, header.center().y),
        egui::Align2::RIGHT_CENTER,
        "MOVE",
        editor_theme::font::caption(),
        color,
    );
    if drag.collapsed || rect.height() <= header.height() {
        return;
    }

    let controls_height = header.height();
    let graph = egui::Rect::from_min_max(
        egui::pos2(rect.left(), header.bottom()),
        egui::pos2(rect.right(), rect.bottom() - controls_height),
    )
    .shrink2(egui::vec2(header.height() * 0.28, header.height() * 0.20));
    if !graph.is_positive() {
        return;
    }
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - controls_height),
            egui::pos2(rect.right(), rect.bottom() - controls_height),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
    );
    let points = if envelope {
        envelope_ghost_points(state, drag.source_slot, graph)
    } else {
        lfo_ghost_points(state, drag.source_slot, graph)
    };
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    ));
    for division in 1..4 {
        let x = egui::lerp(rect.left()..=rect.right(), division as f32 / 4.0);
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - controls_height),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                palette.grid.gamma_multiply(0.62),
            ),
        );
    }
}

fn envelope_ghost_points(
    state: &PluginContext<KurvParams>,
    index: usize,
    plot: egui::Rect,
) -> Vec<egui::Pos2> {
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let curves = envelope_curve_values(state.params(), index);
    let weight = |seconds: f32| (seconds.max(0.0) + f32::EPSILON).sqrt();
    let weights = [weight(attack), weight(decay), 0.32, weight(release)];
    let total: f32 = weights.iter().sum();
    let attack_x = plot.left() + plot.width() * weights[0] / total;
    let decay_x = attack_x + plot.width() * weights[1] / total;
    let sustain_x = decay_x + plot.width() * weights[2] / total;
    let sustain_y = egui::lerp(plot.bottom()..=plot.top(), sustain.clamp(0.0, 1.0));
    let points = [
        plot.left_bottom(),
        egui::pos2(attack_x, plot.top()),
        egui::pos2(decay_x, sustain_y),
        egui::pos2(sustain_x, sustain_y),
        plot.right_bottom(),
    ];
    envelope_path(&points, curves)
}

fn lfo_ghost_points(
    state: &PluginContext<KurvParams>,
    index: usize,
    plot: egui::Rect,
) -> Vec<egui::Pos2> {
    let curve = if index < LEGACY_MODULATION_SOURCES {
        Some(lfo_curve(state.params(), index))
    } else {
        state.params().modulator_rack.curve(index)
    };
    let compiled = curve
        .and_then(WaveCurveState::try_curve_rt)
        .unwrap_or_default();
    let bipolar = if index < LEGACY_MODULATION_SOURCES {
        state.get_param(lfo_params(index).bipolar) >= 0.5
    } else {
        state.params().modulator_rack.config(index).bipolar
    };
    let segments = (plot.width() / editor_theme::space::SM).round().max(4.0) as usize;
    (0..=segments)
        .map(|point| {
            let phase = point as f32 / segments as f32;
            let value = compiled.eval(phase);
            let y = if bipolar {
                plot.center().y - value * plot.height() * 0.46
            } else {
                egui::lerp(plot.bottom()..=plot.top(), value.mul_add(0.5, 0.5))
            };
            egui::pos2(egui::lerp(plot.left()..=plot.right(), phase), y)
        })
        .collect()
}

fn nearest_modulator_insertion(
    ui: &egui::Ui,
    visible_sources: &[usize],
    collapsed_modulators: u64,
    reserved: Option<usize>,
) -> Option<usize> {
    let pointer = ui.input(|input| {
        (input.modifiers.alt && !crate::editor_modulation::source_drag_active(ui))
            .then_some(input.pointer.latest_pos())
            .flatten()
    })?;
    let rack = ui.available_rect_before_wrap();
    if !rack.contains(pointer) {
        return None;
    }
    let threshold = editor_theme::title_height(ui) * 0.72;
    let row_height = editor_theme::title_height(ui);
    let gap = ui.spacing().item_spacing.y;
    let mut edge = ui.cursor().top();
    let mut nearest = None;
    for (insertion, index) in visible_sources.iter().enumerate() {
        if reserved == Some(insertion)
            && (edge - row_height * 0.16..=edge + row_height).contains(&pointer.y)
        {
            return Some(insertion);
        }
        let distance = (pointer.y - edge).abs();
        if distance <= threshold
            && nearest.is_none_or(|(_, nearest_distance)| distance < nearest_distance)
        {
            nearest = Some((insertion, distance));
        }
        if reserved == Some(insertion) {
            edge += row_height;
        }
        edge += if collapsed_modulators & (1_u64 << *index) != 0 {
            collapsed_module_height(ui)
        } else {
            expanded_module_height(ui)
        } + gap;
    }
    nearest.map(|(insertion, _)| insertion)
}

fn place_source_at_active_insertion(
    state: &PluginContext<KurvParams>,
    source_slot: usize,
    active: u64,
    presentation_insertion: usize,
) {
    let order = state.params().modulator_rack.presentation_order();
    let active_slots: Vec<_> = order
        .iter()
        .copied()
        .filter(|slot| active & (1_u64 << *slot) != 0)
        .collect();
    let full_insertion = if let Some(target) = active_slots.get(presentation_insertion) {
        order
            .iter()
            .position(|slot| slot == target)
            .unwrap_or_default()
    } else {
        active_slots
            .last()
            .and_then(|target| order.iter().position(|slot| slot == target))
            .map_or(0, |position| position + 1)
    };
    state
        .params()
        .modulator_rack
        .move_source_slot(source_slot, full_insertion);
}

fn draw_add_modulator(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    view: &mut ModulationUi,
    active: &mut u64,
    width: f32,
    presentation_insertion: usize,
    insertion: bool,
) {
    let palette = editor_theme::semantic();
    let menu = AddMenu {
        presentation_insertion,
        insertion,
    };
    let menu_id = ui
        .id()
        .with(("add-modulator", menu.presentation_insertion, menu.insertion));
    let mut open = view.add_menu == Some(menu);
    let can_add = (0..MAX_MODULATION_SOURCES).any(|index| *active & (1_u64 << index) == 0);
    if !can_add {
        open = false;
    }
    let (id, rect) = ui.allocate_space(egui::vec2(width, editor_theme::title_height(ui)));
    if !rack_item_visible(ui, rect) && !open && !insertion {
        return;
    }
    let response = ui.interact(
        rect,
        id,
        if can_add {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let response = if can_add {
        response
    } else {
        response.on_hover_text("Modulator limit reached; remove a source to add another")
    };
    if insertion {
        ui.painter().rect_filled(
            rect,
            1.0,
            egui::Color32::from_rgba_unmultiplied(
                palette.primary.r(),
                palette.primary.g(),
                palette.primary.b(),
                22,
            ),
        );
    } else if can_add && (response.hovered() || open) {
        ui.painter().rect_filled(rect, 1.0, palette.control);
    }
    let stroke_color = if insertion || (can_add && response.hovered()) {
        palette.primary
    } else if can_add {
        palette.grid
    } else {
        palette.grid.gamma_multiply(0.48)
    };
    ui.painter().rect_stroke(
        rect,
        1.0,
        egui::Stroke::new(
            if open {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            stroke_color,
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.left_center() + egui::vec2(rect.height() * 0.5, 0.0),
        egui::Align2::LEFT_CENTER,
        if can_add {
            "+ ADD MODULATOR".to_owned()
        } else {
            format!("{MAX_MODULATION_SOURCES} MODULATORS · LIMIT")
        },
        editor_theme::font::label(),
        if insertion {
            palette.primary
        } else if can_add && (response.hovered() || open) {
            palette.text
        } else if can_add {
            palette.text_muted
        } else {
            palette.disabled_text
        },
    );
    if can_add && response.clicked() {
        open = !open;
        view.add_menu = open.then_some(menu);
    }
    if open {
        let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
        let popup_width = (width * 0.42).min(screen.width());
        let popup_height = editor_theme::title_height(ui) * 2.0
            + editor_theme::space::XS * 2.0
            + editor_theme::font::caption().size
            + editor_theme::compact_gap(ui) * 2.0;
        let popup_x = rect.left().clamp(
            screen.left(),
            (screen.right() - popup_width).max(screen.left()),
        );
        let popup_y = if rect.bottom() + popup_height <= screen.bottom() {
            rect.bottom()
        } else {
            (rect.top() - popup_height).max(screen.top())
        };
        let popup = egui::Area::new(menu_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(popup_x, popup_y))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0_f32, palette.grid))
                    .inner_margin(egui::Margin::same(editor_theme::space::XS as i8))
                    .show(ui, |ui| {
                        ui.set_min_width(popup_width);
                        let free = (0..MAX_MODULATION_SOURCES)
                            .find(|index| *active & (1_u64 << index) == 0);
                        let lfo_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                        });
                        if ui
                            .add_enabled(
                                free.is_some(),
                                egui::Button::new("1   LFO").min_size(egui::vec2(
                                    popup_width,
                                    editor_theme::title_height(ui),
                                )),
                            )
                            .clicked()
                            || (free.is_some() && lfo_key)
                        {
                            let index = free.expect("enabled only when an LFO slot is free");
                            place_source_at_active_insertion(
                                state,
                                index,
                                *active,
                                presentation_insertion,
                            );
                            *active |= 1_u64 << index;
                            set_source_active(state, index, true, SourceKind::Lfo);
                            view.selected = index;
                            open = false;
                            view.add_menu = None;
                        }
                        let free = (0..MAX_MODULATION_SOURCES)
                            .find(|index| *active & (1_u64 << index) == 0);
                        let envelope_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                        });
                        if ui
                            .add_enabled(
                                free.is_some(),
                                egui::Button::new("2   ENVELOPE").min_size(egui::vec2(
                                    popup_width,
                                    editor_theme::title_height(ui),
                                )),
                            )
                            .clicked()
                            || (free.is_some() && envelope_key)
                        {
                            let index = free.expect("enabled only when a source slot is free");
                            place_source_at_active_insertion(
                                state,
                                index,
                                *active,
                                presentation_insertion,
                            );
                            *active |= 1_u64 << index;
                            set_source_active(state, index, true, SourceKind::Envelope);
                            view.selected = index;
                            open = false;
                            view.add_menu = None;
                        }
                        ui.label(
                            egui::RichText::new("KEYS 1 / 2")
                                .font(editor_theme::font::caption())
                                .color(palette.text_muted),
                        );
                    });
            });
        if ui.input(|input| {
            input.pointer.primary_clicked()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    !response.rect.contains(pointer) && !popup.response.rect.contains(pointer)
                })
        }) {
            open = false;
            view.add_menu = None;
        }
    }
    if open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
        view.add_menu = None;
    }
    if !open && view.add_menu == Some(menu) {
        view.add_menu = None;
    }
}
