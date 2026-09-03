//! VA-table selection and frame actions.

mod curve_editor;
mod wavetable_import;

use super::preview;
use curve_editor::edit_wave_curve_target;
use truce_core::editor::PluginContext;
use wavetable_import::{handle_wavetable_drop, paint_import_status, set_import_status};

use crate::editor_unison::vertical_selector_value;
use crate::generators::{ModuleId, OscillatorConfig, OscillatorEngineKind, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::oscillators::{
    Antialiasing, DEFAULT_VA_FUNCTION, MAX_VA_TABLE_FRAMES, PhaseWarpMode, VA_KEYFRAME_EPSILON,
    VaTableData, compile_va_function, nearest_frame_index, position_for_frame,
    sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::{WaveCurveData, WaveCurveRt, fit_periodic_samples};
use crate::{KurvParams, editor_theme};

use wavetable_import::{
    list_native_tables, queue_library_import, sanitize_table_name, save_native_table,
};

struct VaTableUi {
    chrome: egui::Rect,
    duplicate: bool,
    remove: bool,
    reset: bool,
    exit_edit: bool,
    open_library: bool,
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
    let mut table_data = table_state.snapshot();
    let table_frames = table.frame_count();
    let mut positioned_mode = table_data.frames.is_empty() || table_data.is_positioned();
    let wave_position = (config.shape / 3.0).clamp(0.0, 1.0);
    let selection = table.select(WaveCurveRt::default(), config.custom_shape, wave_position);
    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let pencil_id = response.id.with(("wave-draw-mode", slot.index()));
    let function_id = response.id.with(("wave-function-mode", slot.index()));
    let selected_key_id = response.id.with(("wave-edit-key", slot.index()));
    let mut pencil = ui
        .data(|store| store.get_temp::<bool>(pencil_id))
        .unwrap_or(false);
    let mut edit_frame = ui.data(|store| store.get_temp::<usize>(selected_key_id));
    let mut function_active = ui
        .data(|store| store.get_temp::<bool>(function_id))
        .unwrap_or(false);
    if (pencil || function_active) && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
        ui.data_mut(|store| store.insert_temp(function_id, false));
        pencil = false;
        function_active = false;
        edit_frame = None;
    }
    if pencil && positioned_mode {
        let valid_edit_frame = edit_frame.is_some_and(|index| index < table_data.frames.len());
        if !valid_edit_frame {
            pencil = false;
            edit_frame = None;
            ui.data_mut(|store| {
                store.insert_temp(pencil_id, false);
                store.remove::<usize>(selected_key_id);
            });
            curve_editor::clear_edit_state(ui, response.id, slot.index());
        }
    }
    if function_active
        && edit_frame.is_none_or(|index| {
            index >= table_data.frames.len()
                || table_data.functions.get(index).is_none_or(String::is_empty)
        })
    {
        function_active = false;
        edit_frame = None;
        ui.data_mut(|store| {
            store.insert_temp(function_id, false);
            store.remove::<usize>(selected_key_id);
        });
    }
    let plot_bounds = response.rect;
    let header_height =
        (editor_theme::font::LABEL_SIZE + editor_theme::space::SM).min(height * 0.2);
    let header = egui::Rect::from_min_max(
        plot_bounds.left_top(),
        egui::pos2(plot_bounds.right(), plot_bounds.top() + header_height),
    );
    let graph_bounds = egui::Rect::from_min_max(
        egui::pos2(plot_bounds.left(), header.bottom()),
        plot_bounds.right_bottom(),
    );
    let plot = preview::cycle_plot(graph_bounds);
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
    let editing = !source_dragging && pencil && edit_frame.is_some() && !function_active;
    let accent = editor_theme::palette().accent;
    let audio_rate_routes =
        crate::editor_modulation::generator_preview_routes(ui, state, module_id, slot)
            .into_iter()
            .filter_map(|(source, control, amount)| {
                let source_slot = OscillatorSlot::from_index(usize::from(source))?;
                let source_config = state.generator_stack.oscillator_config(source_slot);
                if !source_config.enabled || source_config.engine != OscillatorEngineKind::Va {
                    return None;
                }
                let source_table_state = state.generator_stack.va_table(source_slot);
                let table_generation = source_table_state.history_generation();
                let source_table = source_table_state
                    .try_table_rt(0)
                    .map(|(_, table)| table)
                    .unwrap_or_else(|| source_table_state.snapshot().compile_rt());
                let source_shape = source_config.shape.clamp(0.0, 3.0);
                let source_selection = source_table.select(
                    WaveCurveRt::default(),
                    source_config.custom_shape,
                    source_shape / 3.0,
                );
                Some(preview::AudioRatePreviewRoute {
                    source,
                    table_generation,
                    control,
                    amount,
                    config: source_config,
                    shape: source_shape,
                    curve: source_selection.curve,
                    mix: source_selection.mix,
                })
            })
            .collect::<Vec<_>>();
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
        &audio_rate_routes,
        accent,
    );
    cache.store(ui, cache_id);
    let imported = handle_wavetable_drop(ui, &response, &painter, plot, slot.index());
    paint_import_status(ui, &painter, plot, response.id, slot.index());
    if let Some(Ok(imported)) = imported {
        let frame_count = imported.table.frames.len();
        let analytic = imported
            .table
            .functions
            .iter()
            .any(|expression| !expression.is_empty());
        let positioned = imported.table.is_positioned();
        table_state.replace(imported.table);
        if positioned {
            evenly_space_positioned_frames(table_state);
        }
        let imported_position = table_state.frame_position(0);
        ui.data_mut(|store| {
            store.insert_temp(pencil_id, false);
            store.insert_temp(function_id, false);
            store.remove::<usize>(selected_key_id);
        });
        curve_editor::clear_edit_state(ui, response.id, slot.index());
        if let Some(position) = imported_position {
            config.shape = position.clamp(0.0, 1.0) * 3.0;
        } else if frame_count > 0 {
            let frame_count_f32 = u8::try_from(frame_count).map_or(1.0, f32::from);
            config.custom_shape = frame_count_f32.recip();
        } else {
            config.custom_shape = 0.0;
        }
        crate::editor_shell::request_structural_commit(ui);
        let message = if analytic {
            "Loaded VA function".to_owned()
        } else if imported.source_frame_count > frame_count {
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
    let function_rect = pencil_rect.translate(egui::vec2(
        -pencil_rect.width() - editor_theme::space::XXS,
        0.0,
    ));
    let function_editor_rect = egui::Rect::from_min_max(
        egui::pos2(
            plot.left(),
            plot.bottom() - (editor_theme::font::LABEL_SIZE * 2.4),
        ),
        plot.right_bottom(),
    );
    let table_name = current_table_name(ui, response.id, slot, table_frames > 0);
    let table_ui = va_table_label(
        ui,
        &painter,
        header,
        &response,
        &table_name,
        editing,
        exact_custom.is_some() && table_frames < MAX_VA_TABLE_FRAMES,
        exact_custom.is_some(),
        axis_host_binding.as_ref().map(|(slot, _, _)| *slot),
        pencil_rect,
    );
    let mut changed = false;
    let mut axis_position_command = false;

    if table_ui.open_library {
        ui.data_mut(|store| {
            store.insert_temp(
                library_popup_id(response.id, slot.index()),
                LibraryPopup::Load,
            );
            store.remove::<Result<Vec<(String, std::path::PathBuf)>, String>>(library_listing_id(
                response.id,
                slot.index(),
            ));
        });
    }

    if table_ui.exit_edit {
        leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
        pencil = false;
        edit_frame = None;
    }

    if table_ui.duplicate
        && let Some(new_selected) =
            table_state.duplicate_after(selected_frame, WaveCurveData::default())
    {
        if positioned_mode {
            evenly_space_positioned_frames(table_state);
            if let Some(position) = table_state.frame_position(new_selected) {
                config.shape = position * 3.0;
            }
        } else {
            let count = table_state.snapshot().frames.len().max(1);
            config.custom_shape = position_for_frame(new_selected, count);
        }
        pencil = false;
        edit_frame = None;
        leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
        axis_position_command = true;
        changed = true;
        crate::editor_shell::request_structural_commit(ui);
    }
    if table_ui.remove && table_state.remove_frame(selected_frame) {
        if positioned_mode {
            evenly_space_positioned_frames(table_state);
        }
        pencil = false;
        edit_frame = None;
        leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
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
        set_current_table_name(ui, response.id, slot, "INIT".to_owned());
        config.custom_shape = 0.0;
        pencil = false;
        edit_frame = None;
        function_active = false;
        ui.data_mut(|store| store.insert_temp(function_id, false));
        leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
        changed = true;
        crate::editor_shell::request_structural_commit(ui);
    }

    response.clone().context_menu(|ui| {
        if pencil && ui.button("EXIT DRAW MODE").clicked() {
            leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
            pencil = false;
            ui.close();
        }
        if ui.button("SAVE VA TABLE…").clicked() {
            let snapshot = table_state.snapshot();
            let current_name =
                current_table_name(ui, response.id, slot, !snapshot.frames.is_empty());
            ui.data_mut(|store| {
                store.insert_temp(
                    library_popup_id(response.id, slot.index()),
                    LibraryPopup::Save,
                );
                store.insert_temp(save_name_id(response.id, slot.index()), current_name);
            });
            ui.close();
        }
        ui.separator();
        if ui.button("Initialize").clicked() {
            table_state.replace(Default::default());
            set_current_table_name(ui, response.id, slot, "INIT".to_owned());
            config.custom_shape = 0.0;
            pencil = false;
            edit_frame = None;
            function_active = false;
            ui.data_mut(|store| store.insert_temp(function_id, false));
            leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
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

    let pointer_over_chrome = response.interact_pointer_pos().is_some_and(|pointer| {
        table_ui.chrome.contains(pointer)
            || pencil_rect.contains(pointer)
            || function_rect.contains(pointer)
            || (function_active && function_editor_rect.contains(pointer))
    });
    if pencil
        && !function_active
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

    if function_editor(
        ui,
        &painter,
        function_editor_rect,
        response.id,
        table_state,
        &table_data,
        function_active,
        edit_frame,
    ) {
        changed = true;
        crate::editor_shell::request_structural_commit(ui);
    }

    if function_toggle(ui, &painter, function_rect, response.id, function_active) {
        if function_active {
            ui.data_mut(|store| store.insert_temp(function_id, false));
            leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
        } else {
            make_positioned(table_state);
            table_data = table_state.snapshot();
            positioned_mode = true;
            let captured =
                capture_unwarped_curve(config, selection.shape, selection.curve, selection.mix);
            let selected = if let Some(index) = table_data.frame_index_at_position(wave_position) {
                Some(index)
            } else {
                let inserted = table_state.insert_positioned_frame(wave_position, captured);
                if inserted.is_some() {
                    evenly_space_positioned_frames(table_state);
                }
                inserted
            };
            if let Some(index) = selected {
                table_state.edit(|data| {
                    if data.functions[index].is_empty() {
                        data.functions[index] = DEFAULT_VA_FUNCTION.to_owned();
                    }
                });
                if let Some(position) = table_state.frame_position(index) {
                    config.shape = position * 3.0;
                }
                axis_position_command = true;
                pencil = false;
                ui.data_mut(|store| {
                    store.insert_temp(function_id, true);
                    store.insert_temp(pencil_id, false);
                    store.insert_temp(selected_key_id, index);
                });
                crate::editor_shell::request_structural_commit(ui);
            } else {
                set_import_status(
                    ui,
                    response.id,
                    slot.index(),
                    format!("VA table limit is {MAX_VA_TABLE_FRAMES} frames"),
                    true,
                );
            }
        }
        changed = true;
    }

    // Register this after the full graph editor so it wins overlapping hit tests.
    if pencil_toggle(ui, &painter, pencil_rect, response.id, pencil) {
        if pencil {
            pencil = false;
            leave_curve_edit_mode(ui, response.id, slot, pencil_id, selected_key_id);
        } else {
            make_positioned(table_state);
            table_data = table_state.snapshot();
            positioned_mode = true;
            let captured =
                capture_unwarped_curve(config, selection.shape, selection.curve, selection.mix);
            let selected = if let Some(index) = table_data.frame_index_at_position(wave_position) {
                table_state.replace_frame(index, captured).then_some(index)
            } else {
                let inserted = table_state.insert_positioned_frame(wave_position, captured);
                if inserted.is_some() {
                    evenly_space_positioned_frames(table_state);
                }
                inserted
            };
            if let Some(index) = selected {
                if let Some(position) = table_state.frame_position(index) {
                    config.shape = position * 3.0;
                }
                pencil = true;
                ui.data_mut(|store| {
                    store.insert_temp(pencil_id, true);
                    store.insert_temp(function_id, false);
                    store.insert_temp(selected_key_id, index);
                });
                axis_position_command = true;
                changed = true;
                crate::editor_shell::request_structural_commit(ui);
                editor_theme::request_display_repaint(ui);
            } else {
                set_import_status(
                    ui,
                    response.id,
                    slot.index(),
                    format!("VA table limit is {MAX_VA_TABLE_FRAMES} frames"),
                    true,
                );
            }
        }
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

fn current_table_name(
    ui: &egui::Ui,
    parent: egui::Id,
    slot: OscillatorSlot,
    has_content: bool,
) -> String {
    ui.data(|store| {
        store
            .get_temp::<String>(table_name_id(parent, slot.index()))
            .unwrap_or_else(|| {
                if has_content {
                    "UNTITLED".to_owned()
                } else {
                    "INIT".to_owned()
                }
            })
    })
}

fn leave_curve_edit_mode(
    ui: &egui::Ui,
    response_id: egui::Id,
    slot: OscillatorSlot,
    pencil_id: egui::Id,
    selected_key_id: egui::Id,
) {
    ui.data_mut(|store| {
        store.insert_temp(pencil_id, false);
        store.remove::<usize>(selected_key_id);
    });
    curve_editor::clear_edit_state(ui, response_id, slot.index());
}

fn set_current_table_name(ui: &egui::Ui, parent: egui::Id, slot: OscillatorSlot, name: String) {
    ui.data_mut(|store| store.insert_temp(table_name_id(parent, slot.index()), name));
}

fn evenly_space_positioned_frames(table: &crate::oscillators::VaTableState) {
    table.edit(|data| {
        if !data.is_positioned() {
            return;
        }
        let count = data.frames.len() as f32;
        for (index, position) in data.positions.iter_mut().enumerate() {
            *position = (index as f32 + 0.5) / count;
        }
    });
}

fn make_positioned(table: &crate::oscillators::VaTableState) {
    table.edit(|data| {
        if data.frames.is_empty() || data.is_positioned() {
            return;
        }
        let count = data.frames.len() as f32;
        data.positions = (0..data.frames.len())
            .map(|index| (index as f32 + 0.5) / count)
            .collect();
    });
}

const CANONICAL_WAVE_POSITIONS: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

fn soft_snap_wave_position(position: f32, custom_positions: &[f32], threshold: f32) -> f32 {
    let Some((candidate, distance)) = CANONICAL_WAVE_POSITIONS
        .iter()
        .chain(custom_positions)
        .copied()
        .map(|candidate| (candidate, (candidate - position).abs()))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= threshold)
    else {
        return position;
    };
    if distance <= VA_KEYFRAME_EPSILON {
        candidate
    } else {
        let pull = (1.0 - distance / threshold).powi(2) * 0.65;
        egui::lerp(position..=candidate, pull)
    }
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

fn function_toggle(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    parent_id: egui::Id,
    enabled: bool,
) -> bool {
    let response = ui
        .interact(rect, parent_id.with("wave-function"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if enabled {
            "Exit function editing"
        } else {
            "Build the wave as f(phase, WAVE)"
        });
    let palette = editor_theme::semantic();
    if enabled || response.hovered() {
        painter.rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            if enabled {
                palette.primary.gamma_multiply(0.22)
            } else {
                palette.control
            },
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "ƒ",
        editor_theme::font::title(),
        if enabled {
            palette.primary
        } else {
            palette.text_muted
        },
    );
    response.clicked()
        || (response.has_focus()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
            }))
}

fn function_editor(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    parent_id: egui::Id,
    table: &crate::oscillators::VaTableState,
    snapshot: &VaTableData,
    enabled: bool,
    frame_index: Option<usize>,
) -> bool {
    let Some(frame_index) = frame_index.filter(|_| enabled) else {
        return false;
    };
    let palette = editor_theme::semantic();
    painter.rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        palette.well.gamma_multiply(0.94),
    );
    let field_rect = rect.shrink2(egui::vec2(
        editor_theme::space::XS,
        editor_theme::space::XXS,
    ));
    let draft_id = parent_id.with(("function-expression-draft", frame_index));
    let error_id = parent_id.with(("function-expression-error", frame_index));
    let expression = snapshot
        .functions
        .get(frame_index)
        .cloned()
        .unwrap_or_default();
    let mut draft = ui
        .data(|store| store.get_temp::<String>(draft_id))
        .unwrap_or(expression.clone());
    let error = ui.data(|store| store.get_temp::<String>(error_id));
    let mut field = ui.put(
        field_rect,
        egui::TextEdit::singleline(&mut draft)
            .id_salt(parent_id.with("function-expression"))
            .font(editor_theme::font::value())
            .desired_width(field_rect.width())
            .frame(egui::Frame::NONE)
            .hint_text("sin(tau*x)   x=phase, w=WAVE"),
    );
    if let Some(error) = &error {
        field = field.on_hover_text(error);
        painter.rect_stroke(
            field_rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::STROKE, palette.danger),
            egui::StrokeKind::Inside,
        );
    }
    if field.changed() {
        ui.data_mut(|store| store.insert_temp(draft_id, draft.clone()));
        match compile_va_function(&draft) {
            Ok(_) => {
                table.edit(|data| data.functions[frame_index].clone_from(&draft));
                ui.data_mut(|store| store.remove::<String>(error_id));
                return true;
            }
            Err(error) => {
                ui.data_mut(|store| store.insert_temp(error_id, error));
            }
        }
    }
    false
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
    header: egui::Rect,
    parent: &egui::Response,
    table_name: &str,
    editing: bool,
    can_duplicate: bool,
    can_remove: bool,
    host_slot: Option<usize>,
    pencil_rect: egui::Rect,
) -> VaTableUi {
    let row_height = editor_theme::font::LABEL_SIZE + editor_theme::space::XS;
    let action_size = row_height;
    let gap = editor_theme::compact_gap(ui);
    let action_count = if editing { 4.0 } else { 3.0 };
    let action_width = action_size * action_count + gap * (action_count - 1.0);
    let right = header.right() - editor_theme::space::XXS;
    let top = header.top() + editor_theme::space::XXS;
    let label_text = table_name.to_owned();
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
        egui::pos2((right - label_width).max(header.left()), top),
        egui::pos2(right, (top + row_height).min(header.bottom())),
    );
    let action_left = (label_rect.left() - gap - action_width).max(header.left());
    let response = ui
        .interact(
            label_rect,
            parent.id.with("va-table-label"),
            egui::Sense::click(),
        )
        .on_hover_text("Open VA table library");
    let palette = editor_theme::semantic();
    painter.text(
        egui::pos2(
            header.left() + editor_theme::space::XXS,
            top + row_height * 0.5,
        ),
        egui::Align2::LEFT_CENTER,
        "VA TABLE",
        label_font.clone(),
        palette.text_muted,
    );
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
    painter.line_segment(
        [header.left_bottom(), header.right_bottom()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.52),
        ),
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

    let actions_fit = label_rect.left() - header.left() >= action_width + gap;
    let action_region = egui::Rect::from_min_max(
        egui::pos2(action_left, top),
        egui::pos2(label_rect.right(), label_rect.bottom()),
    );
    let show_actions = actions_fit
        && (editing
            || ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|pointer| action_region.contains(pointer)));
    let mut duplicate = false;
    let mut remove = false;
    let mut reset = false;
    let mut exit_edit = false;
    if show_actions {
        let action_rect = |index: usize| {
            let left = action_left + index as f32 * (action_size + gap);
            egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(action_size, action_size))
        };
        let mut action_index = 0;
        if editing {
            exit_edit = curve_action(
                ui,
                action_rect(action_index),
                parent.id.with("exit-va-frame-edit"),
                "OK",
                "Exit VA frame editing",
                true,
            );
            action_index += 1;
        }
        duplicate = curve_action(
            ui,
            action_rect(action_index),
            parent.id.with("duplicate-va-frame"),
            "+",
            "Duplicate this VA frame",
            can_duplicate,
        );
        action_index += 1;
        remove = curve_action(
            ui,
            action_rect(action_index),
            parent.id.with("remove-va-frame"),
            "−",
            "Remove this VA frame",
            can_remove,
        );
        action_index += 1;
        reset = curve_action(
            ui,
            action_rect(action_index),
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
        chrome: action_chrome
            .union(label_rect)
            .union(header)
            .union(pencil_rect),
        duplicate,
        remove,
        reset,
        exit_edit,
        open_library: response.clicked(),
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
}

fn library_popup_id(parent: egui::Id, oscillator: usize) -> egui::Id {
    parent.with(("va-library-popup", oscillator))
}

fn save_name_id(parent: egui::Id, oscillator: usize) -> egui::Id {
    parent.with(("va-library-name", oscillator))
}

fn table_name_id(parent: egui::Id, oscillator: usize) -> egui::Id {
    parent.with(("va-table-name", oscillator))
}

fn library_listing_id(parent: egui::Id, oscillator: usize) -> egui::Id {
    parent.with(("va-library-listing", oscillator))
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
    let width = 260.0_f32.min(screen.width());
    let content_width = width - editor_theme::space::SM * 2.0;
    let area = egui::Area::new(popup_id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(screen.right() - width, screen.top()));
    let mut close = false;
    let mut next_kind = None;
    area.show(ui.ctx(), |ui| {
        egui::Frame::new()
            .fill(editor_theme::semantic().surface)
            .stroke(egui::Stroke::new(
                editor_theme::shape::STROKE,
                editor_theme::semantic().grid,
            ))
            .inner_margin(egui::Margin::same(editor_theme::space::SM as i8))
            .show(ui, |ui| {
                ui.set_min_width(content_width);
                ui.set_max_height(screen.height());
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
                                .desired_width(content_width)
                                .font(editor_theme::font::label()),
                        );
                        if edit.changed() {
                            ui.data_mut(|store| {
                                store.insert_temp(save_name_id(parent, slot.index()), name.clone());
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("SAVE").clicked() {
                                let saved_name = sanitize_table_name(&name);
                                match save_native_table(&saved_name, &table.snapshot()) {
                                    Ok(path) => {
                                        set_current_table_name(ui, parent, slot, saved_name);
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
                    LibraryPopup::Load => {
                        let listing_id = library_listing_id(parent, slot.index());
                        let tables = ui
                            .data(|store| {
                                store.get_temp::<Result<Vec<(String, std::path::PathBuf)>, String>>(
                                    listing_id,
                                )
                            })
                            .unwrap_or_else(|| {
                                let tables = list_native_tables();
                                ui.data_mut(|store| store.insert_temp(listing_id, tables.clone()));
                                tables
                            });
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("VA TABLES")
                                    .font(editor_theme::font::caption())
                                    .color(editor_theme::semantic().text_muted),
                            );
                            if ui.button("SAVE AS…").clicked() {
                                next_kind = Some(LibraryPopup::Save);
                                let snapshot = table.snapshot();
                                let current_name = current_table_name(
                                    ui,
                                    parent,
                                    slot,
                                    !snapshot.frames.is_empty(),
                                );
                                ui.data_mut(|store| {
                                    store.insert_temp(
                                        save_name_id(parent, slot.index()),
                                        current_name,
                                    );
                                });
                                close = true;
                            }
                        });
                        match tables {
                            Ok(tables) if tables.is_empty() => {
                                ui.label("No saved VA tables yet.");
                            }
                            Ok(tables) => {
                                egui::ScrollArea::vertical()
                                    .max_height(screen.height() * 0.55)
                                    .show(ui, |ui| {
                                        for (name, path) in tables {
                                            if ui.button(&name).clicked() {
                                                if let Some(error) = queue_library_import(
                                                    ui,
                                                    parent,
                                                    slot.index(),
                                                    path,
                                                ) {
                                                    set_import_status(
                                                        ui,
                                                        parent,
                                                        slot.index(),
                                                        error,
                                                        true,
                                                    );
                                                } else {
                                                    set_current_table_name(ui, parent, slot, name);
                                                }
                                                close = true;
                                            }
                                        }
                                    });
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
    if close || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|store| {
            store.remove::<LibraryPopup>(popup_id);
            store.remove::<Result<Vec<(String, std::path::PathBuf)>, String>>(library_listing_id(
                parent,
                slot.index(),
            ));
            if let Some(next_kind) = next_kind {
                store.insert_temp(popup_id, next_kind);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::capture_unwarped_curve;
    use crate::generators::OscillatorConfig;
    use crate::wave_curve::WaveCurveRt;

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
