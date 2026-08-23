//! VA-table selection and frame actions.

mod curve_editor;
mod wavetable_import;

use super::preview;
use curve_editor::edit_wave_curve_target;
use truce_core::editor::PluginContext;
use wavetable_import::{handle_wavetable_drop, paint_import_status, set_import_status};

use crate::editor_unison::vertical_selector_value;
use crate::generators::{ModuleId, OscillatorConfig, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::oscillators::{
    Antialiasing, MAX_VA_TABLE_FRAMES, PhaseWarpMode, VA_KEYFRAME_EPSILON, VaTableData,
    nearest_frame_index, position_for_frame, sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::{WaveCurveData, WaveCurveRt, fit_periodic_samples};
use crate::{KurvParams, editor_theme};

use wavetable_import::{
    list_native_tables, list_surge_tables, queue_library_import, sanitize_table_name,
    save_native_table, save_surge_table,
};

struct VaTableUi {
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
    let mut cache = preview::VaPreviewCache::load(ui, cache_id, table_state);
    let table = cache.table();
    let table_data = table_state.snapshot();
    let table_frames = table.frame_count();
    let positioned_mode = table_data.frames.is_empty() || table_data.is_positioned();
    let wave_position = (config.shape / 3.0).clamp(0.0, 1.0);
    let selection = table.select(WaveCurveRt::default(), config.custom_shape, wave_position);
    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let pencil_id = response.id.with(("wave-draw-mode", slot.index()));
    let selected_key_id = response.id.with(("wave-edit-key", slot.index()));
    let mut pencil = ui
        .data(|store| store.get_temp::<bool>(pencil_id))
        .unwrap_or(false);
    let mut edit_frame = ui.data(|store| store.get_temp::<usize>(selected_key_id));
    if pencil && positioned_mode {
        let still_on_key = edit_frame
            .and_then(|index| table_data.positions.get(index))
            .is_some_and(|position| (*position - wave_position).abs() <= VA_KEYFRAME_EPSILON);
        if !still_on_key {
            pencil = false;
            edit_frame = None;
            ui.data_mut(|store| {
                store.insert_temp(pencil_id, false);
                store.remove::<usize>(selected_key_id);
            });
            curve_editor::clear_edit_state(ui, response.id, slot.index());
        }
    }
    let plot_bounds = response.rect;
    let plot = preview::cycle_plot(plot_bounds);
    let source_dragging = crate::editor_modulation::source_drag_active(ui);
    let axis_target = ModulationRouteTarget::oscillator(
        module_id,
        slot,
        if positioned_mode {
            OscillatorControl::Shape
        } else {
            OscillatorControl::TablePosition
        },
    );
    let axis_host_binding =
        crate::editor_modulation::host_automation_binding(ui, state, axis_target);
    let editing = !source_dragging && pencil && edit_frame.is_some();
    let accent = editor_theme::palette().accent;
    preview::paint_cached_cycle(
        &mut cache,
        &painter,
        plot_bounds,
        plot,
        config,
        selection.shape,
        selection.curve,
        selection.mix,
        editing,
        source_dragging,
        accent,
    );
    cache.store(ui, cache_id);
    let imported = handle_wavetable_drop(ui, &response, &painter, plot, slot.index());
    paint_import_status(ui, &painter, plot, response.id, slot.index());
    if let Some(Ok(imported)) = imported {
        let frame_count = imported.table.frames.len();
        let imported_position = imported.table.positions.first().copied();
        table_state.replace(imported.table);
        ui.data_mut(|store| {
            store.insert_temp(pencil_id, false);
            store.remove::<usize>(selected_key_id);
        });
        curve_editor::clear_edit_state(ui, response.id, slot.index());
        if let Some(position) = imported_position {
            config.shape = position.clamp(0.0, 1.0) * 3.0;
        } else {
            let frame_count_f32 = u8::try_from(frame_count).map_or(1.0, f32::from);
            config.custom_shape = frame_count_f32.recip();
        }
        crate::editor_shell::request_structural_commit(ui);
        let message = if imported.source_frame_count > frame_count {
            format!(
                "Loaded {} source frames as {frame_count} editable VA frames",
                imported.source_frame_count
            )
        } else {
            format!("Loaded {frame_count} editable VA frame(s)")
        };
        set_import_status(ui, response.id, slot.index(), message, false);
        editor_theme::request_display_repaint(ui);
        return true;
    }
    if source_dragging {
        return false;
    }

    let exact_custom = if positioned_mode {
        table_data.frame_index_at_position(wave_position)
    } else if table_frames > 0 {
        Some(nearest_frame_index(config.custom_shape, table_frames))
    } else {
        None
    };
    let selected_frame = edit_frame
        .or(exact_custom)
        .or_else(|| {
            positioned_mode
                .then(|| table_data.nearest_positioned_frame(wave_position))
                .flatten()
        })
        .unwrap_or(0);
    let pencil_rect = pencil_toggle_rect(plot);
    let table_ui = va_table_label(
        ui,
        &painter,
        plot,
        &response,
        selected_frame,
        table_frames,
        positioned_mode,
        exact_custom.is_some() && table_frames < MAX_VA_TABLE_FRAMES,
        exact_custom.is_some(),
        axis_host_binding.as_ref().map(|(slot, _, _)| *slot),
        pencil_rect,
    );
    let mut changed = false;
    let mut axis_position_command = false;

    if pencil_toggle(ui, &painter, pencil_rect, response.id, pencil) {
        if pencil {
            pencil = false;
            edit_frame = None;
            ui.data_mut(|store| {
                store.insert_temp(pencil_id, false);
                store.remove::<usize>(selected_key_id);
            });
            curve_editor::clear_edit_state(ui, response.id, slot.index());
        } else {
            let captured =
                capture_unwarped_curve(config, selection.shape, selection.curve, selection.mix);
            let selected = if positioned_mode {
                match positioned_capture_decision(&table_data, wave_position) {
                    PositionedCaptureDecision::Canonical => {
                        set_import_status(
                            ui,
                            response.id,
                            slot.index(),
                            "Move between factory shapes to add a custom wave".to_owned(),
                            true,
                        );
                        None
                    }
                    PositionedCaptureDecision::Existing(index) => Some(index),
                    PositionedCaptureDecision::Insert => {
                        if let Some(index) =
                            table_state.insert_positioned_frame(wave_position, captured)
                        {
                            crate::editor_shell::request_structural_commit(ui);
                            changed = true;
                            Some(index)
                        } else {
                            set_import_status(
                                ui,
                                response.id,
                                slot.index(),
                                format!("VA table limit is {MAX_VA_TABLE_FRAMES} frames"),
                                true,
                            );
                            None
                        }
                    }
                }
            } else {
                match frame_capture_decision(config.custom_shape, table_frames) {
                    FrameCaptureDecision::Existing(index) => Some(index),
                    FrameCaptureDecision::Insert(index) => {
                        let inserted = table_state.insert_frame(index, captured);
                        if inserted.is_some() {
                            let new_frame_count = table_frames + 1;
                            config.custom_shape =
                                position_for_frame(inserted.unwrap_or(0), new_frame_count);
                            axis_position_command = true;
                            changed = true;
                            crate::editor_shell::request_structural_commit(ui);
                        }
                        inserted
                    }
                }
            };
            if let Some(index) = selected {
                pencil = true;
                edit_frame = Some(index);
                ui.data_mut(|store| {
                    store.insert_temp(pencil_id, true);
                    store.insert_temp(selected_key_id, index);
                });
            }
        }
    }

    if table_ui.duplicate
        && let Some(new_selected) =
            table_state.duplicate_after(selected_frame, WaveCurveData::default())
    {
        if positioned_mode {
            if let Some(position) = table_state.frame_position(new_selected) {
                config.shape = position * 3.0;
            }
        } else {
            let count = table_state.snapshot().frames.len().max(1);
            config.custom_shape = position_for_frame(new_selected, count);
        }
        pencil = false;
        edit_frame = None;
        ui.data_mut(|store| {
            store.insert_temp(pencil_id, false);
            store.remove::<usize>(selected_key_id);
        });
        curve_editor::clear_edit_state(ui, response.id, slot.index());
        axis_position_command = true;
        changed = true;
        crate::editor_shell::request_structural_commit(ui);
    }
    if table_ui.remove && table_state.remove_frame(selected_frame) {
        pencil = false;
        edit_frame = None;
        ui.data_mut(|store| {
            store.insert_temp(pencil_id, false);
            store.remove::<usize>(selected_key_id);
        });
        curve_editor::clear_edit_state(ui, response.id, slot.index());
        if !positioned_mode {
            let count = table_state.snapshot().frames.len();
            config.custom_shape = if count == 0 {
                0.0
            } else {
                position_for_frame(selected_frame.min(count - 1), count)
            };
            axis_position_command = true;
        }
        changed = true;
        crate::editor_shell::request_structural_commit(ui);
    }
    if table_ui.reset {
        table_state.replace(Default::default());
        config.custom_shape = 0.0;
        pencil = false;
        edit_frame = None;
        ui.data_mut(|store| {
            store.insert_temp(pencil_id, false);
            store.remove::<usize>(selected_key_id);
        });
        curve_editor::clear_edit_state(ui, response.id, slot.index());
        changed = true;
        crate::editor_shell::request_structural_commit(ui);
    }

    response.clone().context_menu(|ui| {
        library_menu(ui, state, table_state, slot, response.id);
        ui.separator();
        if ui.button("Initialize").clicked() {
            table_state.replace(Default::default());
            config.custom_shape = 0.0;
            pencil = false;
            edit_frame = None;
            ui.data_mut(|store| {
                store.insert_temp(pencil_id, false);
                store.remove::<usize>(selected_key_id);
            });
            curve_editor::clear_edit_state(ui, response.id, slot.index());
            changed = true;
            crate::editor_shell::request_structural_commit(ui);
            ui.close();
        }
        ui.separator();
        crate::editor_modulation::host_automation_menu(
            ui,
            state,
            axis_target,
            if positioned_mode {
                config.shape / 3.0
            } else {
                config.custom_shape
            },
        );
    });

    let pointer_over_chrome = response
        .interact_pointer_pos()
        .is_some_and(|pointer| table_ui.chrome.contains(pointer) || pencil_rect.contains(pointer));
    if pencil
        && let Some(index) = edit_frame
        && !pointer_over_chrome
    {
        edit_wave_curve_target(
            ui,
            &response,
            plot,
            table_state,
            index,
            None,
            slot.index(),
            accent,
            true,
        );
    }

    let plot_host_binding =
        crate::editor_modulation::host_automation_binding(ui, state, axis_target);
    let plot_edit_id = response.id.with("wave-position-host-edit");
    if !pencil
        && !pointer_over_chrome
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some((_, param, _)) = plot_host_binding
    {
        crate::editor::begin_edit(state, param);
        ui.data_mut(|store| store.insert_temp(plot_edit_id, true));
    }
    if !pencil
        && !pointer_over_chrome
        && response.dragged_by(egui::PointerButton::Primary)
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let before = if positioned_mode {
            config.shape / 3.0
        } else {
            config.custom_shape
        };
        let mut next = if ui.input(|input| input.modifiers.shift) {
            (before - response.drag_motion().y * 0.001).clamp(0.0, 1.0)
        } else {
            vertical_selector_value(plot, pointer)
        };
        if positioned_mode && !ui.input(|input| input.modifiers.alt) {
            let threshold = (editor_theme::space::SM / plot.height().max(1.0)).clamp(0.01, 0.035);
            next = soft_snap_wave_position(next, &table_data.positions, threshold);
        }
        if next.to_bits() != before.to_bits() {
            if positioned_mode {
                config.shape = next * 3.0;
            } else {
                config.custom_shape = next;
            }
            changed = true;
            if let Some((_, param, _)) = plot_host_binding {
                state.set_param(param, f64::from(next));
            }
        }
    }
    let plot_gesture_ended = response.drag_stopped_by(egui::PointerButton::Primary)
        || (crate::editor_controls::pointer_gesture_aborted(ui)
            && !response.is_pointer_button_down_on());
    let plot_edit_active = ui
        .data(|store| store.get_temp::<bool>(plot_edit_id))
        .unwrap_or(false);
    if plot_edit_active && plot_gesture_ended {
        ui.data_mut(|store| store.remove::<bool>(plot_edit_id));
        if let Some((_, param, _)) = plot_host_binding {
            crate::editor::end_edit(state, param);
        }
    }
    if axis_position_command && let Some((_, param, _)) = axis_host_binding {
        let value = if positioned_mode {
            config.shape / 3.0
        } else {
            config.custom_shape
        };
        crate::editor::automate(state, param, f64::from(value));
    }
    show_library_popups(ui, table_state, slot, response.id);
    changed
}

const CANONICAL_WAVE_POSITIONS: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

fn canonical_wave_position(position: f32) -> Option<f32> {
    CANONICAL_WAVE_POSITIONS
        .into_iter()
        .find(|candidate| (*candidate - position).abs() <= VA_KEYFRAME_EPSILON)
}

fn soft_snap_wave_position(position: f32, custom_positions: &[f32], threshold: f32) -> f32 {
    CANONICAL_WAVE_POSITIONS
        .iter()
        .chain(custom_positions)
        .copied()
        .map(|candidate| (candidate, (candidate - position).abs()))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= threshold)
        .map_or(position, |(candidate, _)| candidate)
}

fn pencil_toggle_rect(plot: egui::Rect) -> egui::Rect {
    let side = editor_theme::font::LABEL_SIZE + editor_theme::space::XS;
    let right = plot.right() - editor_theme::space::XXS;
    let top = plot.top() + editor_theme::space::XXS;
    egui::Rect::from_min_max(
        egui::pos2((right - side).max(plot.left()), top),
        egui::pos2(right, (top + side).min(plot.bottom())),
    )
}

fn pencil_toggle(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    parent_id: egui::Id,
    enabled: bool,
) -> bool {
    let response = ui
        .interact(rect, parent_id.with("wave-pencil"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if enabled {
            "Exit spline editing"
        } else {
            "Capture this wave for spline editing"
        });
    let palette = editor_theme::semantic();
    let active = enabled || response.is_pointer_button_down_on();
    if active || response.hovered() {
        painter.rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            if active {
                palette.primary.gamma_multiply(0.22)
            } else {
                palette.control
            },
        );
    }
    let icon = if crate::editor_widgets::icon_font_ready(ui) {
        egui_phosphor::regular::PENCIL_SIMPLE
    } else {
        "✎"
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(rect.height() * 0.62),
        if enabled {
            palette.primary
        } else if response.hovered() {
            palette.text
        } else {
            palette.text_muted
        },
    );
    response.clicked()
}

fn va_table_label(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    plot: egui::Rect,
    parent: &egui::Response,
    selected_frame: usize,
    frame_count: usize,
    positioned: bool,
    can_duplicate: bool,
    can_remove: bool,
    host_slot: Option<usize>,
    pencil_rect: egui::Rect,
) -> VaTableUi {
    let row_height = editor_theme::font::LABEL_SIZE + editor_theme::space::XS;
    let action_size = row_height;
    let gap = editor_theme::compact_gap(ui);
    let action_width = action_size * 3.0 + gap * 2.0;
    let right = (pencil_rect.left() - gap).max(plot.left());
    let top = plot.top() + editor_theme::space::XXS;
    let label_text = if positioned {
        format!("WAVE {}", frame_count + CANONICAL_WAVE_POSITIONS.len())
    } else if frame_count == 0 {
        "VA".to_owned()
    } else {
        format!("VA {}/{}", selected_frame + 1, frame_count)
    };
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
    let response = ui.interact(
        label_rect,
        parent.id.with("va-table-label"),
        egui::Sense::hover(),
    );
    let palette = editor_theme::semantic();
    painter.text(
        label_rect.right_center() - egui::vec2(editor_theme::space::XXS, 0.0),
        egui::Align2::RIGHT_CENTER,
        label_text,
        label_font.clone(),
        if response.hovered() {
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
            "Initialize the VA table",
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
        chrome: action_chrome.union(label_rect).union(pencil_rect),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PositionedCaptureDecision {
    Canonical,
    Existing(usize),
    Insert,
}

fn positioned_capture_decision(table: &VaTableData, position: f32) -> PositionedCaptureDecision {
    if canonical_wave_position(position).is_some() {
        PositionedCaptureDecision::Canonical
    } else if let Some(index) = table.frame_index_at_position(position) {
        PositionedCaptureDecision::Existing(index)
    } else {
        PositionedCaptureDecision::Insert
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameCaptureDecision {
    Existing(usize),
    Insert(usize),
}

fn frame_capture_decision(position: f32, frame_count: usize) -> FrameCaptureDecision {
    if frame_count == 0 {
        return FrameCaptureDecision::Insert(0);
    }
    let scaled = position.clamp(0.0, 1.0) * frame_count as f32;
    let nearest = scaled.round();
    if nearest >= 1.0 && (scaled - nearest).abs() <= 0.001 {
        FrameCaptureDecision::Existing((nearest as usize - 1).min(frame_count - 1))
    } else {
        FrameCaptureDecision::Insert((scaled.floor() as usize).min(frame_count))
    }
}

/// Materializes the current base/table morph without baking preview phase warp
/// or display-frequency antialiasing into the editable spline frame.
fn capture_unwarped_curve(
    config: &OscillatorConfig,
    shape: f32,
    curve: WaveCurveRt,
    mix: f32,
) -> WaveCurveData {
    const SAMPLES: usize = 256;
    let samples = (0..SAMPLES)
        .map(|index| {
            sample_custom_shape_with_antialiasing_warped(
                shape.clamp(0.0, 3.0),
                f64::from(index as f32 / SAMPLES as f32),
                0.0,
                config.pulse_width.clamp(0.03, 0.97),
                Antialiasing::SplineOptimized,
                PhaseWarpMode::None,
                0.0,
                curve,
                mix,
            )
        })
        .collect::<Vec<_>>();
    fit_periodic_samples(&samples)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryPopup {
    Save,
    Load,
    ImportSurge,
}

fn library_popup_id(parent: egui::Id, oscillator: usize) -> egui::Id {
    parent.with(("va-library-popup", oscillator))
}

fn save_name_id(parent: egui::Id, oscillator: usize) -> egui::Id {
    parent.with(("va-library-name", oscillator))
}

fn library_listing_id(parent: egui::Id, oscillator: usize, native: bool) -> egui::Id {
    parent.with(("va-library-listing", oscillator, native))
}

fn library_menu(
    ui: &mut egui::Ui,
    _state: &PluginContext<KurvParams>,
    table: &crate::oscillators::VaTableState,
    slot: OscillatorSlot,
    parent: egui::Id,
) {
    if ui.button("Save VA Table…").clicked() {
        ui.data_mut(|store| {
            store.insert_temp(library_popup_id(parent, slot.index()), LibraryPopup::Save);
            store.insert_temp(
                save_name_id(parent, slot.index()),
                format!("osc{}", slot.index() + 1),
            );
        });
        ui.close();
    }
    if ui.button("Load VA Table…").clicked() {
        ui.data_mut(|store| {
            store.insert_temp(library_popup_id(parent, slot.index()), LibraryPopup::Load);
        });
        ui.close();
    }
    ui.separator();
    if ui.button("Import Surge .wt…").clicked() {
        ui.data_mut(|store| {
            store.insert_temp(
                library_popup_id(parent, slot.index()),
                LibraryPopup::ImportSurge,
            );
        });
        ui.close();
    }
    if ui.button("Export Surge .wt").clicked() {
        let snapshot = table.snapshot();
        match save_surge_table(&format!("osc{}", slot.index() + 1), &snapshot) {
            Ok(path) => set_import_status(
                ui,
                parent,
                slot.index(),
                format!("Exported {}", path.display()),
                false,
            ),
            Err(error) => set_import_status(ui, parent, slot.index(), error, true),
        }
        ui.close();
    }
}

fn show_library_popups(
    ui: &mut egui::Ui,
    table: &crate::oscillators::VaTableState,
    slot: OscillatorSlot,
    parent: egui::Id,
) {
    let popup_id = library_popup_id(parent, slot.index());
    let Some(kind) = ui.data(|store| store.get_temp::<LibraryPopup>(popup_id)) else {
        return;
    };
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::SM);
    let width = 220.0_f32.min(screen.width());
    let area = egui::Area::new(popup_id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(
            screen.center().x - width * 0.5,
            screen.center().y - editor_theme::space::LG * 4.0,
        ));
    let mut close = false;
    let popup = area.show(ui.ctx(), |ui| {
        egui::Frame::new()
            .fill(editor_theme::semantic().surface)
            .stroke(egui::Stroke::new(
                editor_theme::shape::STROKE,
                editor_theme::semantic().grid,
            ))
            .inner_margin(egui::Margin::same(editor_theme::space::SM as i8))
            .show(ui, |ui| {
                ui.set_min_width(width);
                match kind {
                    LibraryPopup::Save => {
                        ui.label(
                            egui::RichText::new("SAVE VA TABLE")
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                        );
                        let mut name = ui.data(|store| {
                            store
                                .get_temp::<String>(save_name_id(parent, slot.index()))
                                .unwrap_or_else(|| format!("osc{}", slot.index() + 1))
                        });
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut name)
                                .desired_width(width)
                                .font(editor_theme::font::label()),
                        );
                        if edit.changed() {
                            ui.data_mut(|store| {
                                store.insert_temp(save_name_id(parent, slot.index()), name.clone());
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("SAVE").clicked() {
                                match save_native_table(
                                    &sanitize_table_name(&name),
                                    &table.snapshot(),
                                ) {
                                    Ok(path) => {
                                        set_import_status(
                                            ui,
                                            parent,
                                            slot.index(),
                                            format!("Saved {}", path.display()),
                                            false,
                                        );
                                        close = true;
                                    }
                                    Err(error) => {
                                        set_import_status(ui, parent, slot.index(), error, true);
                                    }
                                }
                            }
                            if ui.button("CANCEL").clicked() {
                                close = true;
                            }
                        });
                    }
                    LibraryPopup::Load | LibraryPopup::ImportSurge => {
                        let (title, empty, native) = match kind {
                            LibraryPopup::Load => (
                                "LOAD VA TABLE",
                                "No saved VA tables in KURV/Wavetables.",
                                true,
                            ),
                            LibraryPopup::ImportSurge => (
                                "IMPORT SURGE .WT",
                                "No .wt files in KURV/Wavetables. Drop one on the plot.",
                                false,
                            ),
                            LibraryPopup::Save => unreachable!(),
                        };
                        let listing_id = library_listing_id(parent, slot.index(), native);
                        let tables = ui
                            .data(|store| {
                                store.get_temp::<Result<Vec<(String, std::path::PathBuf)>, String>>(
                                    listing_id,
                                )
                            })
                            .unwrap_or_else(|| {
                                let tables = if native {
                                    list_native_tables()
                                } else {
                                    list_surge_tables()
                                };
                                ui.data_mut(|store| store.insert_temp(listing_id, tables.clone()));
                                tables
                            });
                        ui.label(
                            egui::RichText::new(title)
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                        );
                        match tables {
                            Ok(tables) if tables.is_empty() => {
                                ui.label(empty);
                            }
                            Ok(tables) => {
                                for (name, path) in tables {
                                    if ui.button(name).clicked() {
                                        if let Some(error) =
                                            queue_library_import(ui, parent, slot.index(), path)
                                        {
                                            set_import_status(
                                                ui,
                                                parent,
                                                slot.index(),
                                                error,
                                                true,
                                            );
                                        }
                                        close = true;
                                    }
                                }
                            }
                            Err(error) => {
                                ui.label(error);
                            }
                        }
                        if ui.button("CANCEL").clicked() {
                            close = true;
                        }
                    }
                }
            });
    });
    if close
        || ui.input(|input| {
            input.key_pressed(egui::Key::Escape)
                || (input.pointer.primary_clicked()
                    && input
                        .pointer
                        .latest_pos()
                        .is_some_and(|pointer| !popup.response.rect.contains(pointer)))
        })
    {
        ui.data_mut(|store| {
            store.remove::<LibraryPopup>(popup_id);
            store.remove::<Result<Vec<(String, std::path::PathBuf)>, String>>(library_listing_id(
                parent,
                slot.index(),
                true,
            ));
            store.remove::<Result<Vec<(String, std::path::PathBuf)>, String>>(library_listing_id(
                parent,
                slot.index(),
                false,
            ));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrameCaptureDecision, PositionedCaptureDecision, capture_unwarped_curve,
        frame_capture_decision, positioned_capture_decision,
    };
    use crate::generators::OscillatorConfig;
    use crate::oscillators::VaTableData;
    use crate::wave_curve::{WaveCurveData, WaveCurveRt};

    #[test]
    fn edit_mode_reuses_exact_frames_and_inserts_between_them() {
        assert_eq!(
            frame_capture_decision(0.0, 0),
            FrameCaptureDecision::Insert(0)
        );
        assert_eq!(
            frame_capture_decision(0.5, 2),
            FrameCaptureDecision::Existing(0)
        );
        assert_eq!(
            frame_capture_decision(1.0, 2),
            FrameCaptureDecision::Existing(1)
        );
        assert_eq!(
            frame_capture_decision(0.25, 2),
            FrameCaptureDecision::Insert(0)
        );
        assert_eq!(
            frame_capture_decision(0.75, 2),
            FrameCaptureDecision::Insert(1)
        );
    }

    #[test]
    fn draw_between_factory_anchors_creates_then_reuses_the_fifth_key() {
        let empty = VaTableData::default();
        assert_eq!(
            positioned_capture_decision(&empty, 0.5),
            PositionedCaptureDecision::Insert
        );
        let five_shapes = VaTableData {
            frames: vec![WaveCurveData::default()],
            positions: vec![0.5],
        };
        assert_eq!(
            positioned_capture_decision(&five_shapes, 0.5),
            PositionedCaptureDecision::Existing(0)
        );
        assert_eq!(
            positioned_capture_decision(&five_shapes, 1.0 / 3.0),
            PositionedCaptureDecision::Canonical
        );
    }

    #[test]
    fn canonical_saw_capture_uses_two_corner_points() {
        let mut config = OscillatorConfig::default();
        config.shape = 2.0;
        let captured = capture_unwarped_curve(&config, config.shape, WaveCurveRt::default(), 0.0);
        assert_eq!(captured.knots.len(), 2, "{:#?}", captured.knots);
        assert_eq!(captured.knots[0].value, -1.0);
        assert!(captured.knots[1].value > 0.98);
    }

    #[test]
    fn spline_capture_does_not_bake_phase_warp() {
        let mut warped = OscillatorConfig::default();
        warped.shape = 2.4;
        warped.pulse_width = 0.31;
        warped.phase_warp_mode = 3;
        warped.phase_warp_amount = 1.0;
        let curve = WaveCurveRt::default();

        let with_runtime_warp = capture_unwarped_curve(&warped, warped.shape, curve, 0.4);
        warped.phase_warp_mode = 0;
        warped.phase_warp_amount = 0.0;
        let without_runtime_warp = capture_unwarped_curve(&warped, warped.shape, curve, 0.4);

        assert_eq!(with_runtime_warp, without_runtime_warp);
    }
}
