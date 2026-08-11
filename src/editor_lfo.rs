use truce::params::{FloatParamReadF32, Params};
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{metric_enum_readout, metric_param_readout, paint_metric_readout};
use crate::editor_modulation::{clear_source, source_color, source_handle, used_source_mask};
use crate::modulators::lfo::envelope::shaped_progress as envelope_shaped_progress;
use crate::modulators::state::{LEGACY_MODULATION_SOURCES, MAX_MODULATION_SOURCES, SourceKind};
use crate::wave_curve::{
    WaveCurveData, WaveCurveRt, WaveCurveState, insert_knot, move_knot, remove_knot,
    set_segment_curve,
};
use crate::{KurvParams, P, editor_theme, editor_widgets};

const MODES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
const RATE_MODES: [&str; 4] = ["Hz", "ms", "BEAT", "KEY"];
const ENVELOPE_CURVE_SEGMENTS: usize = 12;
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
    draft: Option<WaveCurveData>,
    snap_phase: Option<f32>,
    snap_value: Option<f32>,
    last_meter: Option<f32>,
    meter_motion_frames: u8,
}

#[derive(Clone, Copy)]
struct LfoParams {
    rate: P,
    rate_mode: P,
    mode: P,
    phase: P,
    sync: P,
    bipolar: P,
}

#[derive(Clone, Copy)]
struct EnvelopeParams {
    attack: P,
    decay: P,
    sustain: P,
    release: P,
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
                                    )
                                })
                                .flatten()
                        })
                })
                .flatten();
            let reorder_insertion = view.reorder.map(|drag| drag.presentation_insertion);
            let keep_rack_interactions_alive = view.reorder.is_some()
                || visible_insertion.is_some()
                || view.add_menu.is_some()
                || crate::editor_modulation::source_drag_active(ui)
                || ui.ctx().dragged_id().is_some()
                || ui.ctx().any_popup_open();
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
    let card_hovered = ui.rect_contains_pointer(rect);
    if card_hovered && ui.input(|input| input.pointer.primary_clicked()) {
        view.selected = index;
        selected = true;
    }
    ui.painter().rect_filled(rect, 2.0, palette.well);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(
            if selected {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if selected {
                color.gamma_multiply(0.72)
            } else if card_hovered {
                palette.grid
            } else {
                palette.grid.gamma_multiply(0.28)
            },
        ),
        egui::StrokeKind::Inside,
    );

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
    let action_size = header.height() * 0.84;
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
    // Keep the modulation jack visually small while giving it a forgiving
    // drag target. The title and reorder grip remain separate interactions.
    let source_width =
        (action_size + editor_theme::space::XS).min((drag_rect.width() - grip_width).max(0.0));
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
    let source_label = format!("{} {}", if envelope { "ENV" } else { "LFO" }, index + 1);
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
    if source_response.drag_started() || source_response.clicked() {
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
            false,
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
        .on_hover_text("Drag this modulation jack onto a highlighted parameter");
    if collapse.clicked() || header_response.double_clicked() {
        collapsed = !collapsed;
        set_modulator_collapsed(state, index, collapsed);
        editor_theme::request_display_repaint(ui);
    }
    if remove.clicked() {
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
    if collapse.hovered() || collapse.is_pointer_button_down_on() {
        let visuals = editor_theme::control_visuals(
            true,
            collapse.hovered(),
            collapse.is_pointer_button_down_on(),
            false,
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
        if collapse.hovered() || collapse.is_pointer_button_down_on() {
            color
        } else {
            palette.text_muted
        },
        egui::Stroke::NONE,
    ));
    ui.painter().text(
        egui::pos2(
            title_rect.left() + editor_theme::space::XXS,
            header.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        &source_label,
        editor_theme::font::label(),
        if source_active {
            palette.text
        } else if selected || header_response.hovered() {
            color
        } else {
            color.gamma_multiply(0.82)
        },
    );
    if title_rect.width() > header.height() * 5.0 {
        let text = if source_active {
            if drag_rect.width() > header.height() * 8.0 {
                "DROP ON CONTROL".to_owned()
            } else {
                "DRAG".to_owned()
            }
        } else if header_response.hovered() {
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
        let text_font = if source_active || header_response.hovered() || collapsed {
            editor_theme::font::caption()
        } else {
            editor_theme::font::value()
        };
        let label_width = ui
            .painter()
            .layout_no_wrap(source_label.clone(), editor_theme::font::label(), color)
            .size()
            .x;
        let text_width = ui
            .painter()
            .layout_no_wrap(text.clone(), text_font.clone(), palette.text_muted)
            .size()
            .x;
        if label_width + text_width + editor_theme::space::MD < title_rect.width() {
            ui.painter().text(
                title_rect.right_center() - egui::vec2(editor_theme::space::XS, 0.0),
                egui::Align2::RIGHT_CENTER,
                text,
                text_font,
                if source_active {
                    palette.text
                } else if header_response.hovered() {
                    color
                } else {
                    palette.text_muted
                },
            );
        }
    }
    if remove.hovered() || remove.is_pointer_button_down_on() {
        let visuals = editor_theme::control_visuals(
            true,
            remove.hovered(),
            remove.is_pointer_button_down_on(),
            false,
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
        if remove.hovered() || remove.is_pointer_button_down_on() {
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
    editor_theme::title_height(ui) * 3.65
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
    let clip = ui.clip_rect();
    let threshold = editor_theme::title_height(ui) * 0.72;
    let gap = ui.spacing().item_spacing.y;
    let mut edge = ui.cursor().top();
    visible_sources
        .iter()
        .enumerate()
        .filter_map(|(insertion, index)| {
            let distance = (pointer.y - edge).abs();
            let candidate = (clip.top()..=clip.bottom())
                .contains(&edge)
                .then_some((insertion, distance));
            edge += if collapsed_modulators & (1_u64 << *index) != 0 {
                collapsed_module_height(ui)
            } else {
                expanded_module_height(ui)
            } + gap;
            candidate
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= threshold)
        .map(|(insertion, _)| insertion)
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

fn draw_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    if index >= LEGACY_MODULATION_SOURCES {
        draw_dynamic_lfo_controls(ui, state, index, width, height);
        return;
    }
    let params = lfo_params(index);
    let cell_width = width / 5.0;
    let color = source_color(index);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        if rate_mode(state, params.rate_mode) == 2 {
            metric_param_readout(
                ui,
                state,
                params.sync,
                "RATE",
                &state.format_param(params.sync),
                cell_width,
                height,
                color,
            );
        } else {
            let text = rate_text(state, index, params.rate_mode);
            metric_param_readout(
                ui,
                state,
                params.rate,
                "RATE",
                &text,
                cell_width,
                height,
                color,
            );
        }
        metric_enum_readout(
            ui,
            state,
            params.rate_mode,
            "UNIT",
            &RATE_MODES,
            cell_width,
            height,
            color,
        );
        metric_enum_readout(
            ui,
            state,
            params.mode,
            "MODE",
            &MODES,
            cell_width,
            height,
            color,
        );
        metric_param_readout(
            ui,
            state,
            params.phase,
            "PHASE",
            &state.format_param(params.phase),
            cell_width,
            height,
            color,
        );
        metric_enum_readout(
            ui,
            state,
            params.bipolar,
            "POLAR",
            &["UNI", "BI"],
            cell_width,
            height,
            color,
        );
    });
}

fn draw_envelope_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    if index >= LEGACY_MODULATION_SOURCES {
        draw_dynamic_envelope_controls(ui, state, index, width, height);
        return;
    }
    let params = envelope_params(index);
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let values = [
        format_envelope_time(attack),
        format_envelope_time(decay),
        format!("{:.0}%", sustain * 100.0),
        format_envelope_time(release),
    ];
    let cell_width = width / 4.0;
    let color = source_color(index);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        for ((param, label), value) in [
            (params.attack, "ATTACK"),
            (params.decay, "DECAY"),
            (params.sustain, "SUSTAIN"),
            (params.release, "RELEASE"),
        ]
        .into_iter()
        .zip(values)
        {
            metric_param_readout(ui, state, param, label, &value, cell_width, height, color);
        }
    });
}

fn draw_dynamic_lfo_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;
    let color = source_color(index);
    ui.set_min_size(egui::vec2(width, height));
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    ui.columns(5, |columns| {
        changed |= dynamic_value(
            &mut columns[0],
            "RATE",
            &mut config.rate_hz,
            0.01..=20_000.0,
            1.0,
            color,
            format_dynamic_rate,
        );
        changed |= dynamic_choice(
            &mut columns[1],
            "UNIT",
            &mut config.rate_mode,
            &RATE_MODES,
            color,
        );
        changed |= dynamic_choice(&mut columns[2], "MODE", &mut config.mode, &MODES, color);
        changed |= dynamic_value(
            &mut columns[3],
            "PHASE",
            &mut config.phase_offset,
            0.0..=1.0,
            0.0,
            color,
            format_dynamic_phase,
        );
        let mut polar = u8::from(config.bipolar);
        changed |= dynamic_choice(&mut columns[4], "POLAR", &mut polar, &["UNI", "BI"], color);
        config.bipolar = polar != 0;
    });
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

fn draw_dynamic_envelope_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;
    let color = source_color(index);
    ui.set_min_size(egui::vec2(width, height));
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    ui.columns(4, |columns| {
        changed |= dynamic_value(
            &mut columns[0],
            "ATTACK",
            &mut config.attack,
            0.0..=8.0,
            0.01,
            color,
            format_envelope_time,
        );
        changed |= dynamic_value(
            &mut columns[1],
            "DECAY",
            &mut config.decay,
            0.0..=8.0,
            0.1,
            color,
            format_envelope_time,
        );
        changed |= dynamic_value(
            &mut columns[2],
            "SUSTAIN",
            &mut config.sustain,
            0.0..=1.0,
            0.8,
            color,
            format_dynamic_percent,
        );
        changed |= dynamic_value(
            &mut columns[3],
            "RELEASE",
            &mut config.release,
            0.0..=12.0,
            0.2,
            color,
            format_envelope_time,
        );
    });
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

fn dynamic_value(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    color: egui::Color32,
    format: fn(f32) -> String,
) -> bool {
    let size = egui::vec2(
        ui.available_width(),
        ui.available_height().max(editor_theme::title_height(ui)),
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(format!(
            "Drag {}. Hold Shift for fine control; double-click to reset.",
            label.to_lowercase()
        ));
    let before = *value;
    if response.dragged() {
        let delta = -ui.input(|input| input.pointer.delta().y)
            * if ui.input(|input| input.modifiers.shift) {
                0.1
            } else {
                1.0
            };
        let start = *range.start();
        let end = *range.end();
        if start > 0.0 && end / start >= 100.0 {
            *value = (*value * (delta * 0.02).exp2()).clamp(start, end);
        } else {
            *value = (*value + delta * (end - start) / 150.0).clamp(start, end);
        }
    } else if response.double_clicked() {
        *value = default.clamp(*range.start(), *range.end());
    }
    paint_metric_readout(
        ui,
        rect,
        label,
        &format(*value),
        color,
        response.is_pointer_button_down_on() || response.dragged(),
    );
    value.to_bits() != before.to_bits()
}

fn dynamic_choice(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u8,
    values: &[&str],
    color: egui::Color32,
) -> bool {
    debug_assert!(!values.is_empty());
    let size = egui::vec2(
        ui.available_width(),
        ui.available_height().max(editor_theme::title_height(ui)),
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("{label}: click to cycle"));
    let current = usize::from(*value).min(values.len() - 1);
    let changed = if response.clicked() {
        *value = ((current + 1) % values.len()) as u8;
        true
    } else {
        false
    };
    let displayed = usize::from(*value).min(values.len() - 1);
    paint_metric_readout(
        ui,
        rect,
        label,
        values[displayed],
        color,
        response.is_pointer_button_down_on(),
    );
    changed
}

fn format_dynamic_rate(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.2} kHz", hz / 1_000.0)
    } else if hz >= 100.0 {
        format!("{hz:.0} Hz")
    } else {
        format!("{hz:.2} Hz")
    }
}

fn format_dynamic_percent(value: f32) -> String {
    format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0)
}

fn format_dynamic_phase(value: f32) -> String {
    format!("{:.0}°", value.rem_euclid(1.0) * 360.0)
}

fn format_envelope_time(seconds: f32) -> String {
    if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1_000.0)
    } else {
        format!("{seconds:.2} s")
    }
}

fn collapsed_source_summary(
    state: &PluginContext<KurvParams>,
    index: usize,
    envelope: bool,
) -> String {
    if envelope {
        let [attack, decay, sustain, release] = envelope_values(state.params(), index);
        return format!(
            "A {} · D {} · S {:.0}% · R {}",
            format_envelope_time(attack),
            format_envelope_time(decay),
            sustain.clamp(0.0, 1.0) * 100.0,
            format_envelope_time(release),
        );
    }
    if index >= LEGACY_MODULATION_SOURCES {
        let config = state.params().modulator_rack.config(index);
        return format!(
            "{} · {}",
            format_dynamic_rate(config.rate_hz),
            MODES[usize::from(config.mode).min(MODES.len() - 1)],
        );
    }
    let params = lfo_params(index);
    let rate = if rate_mode(state, params.rate_mode) == 2 {
        state.format_param(params.sync)
    } else {
        rate_text(state, index, params.rate_mode)
    };
    let mode = (state.get_param(params.mode).clamp(0.0, 1.0) * 3.0).round() as usize;
    format!("{rate} · {}", MODES[mode.min(MODES.len() - 1)])
}

fn draw_envelope_curve(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let plot = rect.shrink(editor_theme::graph_inset(ui));
    let painter = ui.painter_at(rect);
    editor_widgets::graph_frame(&painter, rect);
    let editor_id = response.id.with("envelope-editor");
    let mut editor = ui
        .data(|store| store.get_temp::<EnvelopeEditorUi>(editor_id))
        .unwrap_or_default();
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let curves = envelope_curve_values(state.params(), index);
    let duration_weight = |seconds: f32| (seconds.max(0.0) + 0.002).sqrt();
    let weights = [
        duration_weight(attack),
        duration_weight(decay),
        0.32,
        duration_weight(release),
    ];
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
    let mut handles = vec![
        (EnvelopeDrag::Attack, points[1]),
        (EnvelopeDrag::DecaySustain, points[2]),
        (
            EnvelopeDrag::Sustain,
            points[2] + (points[3] - points[2]) * 0.5,
        ),
        (EnvelopeDrag::Release, points[3]),
    ];
    if index >= LEGACY_MODULATION_SOURCES {
        handles.extend([
            (
                EnvelopeDrag::AttackCurve,
                envelope_curve_handle(points[0], points[1], curves[0]),
            ),
            (
                EnvelopeDrag::DecayCurve,
                envelope_curve_handle(points[1], points[2], curves[1]),
            ),
            (
                EnvelopeDrag::ReleaseCurve,
                envelope_curve_handle(points[3], points[4], curves[2]),
            ),
        ]);
    }
    let handle_radius = (plot.height() * 0.035).clamp(3.5, 6.0);
    let pointer = response.interact_pointer_pos();
    let hovered_handle = pointer.and_then(|pointer| {
        handles
            .iter()
            .map(|(stage, position)| (*stage, position.distance_sq(pointer)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= (handle_radius * 2.6).powi(2))
            .map(|(stage, _)| stage)
    });
    let hovered = hovered_handle.or_else(|| {
        pointer.and_then(|pointer| {
            [
                EnvelopeDrag::Attack,
                EnvelopeDrag::DecaySustain,
                EnvelopeDrag::Sustain,
                EnvelopeDrag::Release,
            ]
            .into_iter()
            .map(|stage| {
                let (start, end) = envelope_segment(&points, stage);
                (
                    stage,
                    distance_to_envelope_stage_sq(
                        pointer,
                        start,
                        end,
                        envelope_curve_for_stage(curves, stage),
                    ),
                )
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= (handle_radius * 2.8).powi(2))
            .map(|(stage, _)| stage)
        })
    });

    let mut reset_stage = None;
    let mut reset_all = false;
    response.context_menu(|ui| {
        if let Some(stage) = hovered
            && ui
                .button(format!("RESET {}", envelope_stage_label(stage)))
                .clicked()
        {
            reset_stage = Some(stage);
            ui.close();
        }
        if ui.button("RESET ENVELOPE").clicked() {
            reset_all = true;
            ui.close();
        }
    });
    if reset_all {
        reset_envelope(state, index, None);
        editor.drag = None;
        editor.selected = None;
    } else if let Some(stage) = reset_stage {
        reset_envelope(state, index, Some(stage));
        editor.drag = None;
        editor.selected = Some(stage);
    } else if response.double_clicked() {
        reset_envelope(state, index, hovered);
        editor.drag = None;
        editor.selected = hovered;
    } else if response.drag_started()
        && let Some(stage) = hovered
    {
        begin_envelope_edit(state, index, stage);
        editor.selected = Some(stage);
        editor.drag = Some(stage);
    }
    if response.clicked() {
        editor.selected = hovered;
    }
    if response.dragged()
        && let Some(stage) = editor.drag
    {
        let delta = ui.input(|input| input.pointer.delta());
        let precision = if ui.input(|input| input.modifiers.shift) {
            0.18
        } else {
            1.0
        };
        let x = delta.x / plot.width().max(1.0) * precision;
        let y = delta.y / plot.height().max(1.0) * precision;
        match stage {
            EnvelopeDrag::Attack => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::Attack,
                    envelope_normalized(state, index, EnvelopeDrag::Attack) + x,
                );
            }
            EnvelopeDrag::AttackCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::AttackCurve,
                    envelope_normalized(state, index, EnvelopeDrag::AttackCurve) - y,
                );
            }
            EnvelopeDrag::DecaySustain => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::DecaySustain,
                    envelope_normalized(state, index, EnvelopeDrag::DecaySustain) + x,
                );
                set_envelope_sustain_normalized(
                    state,
                    index,
                    envelope_sustain_normalized(state, index) - y,
                );
            }
            EnvelopeDrag::DecayCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::DecayCurve,
                    envelope_normalized(state, index, EnvelopeDrag::DecayCurve) + y,
                );
            }
            EnvelopeDrag::Sustain => {
                set_envelope_sustain_normalized(
                    state,
                    index,
                    envelope_sustain_normalized(state, index) - y,
                );
            }
            EnvelopeDrag::Release => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::Release,
                    envelope_normalized(state, index, EnvelopeDrag::Release) - x,
                );
            }
            EnvelopeDrag::ReleaseCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::ReleaseCurve,
                    envelope_normalized(state, index, EnvelopeDrag::ReleaseCurve) + y,
                );
            }
        }
        editor_theme::request_display_repaint(ui);
    }
    if response.drag_stopped() {
        if let Some(stage) = editor.drag.take() {
            end_envelope_edit(state, index, stage);
        }
    }

    let color = source_color(index);
    let curve_points = envelope_path(&points, curves);
    editor_widgets::gradient_area_to_baseline(&painter, &curve_points, plot.bottom(), color, 64);
    if response.hovered() || editor.drag.is_some() || editor.selected.is_some() {
        painter.rect_stroke(
            rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                color.gamma_multiply(if editor.drag.is_some() { 0.7 } else { 0.42 }),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(stage) = editor.drag.or(hovered).or(editor.selected) {
        let (start, end) = envelope_segment(&points, stage);
        painter.add(egui::Shape::line(
            envelope_stage_path(start, end, envelope_curve_for_stage(curves, stage)),
            egui::Stroke::new(
                (plot.height() * 0.05).clamp(3.0, 5.0),
                color.gamma_multiply(if editor.drag == Some(stage) {
                    0.28
                } else {
                    0.14
                }),
            ),
        ));
    }
    painter.add(egui::Shape::line(
        curve_points,
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    ));
    for (stage, position) in handles {
        let active = editor.drag == Some(stage);
        let hot = active || hovered == Some(stage);
        let selected = editor.selected == Some(stage);
        let curve_handle = matches!(
            stage,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve
        );
        let handle_radius = handle_radius * if curve_handle { 0.78 } else { 1.0 };
        if hot {
            painter.circle_filled(position, handle_radius * 1.55, color.gamma_multiply(0.18));
        }
        if selected {
            painter.circle_stroke(
                position,
                handle_radius * 1.25,
                egui::Stroke::new(
                    (handle_radius * 0.22).clamp(0.9, 1.35),
                    color.gamma_multiply(0.58),
                ),
            );
        }
        painter.circle_filled(
            position,
            if active {
                handle_radius * 1.08
            } else if hot || selected {
                handle_radius * 0.86
            } else {
                handle_radius * 0.68
            },
            if active {
                editor_theme::semantic().text
            } else {
                editor_theme::semantic().well
            },
        );
        painter.circle_stroke(
            position,
            if active {
                handle_radius * 1.08
            } else if hot || selected {
                handle_radius * 0.86
            } else {
                handle_radius * 0.68
            },
            egui::Stroke::new(
                (handle_radius * 0.2).clamp(0.8, 1.2),
                if active {
                    editor_theme::semantic().text
                } else {
                    color
                },
            ),
        );
        if hot || selected {
            let label = match stage {
                EnvelopeDrag::Attack => "A",
                EnvelopeDrag::AttackCurve => "A CURVE",
                EnvelopeDrag::DecaySustain => "D/S",
                EnvelopeDrag::DecayCurve => "D CURVE",
                EnvelopeDrag::Sustain => "S",
                EnvelopeDrag::Release => "R",
                EnvelopeDrag::ReleaseCurve => "R CURVE",
            };
            let label_y = if position.y - plot.top()
                < editor_theme::font::CAPTION_SIZE + handle_radius * 2.0
            {
                position.y + handle_radius * 1.4
            } else {
                position.y - handle_radius * 1.4
            };
            painter.text(
                egui::pos2(position.x, label_y),
                if label_y > position.y {
                    egui::Align2::CENTER_TOP
                } else {
                    egui::Align2::CENTER_BOTTOM
                },
                label,
                editor_theme::font::caption(),
                if active {
                    editor_theme::semantic().text
                } else {
                    color
                },
            );
        }
    }
    if response.hovered() {
        let hint = match editor.drag.or(hovered).or(editor.selected) {
            Some(EnvelopeDrag::Attack) => "ATTACK · DRAG X",
            Some(EnvelopeDrag::AttackCurve) => "ATTACK CURVE · DRAG Y",
            Some(EnvelopeDrag::DecaySustain) => "DECAY / SUSTAIN · DRAG X/Y",
            Some(EnvelopeDrag::DecayCurve) => "DECAY CURVE · DRAG Y",
            Some(EnvelopeDrag::Sustain) => "SUSTAIN · DRAG Y",
            Some(EnvelopeDrag::Release) => "RELEASE · DRAG X",
            Some(EnvelopeDrag::ReleaseCurve) => "RELEASE CURVE · DRAG Y",
            None => "DRAG A STAGE",
        };
        painter.text(
            plot.right_top() + egui::vec2(-editor_theme::space::XS, editor_theme::space::XXS),
            egui::Align2::RIGHT_TOP,
            hint,
            editor_theme::font::caption(),
            color.gamma_multiply(0.78),
        );
        ui.output_mut(|output| {
            output.cursor_icon = match editor.drag.or(hovered) {
                Some(_) if editor.drag.is_some() => egui::CursorIcon::Grabbing,
                Some(EnvelopeDrag::Attack | EnvelopeDrag::Release) => {
                    egui::CursorIcon::ResizeHorizontal
                }
                Some(EnvelopeDrag::DecaySustain) => egui::CursorIcon::ResizeNwSe,
                Some(
                    EnvelopeDrag::AttackCurve
                    | EnvelopeDrag::DecayCurve
                    | EnvelopeDrag::Sustain
                    | EnvelopeDrag::ReleaseCurve,
                ) => egui::CursorIcon::ResizeVertical,
                None => egui::CursorIcon::Default,
            };
        });
    }
    response.clone().on_hover_text(
        "Drag ADSR points or segments; drag midpoint handles vertically to bend stages. Hold Shift for fine adjustment. Double-click a stage or bend to reset it; right-click to reset the envelope.",
    );
    let value = source_value_meter(state, index).clamp(0.0, 1.0);
    painter.circle_filled(
        egui::pos2(plot.right(), egui::lerp(plot.bottom()..=plot.top(), value)),
        (plot.height() * 0.025).max(2.0),
        color,
    );
    let meter_moving = meter_is_moving(
        &mut editor.last_meter,
        &mut editor.meter_motion_frames,
        value,
        false,
    );
    request_graph_repaint(ui, meter_moving);
    ui.data_mut(|store| store.insert_temp(editor_id, editor));
}

fn envelope_stage_label(stage: EnvelopeDrag) -> &'static str {
    match stage {
        EnvelopeDrag::Attack => "ATTACK",
        EnvelopeDrag::AttackCurve => "ATTACK CURVE",
        EnvelopeDrag::DecaySustain => "DECAY + SUSTAIN",
        EnvelopeDrag::DecayCurve => "DECAY CURVE",
        EnvelopeDrag::Sustain => "SUSTAIN",
        EnvelopeDrag::Release => "RELEASE",
        EnvelopeDrag::ReleaseCurve => "RELEASE CURVE",
    }
}

fn envelope_segment(points: &[egui::Pos2; 5], stage: EnvelopeDrag) -> (egui::Pos2, egui::Pos2) {
    match stage {
        EnvelopeDrag::Attack | EnvelopeDrag::AttackCurve => (points[0], points[1]),
        EnvelopeDrag::DecaySustain | EnvelopeDrag::DecayCurve => (points[1], points[2]),
        EnvelopeDrag::Sustain => (points[2], points[3]),
        EnvelopeDrag::Release | EnvelopeDrag::ReleaseCurve => (points[3], points[4]),
    }
}

fn envelope_curve_handle(start: egui::Pos2, end: egui::Pos2, curve: f32) -> egui::Pos2 {
    envelope_stage_position(start, end, 0.5, curve)
}

fn envelope_stage_position(
    start: egui::Pos2,
    end: egui::Pos2,
    progress: f32,
    curve: f32,
) -> egui::Pos2 {
    let shaped = envelope_shaped_progress(progress, curve);
    egui::pos2(
        egui::lerp(start.x..=end.x, progress),
        egui::lerp(start.y..=end.y, shaped),
    )
}

fn envelope_path(points: &[egui::Pos2; 5], curves: [f32; 3]) -> Vec<egui::Pos2> {
    let mut path = Vec::with_capacity(ENVELOPE_CURVE_SEGMENTS * 3 + 2);
    append_envelope_stage(&mut path, points[0], points[1], curves[0], true);
    append_envelope_stage(&mut path, points[1], points[2], curves[1], false);
    path.push(points[3]);
    append_envelope_stage(&mut path, points[3], points[4], curves[2], false);
    path
}

fn envelope_stage_path(start: egui::Pos2, end: egui::Pos2, curve: f32) -> Vec<egui::Pos2> {
    let mut path = Vec::with_capacity(ENVELOPE_CURVE_SEGMENTS + 1);
    append_envelope_stage(&mut path, start, end, curve, true);
    path
}

fn append_envelope_stage(
    path: &mut Vec<egui::Pos2>,
    start: egui::Pos2,
    end: egui::Pos2,
    curve: f32,
    include_start: bool,
) {
    let first = if include_start { 0 } else { 1 };
    for step in first..=ENVELOPE_CURVE_SEGMENTS {
        let progress = step as f32 / ENVELOPE_CURVE_SEGMENTS as f32;
        path.push(envelope_stage_position(start, end, progress, curve));
    }
}

fn envelope_curve_for_stage(curves: [f32; 3], stage: EnvelopeDrag) -> f32 {
    match stage {
        EnvelopeDrag::Attack | EnvelopeDrag::AttackCurve => curves[0],
        EnvelopeDrag::DecaySustain | EnvelopeDrag::DecayCurve => curves[1],
        EnvelopeDrag::Sustain => 0.0,
        EnvelopeDrag::Release | EnvelopeDrag::ReleaseCurve => curves[2],
    }
}

fn distance_to_segment_sq(point: egui::Pos2, start: egui::Pos2, end: egui::Pos2) -> f32 {
    let segment = end - start;
    let length_sq = segment.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance_sq(start);
    }
    let position = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance_sq(start + segment * position)
}

fn distance_to_envelope_stage_sq(
    point: egui::Pos2,
    start: egui::Pos2,
    end: egui::Pos2,
    curve: f32,
) -> f32 {
    let mut nearest = f32::INFINITY;
    let mut previous = start;
    for step in 1..=ENVELOPE_CURVE_SEGMENTS {
        let progress = step as f32 / ENVELOPE_CURVE_SEGMENTS as f32;
        let current = envelope_stage_position(start, end, progress, curve);
        nearest = nearest.min(distance_to_segment_sq(point, previous, current));
        previous = current;
    }
    nearest
}

fn begin_envelope_edit(state: &PluginContext<KurvParams>, index: usize, stage: EnvelopeDrag) {
    if index >= LEGACY_MODULATION_SOURCES {
        return;
    }
    let params = envelope_params(index);
    match stage {
        EnvelopeDrag::Attack => state.begin_edit(params.attack),
        EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {}
        EnvelopeDrag::DecaySustain => {
            state.begin_edit(params.decay);
            state.begin_edit(params.sustain);
        }
        EnvelopeDrag::Sustain => state.begin_edit(params.sustain),
        EnvelopeDrag::Release => state.begin_edit(params.release),
    }
}

fn end_envelope_edit(state: &PluginContext<KurvParams>, index: usize, stage: EnvelopeDrag) {
    if index >= LEGACY_MODULATION_SOURCES {
        return;
    }
    let params = envelope_params(index);
    match stage {
        EnvelopeDrag::Attack => state.end_edit(params.attack),
        EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {}
        EnvelopeDrag::DecaySustain => {
            state.end_edit(params.decay);
            state.end_edit(params.sustain);
        }
        EnvelopeDrag::Sustain => state.end_edit(params.sustain),
        EnvelopeDrag::Release => state.end_edit(params.release),
    }
}

fn reset_envelope(state: &PluginContext<KurvParams>, index: usize, stage: Option<EnvelopeDrag>) {
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let targets: &[P] = match stage {
            Some(EnvelopeDrag::Attack) => &[params.attack],
            Some(
                EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve,
            ) => &[],
            Some(EnvelopeDrag::DecaySustain) => &[params.decay, params.sustain],
            Some(EnvelopeDrag::Sustain) => &[params.sustain],
            Some(EnvelopeDrag::Release) => &[params.release],
            None => &[params.attack, params.decay, params.sustain, params.release],
        };
        for &param in targets {
            let raw = u32::from(param);
            let Some(default) = state
                .params()
                .param_infos()
                .into_iter()
                .find(|info| info.id == raw)
                .map(|info| info.range.normalize(info.default_plain))
            else {
                continue;
            };
            state.begin_edit(param);
            state.set_param(param, default);
            state.end_edit(param);
        }
        return;
    }

    let defaults = crate::modulators::state::SourceConfig::default();
    let mut config = state.params().modulator_rack.config(index);
    match stage {
        Some(EnvelopeDrag::Attack) => config.attack = defaults.attack,
        Some(EnvelopeDrag::AttackCurve) => config.attack_curve = defaults.attack_curve,
        Some(EnvelopeDrag::DecaySustain) => {
            config.decay = defaults.decay;
            config.sustain = defaults.sustain;
        }
        Some(EnvelopeDrag::DecayCurve) => config.decay_curve = defaults.decay_curve,
        Some(EnvelopeDrag::Sustain) => config.sustain = defaults.sustain,
        Some(EnvelopeDrag::Release) => config.release = defaults.release,
        Some(EnvelopeDrag::ReleaseCurve) => config.release_curve = defaults.release_curve,
        None => {
            config.attack = defaults.attack;
            config.attack_curve = defaults.attack_curve;
            config.decay = defaults.decay;
            config.decay_curve = defaults.decay_curve;
            config.sustain = defaults.sustain;
            config.release = defaults.release;
            config.release_curve = defaults.release_curve;
        }
    }
    state.params().modulator_rack.set_config(index, config);
}

fn envelope_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
) -> f32 {
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let param = match stage {
            EnvelopeDrag::Attack => params.attack,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {
                return 0.5;
            }
            EnvelopeDrag::DecaySustain => params.decay,
            EnvelopeDrag::Sustain => params.sustain,
            EnvelopeDrag::Release => params.release,
        };
        return state.get_param(param);
    }
    let config = state.params().modulator_rack.config(index);
    match stage {
        EnvelopeDrag::Attack => config.attack / 8.0,
        EnvelopeDrag::AttackCurve => config.attack_curve.mul_add(0.5, 0.5),
        EnvelopeDrag::DecaySustain => config.decay / 8.0,
        EnvelopeDrag::DecayCurve => config.decay_curve.mul_add(0.5, 0.5),
        EnvelopeDrag::Sustain => config.sustain,
        EnvelopeDrag::Release => config.release / 12.0,
        EnvelopeDrag::ReleaseCurve => config.release_curve.mul_add(0.5, 0.5),
    }
}

fn envelope_sustain_normalized(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index < LEGACY_MODULATION_SOURCES {
        state.get_param(envelope_params(index).sustain)
    } else {
        state.params().modulator_rack.config(index).sustain
    }
}

fn set_envelope_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
    normalized: f32,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let param = match stage {
            EnvelopeDrag::Attack => params.attack,
            EnvelopeDrag::DecaySustain => params.decay,
            EnvelopeDrag::Sustain => params.sustain,
            EnvelopeDrag::Release => params.release,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {
                return;
            }
        };
        state.set_param(param, f64::from(normalized));
        return;
    }
    let mut config = state.params().modulator_rack.config(index);
    match stage {
        EnvelopeDrag::Attack => config.attack = normalized * 8.0,
        EnvelopeDrag::AttackCurve => config.attack_curve = normalized.mul_add(2.0, -1.0),
        EnvelopeDrag::DecaySustain => config.decay = normalized * 8.0,
        EnvelopeDrag::DecayCurve => config.decay_curve = normalized.mul_add(2.0, -1.0),
        EnvelopeDrag::Sustain => config.sustain = normalized,
        EnvelopeDrag::Release => config.release = normalized * 12.0,
        EnvelopeDrag::ReleaseCurve => config.release_curve = normalized.mul_add(2.0, -1.0),
    }
    state.params().modulator_rack.set_config(index, config);
}

fn set_envelope_sustain_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    normalized: f32,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        state.set_param(envelope_params(index).sustain, f64::from(normalized));
        return;
    }
    let mut config = state.params().modulator_rack.config(index);
    config.sustain = normalized;
    state.params().modulator_rack.set_config(index, config);
}

fn draw_curve(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let dynamic = index >= LEGACY_MODULATION_SOURCES;
    let dynamic_config = dynamic.then(|| state.params().modulator_rack.config(index));
    let bipolar = dynamic_config.map_or_else(
        || state.get_param(lfo_params(index).bipolar) >= 0.5,
        |config| config.bipolar,
    );
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let plot = rect.shrink(editor_theme::graph_inset(ui));
    let painter = ui.painter_at(rect);
    editor_widgets::graph_frame(&painter, rect);
    let editor_id = response.id.with("spline-editor");
    let mut editor = ui
        .data(|store| store.get_temp::<SplineEditorUi>(editor_id))
        .unwrap_or_default();
    let curve = if dynamic {
        state.params().modulator_rack.curve(index)
    } else {
        Some(lfo_curve(state.params(), index))
    };
    let mut data = curve.map(|curve| editor.draft.clone().unwrap_or_else(|| curve.snapshot()));
    let mut compiled = data.as_ref().map_or_else(WaveCurveRt::default, |data| {
        if editor.draft.is_some() {
            data.compile_rt()
        } else {
            curve
                .and_then(WaveCurveState::try_curve_rt)
                .unwrap_or_else(|| data.compile_rt())
        }
    });
    let pointer = response.interact_pointer_pos();
    let point_radius = (plot.height() * 0.035).clamp(3.5, 6.0);
    let hit = data.as_ref().and_then(|data| {
        nearest_spline_target(data, compiled, plot, pointer?, bipolar, point_radius)
    });
    let mut point_hit = match hit {
        Some(SplineDrag::Point(point)) => Some(point),
        _ => None,
    };
    let mut handle_hit = match hit {
        Some(SplineDrag::Tension(handle)) => Some(handle),
        _ => None,
    };

    if let (Some(curve), Some(data)) = (curve, data.as_mut()) {
        let mut remove_point = None;
        let mut reset_segment = None;
        let mut reset_curve = false;
        response.context_menu(|ui| {
            if let Some(point) = point_hit {
                let removable = point > 0 && point + 1 < data.knots.len();
                if ui
                    .add_enabled(removable, egui::Button::new("REMOVE POINT"))
                    .on_disabled_hover_text("The first and last points anchor the cycle")
                    .clicked()
                {
                    remove_point = Some(point);
                    ui.close();
                }
            } else if let Some(segment) = handle_hit
                && ui.button("RESET BEND").clicked()
            {
                reset_segment = Some(segment);
                ui.close();
            }
            if point_hit.is_some() || handle_hit.is_some() {
                ui.separator();
            }
            if ui.button("RESET CURVE").clicked() {
                reset_curve = true;
                ui.close();
            }
        });
        if let Some(point) = remove_point {
            if remove_knot(data, point) {
                curve.replace(data.clone());
                editor.selected = None;
            }
            editor.drag = None;
            editor.draft = None;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if let Some(segment) = reset_segment {
            if set_segment_curve(data, segment, 0.0) {
                curve.replace(data.clone());
                editor.selected = Some(SplineDrag::Tension(segment));
            }
            editor.drag = None;
            editor.draft = None;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if reset_curve {
            *data = WaveCurveData::default();
            curve.replace(data.clone());
            editor.selected = None;
            editor.drag = None;
            editor.draft = None;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if response.double_clicked() {
            match hit {
                Some(SplineDrag::Point(point)) => {
                    if remove_knot(data, point) {
                        curve.replace(data.clone());
                        editor.selected = None;
                    }
                }
                Some(SplineDrag::Tension(segment)) => {
                    if set_segment_curve(data, segment, 0.0) {
                        curve.replace(data.clone());
                        editor.selected = Some(SplineDrag::Tension(segment));
                    }
                }
                None => {
                    if let Some(pointer) = pointer {
                        let (phase, value) = spline_values_from_pos(plot, pointer, bipolar);
                        if insert_knot(data, phase, value) {
                            curve.replace(data.clone());
                            editor.selected = nearest_knot(data, phase).map(SplineDrag::Point);
                        }
                    }
                }
            }
            editor.drag = None;
            editor.draft = None;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if response.drag_started() {
            if let Some(drag) = hit {
                editor.selected = Some(drag);
                editor.drag = Some(drag);
                editor.draft = Some(data.clone());
            }
        } else if response.clicked() {
            editor.selected = hit;
            editor.snap_phase = None;
            editor.snap_value = None;
        }

        if response.dragged()
            && let (Some(drag), Some(pointer), Some(draft)) = (
                editor.drag,
                response.interact_pointer_pos(),
                editor.draft.as_mut(),
            )
        {
            let (pointer_phase, value) = spline_values_from_pos(plot, pointer, bipolar);
            match drag {
                SplineDrag::Point(point) => {
                    let alt = ui.input(|input| input.modifiers.alt);
                    let (phase, value, snap_phase, snap_value) =
                        snap_spline_point(plot, pointer_phase, value, bipolar, point_radius, alt);
                    move_knot(draft, point, phase, value);
                    editor.snap_phase = snap_phase;
                    editor.snap_value = snap_value;
                    point_hit = Some(point);
                    handle_hit = None;
                }
                SplineDrag::Tension(segment) => {
                    let delta = ui.input(|input| input.pointer.delta());
                    let curve = draft.knots[segment].curve - delta.y / plot.height().max(1.0) * 3.0;
                    set_segment_curve(draft, segment, curve);
                    editor.snap_phase = None;
                    editor.snap_value = None;
                    point_hit = None;
                    handle_hit = Some(segment);
                }
            }
            *data = draft.clone();
            compiled = data.compile_rt();
            editor_theme::request_display_repaint(ui);
        }
        if response.drag_stopped() {
            if let Some(draft) = editor.draft.take() {
                curve.replace(draft);
                *data = curve.snapshot();
                compiled = curve.try_curve_rt().unwrap_or_else(|| data.compile_rt());
            }
            editor.drag = None;
            editor.snap_phase = None;
            editor.snap_value = None;
        }
        if editor.selected.is_some_and(|target| match target {
            SplineDrag::Point(point) | SplineDrag::Tension(point) => point >= data.knots.len(),
        }) {
            editor.selected = None;
        }
    }
    if let (Some(curve), Some(data)) = (curve, data.as_ref()) {
        compiled = if editor.draft.is_some() {
            data.compile_rt()
        } else {
            curve.try_curve_rt().unwrap_or_else(|| data.compile_rt())
        };
    }

    let baseline = if bipolar {
        plot.center().y
    } else {
        plot.bottom()
    };
    let points: Vec<_> = (0..=192)
        .map(|point| {
            let phase = point as f32 / 192.0;
            spline_pos(plot, phase, compiled.eval(phase), bipolar)
        })
        .collect();
    let color = source_color(index);
    if curve.is_some() && (response.hovered() || editor.selected.is_some()) {
        painter.rect_stroke(
            rect.shrink(1.0),
            3.0,
            egui::Stroke::new(
                1.0_f32,
                color.gamma_multiply(if response.hovered() { 0.6 } else { 0.32 }),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(phase) = editor.snap_phase {
        let x = egui::lerp(plot.left()..=plot.right(), phase);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.32)),
        );
    }
    if let Some(value) = editor.snap_value {
        let y = spline_pos(plot, 0.0, value, bipolar).y;
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.32)),
        );
    }
    editor_widgets::gradient_area_to_baseline(&painter, &points, baseline, color, 72);
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    ));
    let phase = lfo_phase_meter(state, index).clamp(0.0, 1.0);
    let playhead_x = egui::lerp(plot.left()..=plot.right(), phase);
    painter.line_segment(
        [
            egui::pos2(playhead_x, plot.top()),
            egui::pos2(playhead_x, plot.bottom()),
        ],
        egui::Stroke::new(3.0_f32, color.gamma_multiply(0.18)),
    );
    painter.line_segment(
        [
            egui::pos2(playhead_x, plot.top()),
            egui::pos2(playhead_x, plot.bottom()),
        ],
        egui::Stroke::new(1.0_f32, color),
    );
    painter.circle_filled(
        egui::pos2(playhead_x, plot.top() + point_radius * 0.5),
        point_radius * 0.42,
        color,
    );

    if let Some(data) = data.as_ref() {
        paint_spline_handles(
            ui,
            &painter,
            data,
            compiled,
            plot,
            bipolar,
            color,
            point_hit,
            handle_hit,
            editor.selected,
            editor.drag,
            point_radius,
        );
    }
    if response.hovered() {
        let hint = match editor.drag.or(hit).or(editor.selected) {
            Some(SplineDrag::Point(_)) => "POINT · DRAG X/Y",
            Some(SplineDrag::Tension(_)) => "BEND · DRAG Y",
            None => "DOUBLE-CLICK · ADD POINT",
        };
        painter.text(
            plot.right_top() + egui::vec2(-editor_theme::space::XS, editor_theme::space::XXS),
            egui::Align2::RIGHT_TOP,
            hint,
            editor_theme::font::caption(),
            color.gamma_multiply(0.78),
        );
        let cursor = if editor.drag.is_some() {
            egui::CursorIcon::Grabbing
        } else if point_hit.is_some() {
            egui::CursorIcon::Grab
        } else if handle_hit.is_some() {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::Crosshair
        };
        ui.output_mut(|output| output.cursor_icon = cursor);
    }
    response.clone().on_hover_text(
        "Drag points in X/Y; hold Alt to bypass nearby snaps. Drag segment handles to bend. Double-click empty space to add, a point to remove, or a bend handle to reset. Right-click for target-aware reset actions.",
    );
    let meter_moving = meter_is_moving(
        &mut editor.last_meter,
        &mut editor.meter_motion_frames,
        phase,
        true,
    );
    request_graph_repaint(ui, meter_moving);
    ui.data_mut(|store| store.insert_temp(editor_id, editor));
}

fn meter_is_moving(
    previous: &mut Option<f32>,
    motion_frames: &mut u8,
    value: f32,
    wraps: bool,
) -> bool {
    let changed = previous.is_some_and(|previous| {
        let delta = (value - previous).abs();
        let delta = if wraps { delta.min(1.0 - delta) } else { delta };
        delta > 0.000_5
    });
    *previous = Some(value);
    *motion_frames = if changed {
        2
    } else {
        (*motion_frames).saturating_sub(1)
    };
    *motion_frames > 0
}

fn request_graph_repaint(ui: &egui::Ui, meter_moving: bool) {
    if crate::editor_modulation::source_drag_active(ui) || ui.ctx().dragged_id().is_some() {
        editor_theme::request_display_repaint(ui);
    } else {
        ui.ctx().request_repaint_after(if meter_moving {
            LIVE_METER_REPAINT
        } else {
            IDLE_METER_REPAINT
        });
    }
}

#[derive(Clone, Copy)]
struct SegmentHandle {
    index: usize,
    position: egui::Pos2,
}

fn segment_handles(
    data: &WaveCurveData,
    compiled: WaveCurveRt,
    plot: egui::Rect,
    bipolar: bool,
    point_radius: f32,
) -> impl Iterator<Item = SegmentHandle> + '_ {
    data.knots
        .iter()
        .enumerate()
        .filter_map(move |(index, knot)| {
            let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
            let phase = (knot.phase + end) * 0.5;
            ((end - knot.phase) * plot.width() >= point_radius * 4.0).then(|| {
                let value = compiled.eval(phase);
                SegmentHandle {
                    index,
                    position: spline_pos(plot, phase, value, bipolar),
                }
            })
        })
}

fn nearest_spline_target(
    data: &WaveCurveData,
    compiled: WaveCurveRt,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
    point_radius: f32,
) -> Option<SplineDrag> {
    let hit_radius_sq = (point_radius * 2.5).powi(2);
    let point = data
        .knots
        .iter()
        .enumerate()
        .map(|(index, knot)| {
            (
                SplineDrag::Point(index),
                spline_pos(plot, knot.phase, knot.value, bipolar).distance_sq(pointer),
            )
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= hit_radius_sq)
        .map(|(target, _)| target);
    point.or_else(|| {
        segment_handles(data, compiled, plot, bipolar, point_radius)
            .map(|handle| {
                (
                    SplineDrag::Tension(handle.index),
                    handle.position.distance_sq(pointer),
                )
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= hit_radius_sq)
            .map(|(target, _)| target)
    })
}

fn snap_spline_point(
    plot: egui::Rect,
    phase: f32,
    value: f32,
    bipolar: bool,
    point_radius: f32,
    disabled: bool,
) -> (f32, f32, Option<f32>, Option<f32>) {
    if disabled {
        return (phase, value, None, None);
    }
    let proximity = point_radius * 1.5;
    let snap_phase = [0.25_f32, 0.5, 0.75]
        .into_iter()
        .map(|target| (target, (target - phase).abs() * plot.width()))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= proximity)
        .map(|(target, _)| target);
    let value_scale = plot.height() * if bipolar { 0.42 } else { 0.45 };
    let snap_value = [-1.0_f32, -0.5, 0.0, 0.5, 1.0]
        .into_iter()
        .map(|target| (target, (target - value).abs() * value_scale))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= proximity)
        .map(|(target, _)| target);
    (
        snap_phase.unwrap_or(phase),
        snap_value.unwrap_or(value),
        snap_phase,
        snap_value,
    )
}

#[allow(clippy::too_many_arguments)]
fn paint_spline_handles(
    ui: &egui::Ui,
    painter: &egui::Painter,
    data: &WaveCurveData,
    compiled: WaveCurveRt,
    plot: egui::Rect,
    bipolar: bool,
    color: egui::Color32,
    hovered_point: Option<usize>,
    hovered_handle: Option<usize>,
    selected: Option<SplineDrag>,
    active_drag: Option<SplineDrag>,
    point_radius: f32,
) {
    let palette = editor_theme::semantic();
    let removing = ui.input(|input| input.pointer.button_down(egui::PointerButton::Secondary));
    for handle in segment_handles(data, compiled, plot, bipolar, point_radius) {
        let hovered = hovered_handle == Some(handle.index);
        let selected = selected == Some(SplineDrag::Tension(handle.index));
        let active = active_drag == Some(SplineDrag::Tension(handle.index));
        let radius = point_radius
            * if active {
                1.0
            } else if selected {
                0.84
            } else if hovered {
                0.72
            } else {
                0.48
            };
        if hovered || selected || active {
            painter.circle_filled(handle.position, radius * 1.55, color.gamma_multiply(0.14));
        }
        painter.circle_filled(
            handle.position,
            radius,
            if active {
                color
            } else if hovered {
                palette.control_hover
            } else {
                palette.well
            },
        );
        painter.circle_stroke(
            handle.position,
            radius,
            egui::Stroke::new(
                (point_radius * 0.2).clamp(0.8, 1.25),
                color.gamma_multiply(if active || selected || hovered {
                    0.9
                } else {
                    0.48
                }),
            ),
        );
        painter.line_segment(
            [
                handle.position - egui::vec2(0.0, radius * 0.5),
                handle.position + egui::vec2(0.0, radius * 0.5),
            ],
            egui::Stroke::new(
                (point_radius * 0.14).clamp(0.7, 1.0),
                if active || selected || hovered {
                    color
                } else {
                    palette.text_muted
                },
            ),
        );
    }
    for (index, knot) in data.knots.iter().enumerate() {
        let position = spline_pos(plot, knot.phase, knot.value, bipolar);
        let hovered = hovered_point == Some(index);
        let selected = selected == Some(SplineDrag::Point(index));
        let active = active_drag == Some(SplineDrag::Point(index));
        let removing = hovered && removing;
        let radius = point_radius
            * if active {
                1.16
            } else if selected {
                1.0
            } else if hovered {
                0.88
            } else {
                0.72
            };
        if active || selected || removing {
            painter.circle_stroke(
                position,
                radius * 1.45,
                egui::Stroke::new(
                    (point_radius * 0.22).clamp(0.9, 1.4),
                    if removing {
                        palette.danger.gamma_multiply(0.72)
                    } else {
                        color.gamma_multiply(0.52)
                    },
                ),
            );
        }
        painter.circle_filled(
            position,
            radius,
            if removing {
                palette.danger
            } else if active || selected || hovered {
                color
            } else {
                palette.well
            },
        );
        painter.circle_stroke(
            position,
            radius,
            egui::Stroke::new(
                (point_radius * 0.2).clamp(0.8, 1.25),
                if removing {
                    palette.text
                } else if active || selected || hovered {
                    palette.text
                } else {
                    color
                },
            ),
        );
    }
}

fn nearest_knot(data: &WaveCurveData, phase: f32) -> Option<usize> {
    data.knots
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            (left.phase - phase)
                .abs()
                .total_cmp(&(right.phase - phase).abs())
        })
        .map(|(index, _)| index)
}

fn spline_pos(plot: egui::Rect, phase: f32, value: f32, bipolar: bool) -> egui::Pos2 {
    let y = if bipolar {
        (-value * plot.height() * 0.42).mul_add(1.0, plot.center().y)
    } else {
        plot.bottom() - value.mul_add(0.5, 0.5) * plot.height() * 0.9
    };
    egui::pos2(phase.mul_add(plot.width(), plot.left()), y)
}

fn spline_values_from_pos(plot: egui::Rect, position: egui::Pos2, bipolar: bool) -> (f32, f32) {
    let phase = ((position.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let value = if bipolar {
        (plot.center().y - position.y) / (plot.height() * 0.42)
    } else {
        ((plot.bottom() - position.y) / (plot.height() * 0.9)).mul_add(2.0, -1.0)
    }
    .clamp(-1.0, 1.0);
    (phase, value)
}

fn draw_in_rect(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    add: impl FnOnce(&mut egui::Ui),
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing = egui::Vec2::ZERO;
    add(&mut child);
}

fn lfo_phase_meter(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index >= LEGACY_MODULATION_SOURCES {
        return state.params().modulator_rack.ui_snapshot(index).0;
    }
    let params = state.params();
    let meter = match index {
        0 => &params.lfo1_phase_meter,
        1 => &params.lfo2_phase_meter,
        2 => &params.lfo3_phase_meter,
        3 => &params.lfo4_phase_meter,
        4 => &params.lfo5_phase_meter,
        5 => &params.lfo6_phase_meter,
        6 => &params.lfo7_phase_meter,
        _ => &params.lfo8_phase_meter,
    };
    state.get_meter(meter)
}

fn source_value_meter(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index >= LEGACY_MODULATION_SOURCES {
        return state.params().modulator_rack.ui_snapshot(index).1;
    }
    let params = state.params();
    let meter = match index {
        0 => &params.lfo1_value_meter,
        1 => &params.lfo2_value_meter,
        2 => &params.lfo3_value_meter,
        3 => &params.lfo4_value_meter,
        4 => &params.lfo5_value_meter,
        5 => &params.lfo6_value_meter,
        6 => &params.lfo7_value_meter,
        _ => &params.lfo8_value_meter,
    };
    state.get_meter(meter)
}

fn source_is_envelope(state: &PluginContext<KurvParams>, index: usize) -> bool {
    if index < LEGACY_MODULATION_SOURCES {
        state.get_param(source_envelope_param(index)) >= 0.5
    } else {
        state.params().modulator_rack.config(index).kind == SourceKind::Envelope
    }
}

const fn source_envelope_param(index: usize) -> P {
    match index {
        0 => P::Source1Envelope,
        1 => P::Source2Envelope,
        2 => P::Source3Envelope,
        3 => P::Source4Envelope,
        4 => P::Source5Envelope,
        5 => P::Source6Envelope,
        6 => P::Source7Envelope,
        _ => P::Source8Envelope,
    }
}

const fn envelope_params(index: usize) -> EnvelopeParams {
    match index {
        0 => EnvelopeParams {
            attack: P::Source1Attack,
            decay: P::Source1Decay,
            sustain: P::Source1Sustain,
            release: P::Source1Release,
        },
        1 => EnvelopeParams {
            attack: P::Source2Attack,
            decay: P::Source2Decay,
            sustain: P::Source2Sustain,
            release: P::Source2Release,
        },
        2 => EnvelopeParams {
            attack: P::Source3Attack,
            decay: P::Source3Decay,
            sustain: P::Source3Sustain,
            release: P::Source3Release,
        },
        3 => EnvelopeParams {
            attack: P::Source4Attack,
            decay: P::Source4Decay,
            sustain: P::Source4Sustain,
            release: P::Source4Release,
        },
        4 => EnvelopeParams {
            attack: P::Source5Attack,
            decay: P::Source5Decay,
            sustain: P::Source5Sustain,
            release: P::Source5Release,
        },
        5 => EnvelopeParams {
            attack: P::Source6Attack,
            decay: P::Source6Decay,
            sustain: P::Source6Sustain,
            release: P::Source6Release,
        },
        6 => EnvelopeParams {
            attack: P::Source7Attack,
            decay: P::Source7Decay,
            sustain: P::Source7Sustain,
            release: P::Source7Release,
        },
        _ => EnvelopeParams {
            attack: P::Source8Attack,
            decay: P::Source8Decay,
            sustain: P::Source8Sustain,
            release: P::Source8Release,
        },
    }
}

fn envelope_values(params: &KurvParams, index: usize) -> [f32; 4] {
    if index >= LEGACY_MODULATION_SOURCES {
        let config = params.modulator_rack.config(index);
        return [config.attack, config.decay, config.sustain, config.release];
    }
    match index {
        0 => [
            params.source1_attack.value(),
            params.source1_decay.value(),
            params.source1_sustain.value(),
            params.source1_release.value(),
        ],
        1 => [
            params.source2_attack.value(),
            params.source2_decay.value(),
            params.source2_sustain.value(),
            params.source2_release.value(),
        ],
        2 => [
            params.source3_attack.value(),
            params.source3_decay.value(),
            params.source3_sustain.value(),
            params.source3_release.value(),
        ],
        3 => [
            params.source4_attack.value(),
            params.source4_decay.value(),
            params.source4_sustain.value(),
            params.source4_release.value(),
        ],
        4 => [
            params.source5_attack.value(),
            params.source5_decay.value(),
            params.source5_sustain.value(),
            params.source5_release.value(),
        ],
        5 => [
            params.source6_attack.value(),
            params.source6_decay.value(),
            params.source6_sustain.value(),
            params.source6_release.value(),
        ],
        6 => [
            params.source7_attack.value(),
            params.source7_decay.value(),
            params.source7_sustain.value(),
            params.source7_release.value(),
        ],
        _ => [
            params.source8_attack.value(),
            params.source8_decay.value(),
            params.source8_sustain.value(),
            params.source8_release.value(),
        ],
    }
}

fn envelope_curve_values(params: &KurvParams, index: usize) -> [f32; 3] {
    if index < LEGACY_MODULATION_SOURCES {
        return [0.0; 3];
    }
    let config = params.modulator_rack.config(index);
    [
        config.attack_curve,
        config.decay_curve,
        config.release_curve,
    ]
}

fn active_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    let stored = active_params()
        .into_iter()
        .enumerate()
        .fold(0, |mask, (index, param)| {
            if state.get_param(param) >= 0.5 {
                mask | (1_u64 << index)
            } else {
                mask
            }
        });
    stored | state.params().modulator_rack.active_mask() | used_source_mask(state)
}

fn set_source_active(
    state: &PluginContext<KurvParams>,
    index: usize,
    active: bool,
    kind: SourceKind,
) {
    if index < LEGACY_MODULATION_SOURCES {
        state.automate(active_params()[index], if active { 1.0 } else { 0.0 });
        state.automate(
            source_envelope_param(index),
            if kind == SourceKind::Envelope {
                1.0
            } else {
                0.0
            },
        );
    } else {
        let mut config = state.params().modulator_rack.config(index);
        config.active = active;
        if active {
            config.kind = kind;
        }
        state.params().modulator_rack.set_config(index, config);
    }
}

const fn active_params() -> [P; 8] {
    [
        P::Lfo1Active,
        P::Lfo2Active,
        P::Lfo3Active,
        P::Lfo4Active,
        P::Lfo5Active,
        P::Lfo6Active,
        P::Lfo7Active,
        P::Lfo8Active,
    ]
}

fn rate_mode(state: &PluginContext<KurvParams>, param: P) -> u8 {
    (state.get_param(param).clamp(0.0, 1.0) * 3.0).round() as u8
}

fn rate_text(state: &PluginContext<KurvParams>, index: usize, rate_mode_param: P) -> String {
    let rate = lfo_rate(state.params(), index).max(0.000_01);
    match rate_mode(state, rate_mode_param) {
        1 => {
            let milliseconds = rate;
            if milliseconds < 10.0 {
                format!("{milliseconds:.2} ms")
            } else {
                format!("{milliseconds:.0} ms")
            }
        }
        3 => format!("{:.2}×", crate::modulators::lfo::keytrack_multiplier(rate)),
        _ if rate < 10.0 => format!("{rate:.2} Hz"),
        _ => format!("{rate:.0} Hz"),
    }
}

fn lfo_rate(params: &KurvParams, index: usize) -> f32 {
    match index {
        0 => params.lfo1_rate.value(),
        1 => params.lfo2_rate.value(),
        2 => params.lfo3_rate.value(),
        3 => params.lfo4_rate.value(),
        4 => params.lfo5_rate.value(),
        5 => params.lfo6_rate.value(),
        6 => params.lfo7_rate.value(),
        _ => params.lfo8_rate.value(),
    }
}

fn lfo_params(index: usize) -> LfoParams {
    match index {
        0 => LfoParams {
            rate: P::Lfo1Rate,
            rate_mode: P::Lfo1RateMode,
            mode: P::Lfo1Mode,
            phase: P::Lfo1Phase,
            sync: P::Lfo1Sync,
            bipolar: P::Lfo1Bipolar,
        },
        1 => LfoParams {
            rate: P::Lfo2Rate,
            rate_mode: P::Lfo2RateMode,
            mode: P::Lfo2Mode,
            phase: P::Lfo2Phase,
            sync: P::Lfo2Sync,
            bipolar: P::Lfo2Bipolar,
        },
        2 => LfoParams {
            rate: P::Lfo3Rate,
            rate_mode: P::Lfo3RateMode,
            mode: P::Lfo3Mode,
            phase: P::Lfo3Phase,
            sync: P::Lfo3Sync,
            bipolar: P::Lfo3Bipolar,
        },
        3 => LfoParams {
            rate: P::Lfo4Rate,
            rate_mode: P::Lfo4RateMode,
            mode: P::Lfo4Mode,
            phase: P::Lfo4Phase,
            sync: P::Lfo4Sync,
            bipolar: P::Lfo4Bipolar,
        },
        4 => LfoParams {
            rate: P::Lfo5Rate,
            rate_mode: P::Lfo5RateMode,
            mode: P::Lfo5Mode,
            phase: P::Lfo5Phase,
            sync: P::Lfo5Sync,
            bipolar: P::Lfo5Bipolar,
        },
        5 => LfoParams {
            rate: P::Lfo6Rate,
            rate_mode: P::Lfo6RateMode,
            mode: P::Lfo6Mode,
            phase: P::Lfo6Phase,
            sync: P::Lfo6Sync,
            bipolar: P::Lfo6Bipolar,
        },
        6 => LfoParams {
            rate: P::Lfo7Rate,
            rate_mode: P::Lfo7RateMode,
            mode: P::Lfo7Mode,
            phase: P::Lfo7Phase,
            sync: P::Lfo7Sync,
            bipolar: P::Lfo7Bipolar,
        },
        _ => LfoParams {
            rate: P::Lfo8Rate,
            rate_mode: P::Lfo8RateMode,
            mode: P::Lfo8Mode,
            phase: P::Lfo8Phase,
            sync: P::Lfo8Sync,
            bipolar: P::Lfo8Bipolar,
        },
    }
}

fn lfo_curve(params: &KurvParams, index: usize) -> &WaveCurveState {
    match index {
        0 => &params.lfo1_curve_state,
        1 => &params.lfo2_curve_state,
        2 => &params.lfo3_curve_state,
        3 => &params.lfo4_curve_state,
        4 => &params.lfo5_curve_state,
        5 => &params.lfo6_curve_state,
        6 => &params.lfo7_curve_state,
        _ => &params.lfo8_curve_state,
    }
}
