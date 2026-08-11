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

mod controls;
mod envelope_editor;
mod source;
mod source_card;
mod spline_editor;

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

fn nearest_modulator_insertion(
    ui: &egui::Ui,
    visible_sources: &[usize],
    collapsed_modulators: u64,
    reserved: Option<usize>,
) -> Option<usize> {
    let pointer = ui.input(|input| {
        (input.modifiers.alt
            && ui.ctx().dragged_id().is_none()
            && !crate::editor_modulation::source_drag_active(ui))
        .then_some(input.pointer.latest_pos())
        .flatten()
    })?;
    let rack = ui.available_rect_before_wrap();
    if !rack.contains(pointer) {
        return None;
    }
    let row_height = editor_theme::title_height(ui);
    let gap = ui.spacing().item_spacing.y;
    let threshold = (gap * 0.5).max(editor_theme::shape::FOCUS_STROKE * 2.0);
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
    }
    let stroke_color = if insertion || (can_add && response.hovered()) {
        palette.primary
    } else if can_add {
        palette.grid
    } else {
        palette.grid.gamma_multiply(0.48)
    };
    let stroke = egui::Stroke::new(
        if open {
            editor_theme::shape::FOCUS_STROKE
        } else {
            editor_theme::shape::STROKE
        },
        stroke_color,
    );
    let outline = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &outline,
        stroke,
        rect.height() * 0.42,
        rect.height() * 0.30,
    ));
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
