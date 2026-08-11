//! Oscillator waveform preview and quality controls.

use std::sync::Arc;

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::generators::{ModuleId, OscillatorConfig, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::oscillators::{
    Antialiasing, MAX_VA_TABLE_FRAMES, PhaseWarpMode, VaTableRt, VaTableState,
    sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::{
    WaveCurveData, WaveCurveRt, fit_freehand_curve, insert_knot, move_knot, remove_knot,
    set_segment_curve,
};
use crate::{KurvParams, P, editor_theme, editor_widgets};

const HOST_PREVIEW_SAMPLE_RATE: f32 = 48_000.0;
const PREVIEW_POINTS: u16 = 512;

#[derive(Clone, Default)]
struct FreehandStroke {
    points: Vec<(f32, f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurveDragTarget {
    Knot(usize),
    Segment(usize),
}

#[derive(Clone)]
struct VaPreviewCache {
    generation: u32,
    table: Arc<VaTableRt>,
}

struct VaTableUi {
    response: egui::Response,
    menu_response: egui::Response,
    position: f32,
    chrome: egui::Rect,
    duplicate: bool,
    remove: bool,
    reset: bool,
}

/// Full VA-table editor for structurally-added oscillator slots.
pub(crate) fn oscillator_waveform_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut OscillatorConfig,
) -> bool {
    let table_state = state.params().generator_stack.va_table(slot);
    let cache_id = ui.id().with(("va-preview-cache", slot.index()));
    let mut cache = ui.data(|store| store.get_temp::<VaPreviewCache>(cache_id));
    let mut cache_changed = false;
    if let Some((generation, table)) =
        table_state.try_table_rt(cache.as_ref().map_or(0, |cache| cache.generation))
    {
        cache = Some(VaPreviewCache {
            generation,
            table: Arc::new(table),
        });
        cache_changed = true;
    }
    if cache.is_none() {
        cache = Some(VaPreviewCache {
            generation: 0,
            table: Arc::new(table_state.snapshot().compile_rt()),
        });
        cache_changed = true;
    }
    let cache = cache.expect("VA preview cache is initialized above");
    if cache_changed {
        ui.data_mut(|store| store.insert_temp(cache_id, cache.clone()));
    }
    let table = cache.table.as_ref();
    let fallback = WaveCurveData::default();
    let selection = table.select(fallback.compile_rt(), config.custom_shape);
    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let phase_step = 110.0_f64 / f64::from(HOST_PREVIEW_SAMPLE_RATE);
    let plot = paint_cycle(ui, &painter, response.rect, |normalized| {
        sample_custom_shape_with_antialiasing_warped(
            config.shape.clamp(0.0, 3.0),
            f64::from((normalized + config.phase_position).rem_euclid(1.0)),
            phase_step,
            config.pulse_width.clamp(0.03, 0.97),
            Antialiasing::Spline,
            PhaseWarpMode::from_index(config.phase_warp_mode),
            config.phase_warp_amount,
            selection.curve,
            selection.mix,
        )
    });
    let table_frames = table.frame_count();
    let custom_frames = table_frames.max(1);
    let selected_frame =
        ((config.custom_shape * custom_frames as f32).round() as usize).clamp(1, custom_frames) - 1;
    let host_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::TablePosition);
    let host_binding = crate::editor_modulation::host_automation_binding(state, host_target);
    let table_ui = va_table_label(
        ui,
        &painter,
        plot,
        &response,
        config.custom_shape,
        selected_frame,
        custom_frames,
        custom_frames < MAX_VA_TABLE_FRAMES,
        table_frames > 0,
        host_binding.as_ref().map(|(slot, _, _)| *slot),
    );
    let mut changed = false;
    if table_ui.response.dragged() || table_ui.response.clicked() {
        config.custom_shape = table_ui.position;
        changed = true;
    }
    if table_ui.duplicate
        && let Some(new_selected) = table_state.duplicate_after(selected_frame, fallback.clone())
    {
        let new_frame_count = table_state.snapshot().frames.len().max(1);
        config.custom_shape = (new_selected + 1) as f32 / new_frame_count as f32;
        changed = true;
    }
    if table_ui.remove && table_state.remove_frame(selected_frame) {
        let new_frame_count = table_state.snapshot().frames.len().max(1);
        let new_selected = selected_frame.min(new_frame_count - 1);
        config.custom_shape = (new_selected + 1) as f32 / new_frame_count as f32;
        changed = true;
    }
    if table_ui.reset {
        table_state.replace(Default::default());
        config.custom_shape = 0.0;
        changed = true;
    }
    table_ui.menu_response.context_menu(|ui| {
        if ui
            .add_enabled(
                custom_frames < MAX_VA_TABLE_FRAMES,
                egui::Button::new("Duplicate frame"),
            )
            .clicked()
            && let Some(new_selected) =
                table_state.duplicate_after(selected_frame, fallback.clone())
        {
            let new_frame_count = table_state.snapshot().frames.len().max(1);
            config.custom_shape = (new_selected + 1) as f32 / new_frame_count as f32;
            changed = true;
            ui.close();
        }
        if ui
            .add_enabled(table_frames > 0, egui::Button::new("Remove this VA frame"))
            .clicked()
            && table_state.remove_frame(selected_frame)
        {
            let new_frame_count = table_state.snapshot().frames.len().max(1);
            let new_selected = selected_frame.min(new_frame_count - 1);
            config.custom_shape = (new_selected + 1) as f32 / new_frame_count as f32;
            changed = true;
            ui.close();
        }
        if ui.button("Reset VA table").clicked() {
            table_state.replace(Default::default());
            config.custom_shape = 0.0;
            changed = true;
            ui.close();
        }
        ui.separator();
        crate::editor_modulation::host_automation_menu(ui, state, host_target, config.custom_shape);
    });
    let pointer_over_chrome = response
        .interact_pointer_pos()
        .is_some_and(|pointer| table_ui.chrome.contains(pointer));
    let pointer_over_handle = table_frames > 0
        && config.custom_shape > 0.001
        && response
            .interact_pointer_pos()
            .filter(|pointer| plot.contains(*pointer))
            .and_then(|pointer| {
                table_state.frame_snapshot(selected_frame).and_then(|data| {
                    let curve = data.compile_rt();
                    hit_curve_target(&data, &curve, plot, pointer, true)
                })
            })
            .is_some();
    let reset_requested = table_frames > 0
        && response.secondary_clicked()
        && !pointer_over_chrome
        && !pointer_over_handle;
    if reset_requested {
        changed |= table_state.replace_frame(selected_frame, fallback.clone());
    }
    if table_frames > 0 && config.custom_shape > 0.001 && !reset_requested {
        edit_wave_curve_target(
            ui,
            &response,
            plot,
            table_state,
            selected_frame,
            slot.index(),
            editor_theme::palette().accent,
            true,
        );
    } else if response.double_clicked() && !pointer_over_chrome {
        let _ = table_state.materialize(fallback);
        config.custom_shape = 1.0;
        changed = true;
    }
    if table_frames == 0 && response.hovered() {
        painter.text(
            plot.left_top() + egui::vec2(editor_theme::space::XS, editor_theme::space::XXS),
            egui::Align2::LEFT_TOP,
            "DOUBLE-CLICK TO EDIT",
            editor_theme::font::caption(),
            editor_theme::semantic().text_muted.gamma_multiply(0.72),
        );
    }
    response.on_hover_text(if pointer_over_handle {
        "Drag the handle to reshape the cycle. Right-click for its reset or remove action."
    } else if table_frames > 0 && config.custom_shape > 0.001 {
        "Drag to draw. Double-click to add a point. Right-click empty space to reset this frame."
    } else {
        "Double-click to edit this cycle. Right-click empty space to reset the VA table."
    });
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state,
            param,
            &table_ui.response,
            config.custom_shape,
            changed,
        );
    }
    changed && host_binding.is_none()
}

fn va_table_label(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    plot: egui::Rect,
    parent: &egui::Response,
    position: f32,
    selected_frame: usize,
    custom_frames: usize,
    can_duplicate: bool,
    can_remove: bool,
    host_slot: Option<usize>,
) -> VaTableUi {
    let row_height = editor_theme::font::LABEL_SIZE + editor_theme::space::XS;
    let action_size = row_height;
    let gap = editor_theme::compact_gap(ui);
    let action_width = action_size * 3.0 + gap * 2.0;
    let right = plot.right() - editor_theme::space::XXS;
    let top = plot.top() + editor_theme::space::XXS;
    let label_text = format!("VA {}/{}", selected_frame + 1, custom_frames);
    let label_font = editor_theme::font::caption();
    let label_width = painter
        .layout_no_wrap(label_text.clone(), label_font.clone(), egui::Color32::WHITE)
        .size()
        .x
        + host_slot.map_or(0.0, |_| {
            painter
                .layout_no_wrap("H00".into(), label_font.clone(), egui::Color32::WHITE)
                .size()
                .x
                + editor_theme::space::SM
        })
        + editor_theme::space::XS;
    let label_rect = egui::Rect::from_min_max(
        egui::pos2((right - label_width).max(plot.left()), top),
        egui::pos2(right, (top + row_height).min(plot.bottom())),
    );
    let action_left = (label_rect.left() - gap - action_width).max(plot.left());
    let label_response = ui
        .interact(
            label_rect,
            parent.id.with("va-table-label"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Drag vertically to morph VA frames. Hold Shift for fine control; double-click resets. Right-click for frame actions.",
        );
    let response = label_response.clone();
    let value = if response.double_clicked() {
        0.0
    } else if response.dragged() {
        let sensitivity = if ui.input(|input| input.modifiers.shift) {
            0.001
        } else {
            0.005
        };
        (position - response.drag_motion().y * sensitivity).clamp(0.0, 1.0)
    } else {
        position
    };
    let palette = editor_theme::semantic();
    painter.text(
        label_rect.right_center() - egui::vec2(editor_theme::space::XXS, 0.0),
        egui::Align2::RIGHT_CENTER,
        label_text,
        label_font.clone(),
        if response.is_pointer_button_down_on() || response.dragged() {
            palette.text
        } else if response.hovered() {
            palette.primary.gamma_multiply(0.82)
        } else {
            palette.text_muted
        },
    );
    if let Some(slot) = host_slot {
        painter.text(
            label_rect.left_center() + egui::vec2(editor_theme::space::XXS, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("H{:02}", slot + 1),
            label_font,
            palette.primary,
        );
    }

    let actions_fit = label_rect.left() - plot.left() >= action_width + gap;
    let action_region = egui::Rect::from_min_max(
        egui::pos2(action_left, top),
        egui::pos2(label_rect.right(), label_rect.bottom()),
    );
    let show_actions = actions_fit
        && ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pointer| action_region.contains(pointer));
    let mut duplicate = false;
    let mut remove = false;
    let mut reset = false;
    if show_actions {
        let action_rect = |index: usize| {
            let left = action_left + index as f32 * (action_size + gap);
            egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(action_size, action_size))
        };
        duplicate = curve_action(
            ui,
            action_rect(0),
            parent.id.with("duplicate-va-frame"),
            "+",
            "Duplicate this VA frame",
            can_duplicate,
        );
        remove = curve_action(
            ui,
            action_rect(1),
            parent.id.with("remove-va-frame"),
            "−",
            "Remove this VA frame",
            can_remove,
        );
        reset = curve_action(
            ui,
            action_rect(2),
            parent.id.with("reset-va-table"),
            "↺",
            "Reset the VA table",
            true,
        );
    }
    let action_chrome = if show_actions {
        egui::Rect::from_min_max(
            egui::pos2(action_left, top),
            egui::pos2(label_rect.left() - gap, top + row_height),
        )
    } else {
        label_rect
    };
    VaTableUi {
        menu_response: response.clone(),
        response,
        position: value,
        chrome: action_chrome.union(label_rect),
        duplicate,
        remove,
        reset,
    }
}

fn curve_action(
    ui: &egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    symbol: &str,
    tooltip: &str,
    enabled: bool,
) -> bool {
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_text(tooltip);
    if enabled && response.hovered() {
        ui.output_mut(|output| output.cursor_icon = egui::CursorIcon::PointingHand);
    }
    let palette = editor_theme::semantic();
    if enabled && (response.hovered() || response.is_pointer_button_down_on()) {
        ui.painter().rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            if response.is_pointer_button_down_on() {
                palette.control_hover
            } else {
                palette.control
            },
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        symbol,
        editor_theme::font::label(),
        if enabled {
            if response.hovered() {
                palette.primary
            } else {
                palette.text_muted
            }
        } else {
            palette.disabled_text
        },
    );
    enabled && response.clicked()
}

fn paint_cycle(
    _ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    mut sample_at: impl FnMut(f32) -> f32,
) -> egui::Rect {
    let inset = editor_theme::space::XS.min(rect.height() * 0.12);
    let plot = rect.shrink2(egui::vec2(inset, inset * 0.65));
    painter.rect_filled(rect, 0.0, editor_theme::semantic().well);
    let points: Vec<_> = (0..=PREVIEW_POINTS)
        .map(|index| {
            let normalized = f32::from(index) / f32::from(PREVIEW_POINTS);
            let sample = sample_at(normalized);
            egui::pos2(
                plot.width().mul_add(normalized, plot.left()),
                (sample * plot.height()).mul_add(-0.42, plot.center().y),
            )
        })
        .collect();
    let color = editor_theme::palette().accent;
    editor_widgets::gradient_area_to_baseline(painter, &points, plot.center().y, color, 42);
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.45_f32, color),
    ));
    plot
}

#[allow(clippy::too_many_arguments)]
fn edit_wave_curve_target(
    ui: &egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    table: &VaTableState,
    frame: usize,
    oscillator: usize,
    color: egui::Color32,
    bipolar: bool,
) {
    let drag_id = response.id.with(("wave-curve-drag", oscillator));
    let stroke_id = response.id.with(("wave-curve-stroke", oscillator));
    let draft_id = response.id.with(("wave-curve-draft", oscillator));
    let selection_id = response
        .id
        .with(("wave-curve-selection", oscillator, frame));
    let mut data = if let Some(draft) = ui.data(|store| store.get_temp(draft_id)) {
        draft
    } else {
        let Some(data) = table.frame_snapshot(frame) else {
            return;
        };
        data
    };
    let drag_pointer = response.interact_pointer_pos();
    let pointer = drag_pointer.filter(|pointer| plot.contains(*pointer));
    let hit_curve = data.compile_rt();
    let hit =
        pointer.and_then(|pointer| hit_curve_target(&data, &hit_curve, plot, pointer, bipolar));
    let knot_hit = match hit {
        Some(CurveDragTarget::Knot(index)) => Some(index),
        _ => None,
    };
    let curve_hit = match hit {
        Some(CurveDragTarget::Segment(index)) => Some(index),
        _ => None,
    };

    if response.double_clicked() {
        if let Some(index) = curve_hit {
            let _ = table.edit_frame(frame, |data| set_segment_curve(data, index, 0.0));
            data = table.frame_snapshot(frame).unwrap_or_default();
        } else if knot_hit.is_none()
            && let Some(pointer) = pointer
        {
            let (phase, value) = values_from_pos(plot, pointer, bipolar);
            let _ = table.edit_frame(frame, |data| insert_knot(data, phase, value));
            data = table.frame_snapshot(frame).unwrap_or_default();
        }
    } else if response.secondary_clicked() {
        if let Some(index) = knot_hit {
            let _ = table.edit_frame(frame, |data| remove_knot(data, index));
            data = table.frame_snapshot(frame).unwrap_or_default();
            ui.data_mut(|store| store.remove::<CurveDragTarget>(selection_id));
        } else if let Some(index) = curve_hit {
            let _ = table.edit_frame(frame, |data| set_segment_curve(data, index, 0.0));
            data = table.frame_snapshot(frame).unwrap_or_default();
        }
    } else if response.drag_started() {
        if let Some(index) = knot_hit {
            ui.data_mut(|store| {
                store.insert_temp(drag_id, CurveDragTarget::Knot(index));
                store.insert_temp(selection_id, CurveDragTarget::Knot(index));
                store.insert_temp(draft_id, data.clone());
            });
        } else if let Some(index) = curve_hit {
            ui.data_mut(|store| {
                store.insert_temp(drag_id, CurveDragTarget::Segment(index));
                store.insert_temp(selection_id, CurveDragTarget::Segment(index));
                store.insert_temp(draft_id, data.clone());
            });
        } else if let Some(pointer) = pointer {
            ui.data_mut(|store| {
                store.insert_temp(
                    stroke_id,
                    FreehandStroke {
                        points: vec![values_from_pos(plot, pointer, bipolar)],
                    },
                );
            });
        }
    }

    if response.dragged()
        && let Some(pointer) = drag_pointer
    {
        let point = values_from_pos(plot, pointer, bipolar);
        if let Some(drag) = ui.data(|store| store.get_temp::<CurveDragTarget>(drag_id)) {
            if let Some(mut draft) = ui.data(|store| store.get_temp::<WaveCurveData>(draft_id)) {
                match drag {
                    CurveDragTarget::Knot(index) => {
                        let snap = !ui.input(|input| input.modifiers.alt);
                        let (phase, value) = if snap {
                            snap_curve_point(point, plot)
                        } else {
                            point
                        };
                        move_knot(&mut draft, index, phase, value);
                    }
                    CurveDragTarget::Segment(index) => {
                        let precision = if ui.input(|input| input.modifiers.shift) {
                            0.25
                        } else {
                            1.0
                        };
                        let current = draft.knots.get(index).map_or(0.0, |knot| knot.curve);
                        let delta = -response.drag_motion().y / plot.height().max(1.0);
                        set_segment_curve(&mut draft, index, current + delta * 3.0 * precision);
                    }
                }
                data = draft.clone();
                ui.data_mut(|store| store.insert_temp(draft_id, draft));
            }
        } else if let Some(mut stroke) =
            ui.data_mut(|store| store.remove_temp::<FreehandStroke>(stroke_id))
        {
            if stroke.points.last().is_none_or(|last| {
                (last.0 - point.0).abs() > 0.001 || (last.1 - point.1).abs() > 0.002
            }) {
                stroke.points.push(point);
            }
            ui.data_mut(|store| store.insert_temp(stroke_id, stroke));
        }
        editor_theme::request_display_repaint(ui);
    }
    if response.drag_stopped() {
        let draft = ui.data_mut(|store| {
            let draft = store.remove_temp::<WaveCurveData>(draft_id);
            store.remove::<CurveDragTarget>(drag_id);
            draft
        });
        if let Some(draft) = draft {
            let _ = table.replace_frame(frame, draft);
            data = table.frame_snapshot(frame).unwrap_or_default();
        } else if let Some(stroke) =
            ui.data_mut(|store| store.remove_temp::<FreehandStroke>(stroke_id))
            && stroke.points.len() >= 2
        {
            let _ = table.replace_frame(frame, fit_freehand_curve(&data, &stroke.points));
            data = table.frame_snapshot(frame).unwrap_or_default();
        }
    }

    if let Some(stroke) = ui.data_mut(|store| store.remove_temp::<FreehandStroke>(stroke_id)) {
        let points = stroke
            .points
            .iter()
            .map(|(phase, value)| value_pos(plot, *phase, *value, bipolar))
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, egui::Stroke::new(1.5_f32, color)));
        ui.data_mut(|store| store.insert_temp(stroke_id, stroke));
    }

    let painted_curve = data.compile_rt();
    let hovered =
        pointer.and_then(|pointer| hit_curve_target(&data, &painted_curve, plot, pointer, bipolar));
    if response.clicked() || response.double_clicked() {
        ui.data_mut(|store| {
            if let Some(target) = hovered {
                store.insert_temp(selection_id, target);
            } else {
                store.remove::<CurveDragTarget>(selection_id);
            }
        });
    }
    let selected = ui
        .data(|store| store.get_temp::<CurveDragTarget>(selection_id))
        .filter(|target| match *target {
            CurveDragTarget::Knot(index) | CurveDragTarget::Segment(index) => {
                index < data.knots.len()
            }
        });
    if selected.is_none() {
        ui.data_mut(|store| store.remove::<CurveDragTarget>(selection_id));
    }
    let knot_hit = match hovered {
        Some(CurveDragTarget::Knot(index)) => Some(index),
        _ => None,
    };
    let curve_hit = match hovered {
        Some(CurveDragTarget::Segment(index)) => Some(index),
        _ => None,
    };
    let active = ui.data(|store| store.get_temp::<CurveDragTarget>(drag_id));
    let drawing = ui
        .data(|store| store.get_temp::<FreehandStroke>(stroke_id))
        .is_some();
    if pointer.is_some() {
        ui.output_mut(|output| {
            output.cursor_icon = if active.is_some() || drawing {
                egui::CursorIcon::Grabbing
            } else if knot_hit.is_some() || curve_hit.is_some() {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
    if response.hovered() || active.is_some() || selected.is_some() {
        let handle_radius = (plot.height() * 0.022).clamp(
            editor_theme::space::XXS * 0.72,
            editor_theme::space::XS * 0.62,
        );
        let emphasized_segment = active
            .or(hovered)
            .or(selected)
            .and_then(|target| match target {
                CurveDragTarget::Segment(index) => Some(index),
                CurveDragTarget::Knot(_) => None,
            });
        if let Some(index) = emphasized_segment {
            paint_curve_segment(
                ui.painter(),
                &data,
                &painted_curve,
                index,
                plot,
                bipolar,
                color.gamma_multiply(if active == Some(CurveDragTarget::Segment(index)) {
                    1.0
                } else {
                    0.72
                }),
            );
        }
        for (index, knot) in data.knots.iter().enumerate() {
            let position = knot_pos(plot, *knot, bipolar);
            let captured = active == Some(CurveDragTarget::Knot(index));
            let chosen = selected == Some(CurveDragTarget::Knot(index));
            let hot = captured || knot_hit == Some(index);
            if hot || chosen {
                ui.painter().circle_filled(
                    position,
                    handle_radius * if captured { 1.9 } else { 1.65 },
                    color.gamma_multiply(if captured {
                        0.24
                    } else if hot {
                        0.14
                    } else {
                        0.08
                    }),
                );
            }
            ui.painter().circle_filled(
                position,
                handle_radius
                    * if captured {
                        1.0
                    } else if hot || chosen {
                        0.9
                    } else {
                        0.64
                    },
                if captured {
                    editor_theme::semantic().text
                } else if hot || chosen {
                    color
                } else {
                    editor_theme::semantic().well
                },
            );
            ui.painter().circle_stroke(
                position,
                handle_radius * if hot || chosen { 1.0 } else { 0.7 },
                egui::Stroke::new(
                    if chosen {
                        editor_theme::shape::FOCUS_STROKE
                    } else {
                        editor_theme::shape::STROKE
                    },
                    color.gamma_multiply(if hot || chosen { 1.0 } else { 0.62 }),
                ),
            );
        }
        for index in 0..data.knots.len() {
            let position = curve_handle_pos(&data, &painted_curve, index, plot, bipolar);
            let captured = active == Some(CurveDragTarget::Segment(index));
            let chosen = selected == Some(CurveDragTarget::Segment(index));
            let hot = captured || curve_hit == Some(index);
            if !hot && !chosen && data.knots[index].curve.abs() <= f32::EPSILON {
                continue;
            }
            let radius = handle_radius
                * if captured {
                    0.82
                } else if hot || chosen {
                    0.72
                } else {
                    0.46
                };
            ui.painter().circle_filled(
                position,
                radius,
                if captured {
                    editor_theme::semantic().text
                } else {
                    color.gamma_multiply(if hot || chosen { 0.82 } else { 0.42 })
                },
            );
            ui.painter().circle_stroke(
                position,
                radius * 1.22,
                egui::Stroke::new(
                    if chosen {
                        editor_theme::shape::FOCUS_STROKE
                    } else {
                        editor_theme::shape::STROKE
                    },
                    color.gamma_multiply(if hot || chosen { 0.9 } else { 0.3 }),
                ),
            );
        }
        if let Some(pointer) = pointer
            .filter(|_| knot_hit.is_none() && curve_hit.is_none() && active.is_none() && !drawing)
        {
            ui.painter().circle_stroke(
                pointer,
                handle_radius * 0.68,
                egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.42)),
            );
        }
    }
    response.clone().on_hover_text(if knot_hit.is_some() {
        "Drag this point to reshape the cycle. Right-click to remove it."
    } else if curve_hit.is_some() {
        "Drag vertically to bend this segment. Double-click or right-click to reset it."
    } else {
        "Drag to draw a cycle. Double-click to add a point. Hold Alt to bypass snapping."
    });
}

fn snap_curve_point((phase, value): (f32, f32), plot: egui::Rect) -> (f32, f32) {
    let radius_x = (editor_theme::space::XS / plot.width().max(1.0)).clamp(0.008, 0.04);
    let radius_y = (editor_theme::space::XS / plot.height().max(1.0)).clamp(0.015, 0.08);
    let phase_step = 1.0 / 16.0;
    let value_step = 0.25;
    let snapped_phase = (phase / phase_step).round() * phase_step;
    let snapped_value = (value / value_step).round() * value_step;
    (
        if (phase - snapped_phase).abs() <= radius_x {
            snapped_phase.clamp(0.0, 1.0)
        } else {
            phase
        },
        if (value - snapped_value).abs() <= radius_y {
            snapped_value.clamp(-1.0, 1.0)
        } else {
            value
        },
    )
}

fn hit_curve_target(
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
) -> Option<CurveDragTarget> {
    let knot_radius = (plot.height() * 0.065).clamp(
        editor_theme::space::SM + editor_theme::space::XXS,
        editor_theme::space::LG,
    );
    if let Some((index, _)) = data
        .knots
        .iter()
        .enumerate()
        .map(|(index, knot)| (index, knot_pos(plot, *knot, bipolar).distance_sq(pointer)))
        .filter(|(_, distance)| *distance <= knot_radius * knot_radius)
        .min_by(|left, right| left.1.total_cmp(&right.1))
    {
        return Some(CurveDragTarget::Knot(index));
    }

    let segment_radius = (plot.height() * 0.055).clamp(
        editor_theme::space::SM,
        editor_theme::space::MD + editor_theme::space::XXS,
    );
    (0..data.knots.len())
        .filter_map(|index| {
            let distance = curve_segment_distance_sq(data, curve, index, plot, pointer, bipolar)?;
            (distance <= segment_radius * segment_radius).then_some((index, distance))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| CurveDragTarget::Segment(index))
}

fn curve_segment_distance_sq(
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    index: usize,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
) -> Option<f32> {
    let knot = data.knots.get(index)?;
    let start = knot.phase;
    let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
    let steps = 16;
    let mut previous = value_pos(plot, start, curve.eval(start), bipolar);
    let mut nearest = f32::INFINITY;
    for step in 1..=steps {
        let phase = (end - start).mul_add(step as f32 / steps as f32, start);
        let current = value_pos(plot, phase, curve.eval(phase), bipolar);
        nearest = nearest.min(distance_to_segment_sq(pointer, previous, current));
        previous = current;
    }
    Some(nearest)
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

fn paint_curve_segment(
    painter: &egui::Painter,
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    index: usize,
    plot: egui::Rect,
    bipolar: bool,
    color: egui::Color32,
) {
    let Some(knot) = data.knots.get(index) else {
        return;
    };
    let start = knot.phase;
    let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
    let points = (0..=24)
        .map(|step| {
            let phase = (end - start).mul_add(step as f32 / 24.0, start);
            value_pos(plot, phase, curve.eval(phase), bipolar)
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    ));
}

fn curve_handle_pos(
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    index: usize,
    plot: egui::Rect,
    bipolar: bool,
) -> egui::Pos2 {
    let current = data.knots[index].phase;
    let next = data
        .knots
        .get(index + 1)
        .map_or(data.knots[0].phase + 1.0, |knot| knot.phase);
    let phase = ((current + next) * 0.5).rem_euclid(1.0);
    value_pos(plot, phase, curve.eval(phase), bipolar)
}

fn knot_pos(plot: egui::Rect, knot: crate::wave_curve::WaveKnot, bipolar: bool) -> egui::Pos2 {
    value_pos(plot, knot.phase, knot.value, bipolar)
}

fn value_pos(plot: egui::Rect, phase: f32, value: f32, bipolar: bool) -> egui::Pos2 {
    let y = if bipolar {
        (-value * plot.height() * 0.42).mul_add(1.0, plot.center().y)
    } else {
        plot.bottom() - value.mul_add(0.5, 0.5) * plot.height() * 0.9
    };
    egui::pos2(phase.mul_add(plot.width(), plot.left()), y)
}

fn values_from_pos(plot: egui::Rect, position: egui::Pos2, bipolar: bool) -> (f32, f32) {
    let phase = ((position.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let value = if bipolar {
        (plot.center().y - position.y) / (plot.height() * 0.42)
    } else {
        ((plot.bottom() - position.y) / (plot.height() * 0.9)).mul_add(2.0, -1.0)
    }
    .clamp(-1.0, 1.0);
    (phase, value)
}

pub(crate) fn antialiasing_selector_compact(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
) {
    antialiasing_menu(ui, state, width);
}

fn antialiasing_menu(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, width: f32) {
    egui::ComboBox::from_id_salt("oscillator_antialiasing_menu")
        .selected_text(antialiasing_label(state))
        .width(width)
        .show_ui(ui, |ui| {
            let spline = true;
            if ui.selectable_label(spline, "SPLINE 4PT").clicked() {
                state.begin_edit(P::Antialiasing);
                state.set_param(P::Antialiasing, 0.5);
                state.end_edit(P::Antialiasing);
            }
        });
}

pub(crate) fn quality_selector_compact(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
) {
    quality_menu(ui, state, width);
}

fn quality_menu(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, width: f32) {
    egui::ComboBox::from_id_salt("oscillator_quality_menu")
        .selected_text(quality_label(state))
        .width(width)
        .show_ui(ui, |ui| {
            for (index, label) in ["ECO 1x", "NORMAL 2x", "HIGH 3x", "ULTRA 4x"]
                .into_iter()
                .enumerate()
            {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "the dropdown has exactly four quality modes"
                )]
                let normalized = index as f32 / 3.0;
                let selected = (state.get_param(P::Oversampling) - normalized).abs() < 0.01;
                if ui.selectable_label(selected, label).clicked() {
                    state.begin_edit(P::Oversampling);
                    state.set_param(P::Oversampling, f64::from(normalized));
                    state.end_edit(P::Oversampling);
                }
            }
        });
}

pub(crate) fn antialiasing_label(state: &PluginContext<KurvParams>) -> String {
    state.format_param(P::Antialiasing)
}

pub(crate) fn quality_label(state: &PluginContext<KurvParams>) -> String {
    state.format_param(P::Oversampling)
}
