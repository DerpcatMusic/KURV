//! VA-table selection and frame actions.

mod curve_editor;

use super::preview;
use curve_editor::{FreehandStroke, edit_wave_curve_target, hit_curve_target};
use truce_core::editor::PluginContext;

use crate::generators::{ModuleId, OscillatorConfig, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::oscillators::MAX_VA_TABLE_FRAMES;
use crate::wave_curve::WaveCurveData;
use crate::{KurvParams, editor_theme};

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
    let mut cache = preview::VaPreviewCache::load(ui, cache_id, table_state);
    let table = cache.table();
    let fallback = WaveCurveData::default();
    let selection = table.select(fallback.compile_rt(), config.custom_shape);
    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let plot = preview::cycle_plot(response.rect);
    let editing = ui.data(|store| {
        store
            .get_temp::<WaveCurveData>(response.id.with(("wave-curve-draft", slot.index())))
            .is_some()
            || store
                .get_temp::<FreehandStroke>(response.id.with(("wave-curve-stroke", slot.index())))
                .is_some()
    });
    let accent = editor_theme::palette().accent;
    preview::paint_cached_cycle(
        &mut cache,
        &painter,
        response.rect,
        plot,
        config,
        selection.curve,
        selection.mix,
        editing,
        accent,
    );
    cache.store(ui, cache_id);
    let table_frames = table.frame_count();
    let custom_frames = table_frames.max(1);
    let selected_frame =
        ((config.custom_shape * custom_frames as f32).round() as usize).clamp(1, custom_frames) - 1;
    let host_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::TablePosition);
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, host_target);
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
