use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::editor_controls::fit_font_to_width;
use crate::editor_theme;
use crate::generators::{GroupId, GroupOutput};

use super::super::drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
use super::super::translucent;
use super::GroupOutputInteraction;
use super::controls::output_pair_label;

pub(crate) fn clear_group_name_edit_state(ui: &egui::Ui, group_id: GroupId) {
    let rename_id = egui::Id::new(("generator-group-rename", group_id.get()));
    ui.data_mut(|data| {
        data.remove::<bool>(rename_id);
        data.remove::<String>(rename_id.with("draft"));
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_group_identity(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: GroupId,
    group_index: usize,
    can_remove_group: bool,
    module_count: usize,
    group_size: egui::Vec2,
    collapsed: bool,
    output: GroupOutput,
    group_accent: egui::Color32,
) -> (egui::Rect, GroupOutputInteraction) {
    let palette = editor_theme::semantic();
    let inset = rect.shrink2(egui::vec2(
        editor_theme::space::SM.min(rect.width() * 0.008),
        editor_theme::space::XXS,
    ));
    let default_group_label = format!("Group {}", group_index + 1);
    let group_label = state
        .params()
        .editor_state
        .lock()
        .ok()
        .and_then(|editor| editor.group_name(group_id.get()).map(str::to_owned))
        .unwrap_or_else(|| default_group_label.clone());
    let label_width = ui
        .painter()
        .layout_no_wrap(
            group_label.clone(),
            editor_theme::font::label(),
            palette.text,
        )
        .size()
        .x
        + editor_theme::space::SM;
    let action_count = if can_remove_group { 4.0 } else { 3.0 };
    let action_cell = inset.height().min(inset.width() / action_count);
    let action_width = action_cell * action_count;
    let identity_width = (label_width + action_width).min(inset.width());
    let identity = egui::Rect::from_min_size(inset.min, egui::vec2(identity_width, inset.height()));
    let controls = egui::Rect::from_min_max(
        egui::pos2(identity.right() + editor_theme::space::XS, inset.top()),
        inset.max,
    );
    let remove_width = if can_remove_group { action_cell } else { 0.0 };
    let collapse_rect =
        egui::Rect::from_min_size(identity.min, egui::vec2(action_cell, identity.height()));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), identity.top()),
        egui::pos2(collapse_rect.right() + action_cell, identity.bottom()),
    );
    let accent_rect = egui::Rect::from_min_max(
        egui::pos2(drag_rect.right(), identity.top()),
        egui::pos2(drag_rect.right() + action_cell, identity.bottom()),
    );
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(accent_rect.right(), identity.top()),
        egui::pos2(identity.right() - remove_width, identity.bottom()),
    );
    let remove_rect =
        egui::Rect::from_min_max(egui::pos2(label_rect.right(), identity.top()), identity.max);
    let collapse_response = ui
        .interact(
            collapse_rect,
            egui::Id::new(("generator-group-collapse", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Expand this group"
        } else {
            "Collapse this group"
        });
    if collapse_response.has_focus() {
        ui.painter().rect_stroke(
            collapse_rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, group_accent),
            egui::StrokeKind::Inside,
        );
    }
    let group_drag = ui
        .interact(
            drag_rect,
            egui::Id::new(("generator-group-drag", group_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to move this whole group; arrow keys reorder");
    let reorder = if group_drag.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                group_drag.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        ui.input(|input| {
            i8::from(input.key_pressed(egui::Key::ArrowDown))
                - i8::from(input.key_pressed(egui::Key::ArrowUp))
        })
    } else {
        0
    };
    group_drag.dnd_set_drag_payload(group_id);
    if group_drag.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            paint_generator_drag_ghost(
                ui,
                ("group", group_id.get()),
                pointer,
                group_size,
                group_accent,
                &group_label,
                &output_pair_label(output.pair),
                GeneratorDragGhostKind::Group { module_count },
            );
        }
    }
    let accent_response = ui
        .interact(
            accent_rect,
            egui::Id::new(("generator-group-accent", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Choose this group's color");
    if keyboard_activate(ui, &accent_response) {
        egui::Popup::toggle_id(ui.ctx(), egui::Popup::default_response_id(&accent_response));
    }
    let accent_button = accent_rect.shrink(editor_theme::space::XXS);
    let swatch_side = accent_button.height() * 0.58;
    let swatch =
        egui::Rect::from_center_size(accent_button.center(), egui::Vec2::splat(swatch_side));
    let swatch_radius = editor_theme::shape::CONTROL_RADIUS.min(swatch_side * 0.22);
    if accent_response.hovered() || accent_response.is_pointer_button_down_on() {
        ui.painter().rect_filled(
            swatch.expand(editor_theme::space::XXS),
            swatch_radius + editor_theme::space::XXS,
            translucent(group_accent, 32),
        );
    }
    ui.painter().rect_stroke(
        swatch,
        swatch_radius,
        egui::Stroke::new(
            if accent_response.has_focus() {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::GROUP_STROKE
            },
            group_accent,
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(
        swatch.shrink(editor_theme::shape::STROKE),
        (swatch_radius - editor_theme::shape::STROKE).max(0.0),
        group_accent.gamma_multiply(if accent_response.is_pointer_button_down_on() {
            0.72
        } else {
            1.0
        }),
    );
    let cue_side = swatch_side * 0.22;
    let cue_center = accent_button.right_bottom() - egui::vec2(cue_side * 0.72, cue_side * 0.58);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            cue_center + egui::vec2(-cue_side * 0.50, -cue_side * 0.28),
            cue_center + egui::vec2(cue_side * 0.50, -cue_side * 0.28),
            cue_center + egui::vec2(0.0, cue_side * 0.42),
        ],
        if accent_response.hovered() || accent_response.has_focus() {
            palette.text
        } else {
            palette.text_muted.gamma_multiply(0.62)
        },
        egui::Stroke::NONE,
    ));
    let mut selected = group_accent;
    let selected_accent = plugcat::widgets::color_picker_popup(ui, &accent_response, &mut selected)
        .then_some(selected);
    let marker_side = collapse_rect.height() * 0.14;
    let marker_center = collapse_rect.center();
    let marker_points = if collapsed {
        vec![
            marker_center + egui::vec2(-marker_side * 0.42, -marker_side * 0.72),
            marker_center + egui::vec2(marker_side * 0.42, 0.0),
            marker_center + egui::vec2(-marker_side * 0.42, marker_side * 0.72),
        ]
    } else {
        vec![
            marker_center + egui::vec2(-marker_side * 0.72, -marker_side * 0.42),
            marker_center + egui::vec2(0.0, marker_side * 0.42),
            marker_center + egui::vec2(marker_side * 0.72, -marker_side * 0.42),
        ]
    };
    ui.painter().add(egui::Shape::line(
        marker_points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            if collapse_response.hovered()
                || collapse_response.is_pointer_button_down_on()
                || collapse_response.has_focus()
            {
                palette.text
            } else {
                group_accent
            },
        ),
    ));
    let grip_dot = editor_theme::shape::STROKE;
    let grip_gap = editor_theme::space::XXS;
    let grip_origin = drag_rect.center() - egui::vec2(grip_gap * 0.5, grip_gap);
    let grip_color = if group_drag.dragged() {
        palette.text
    } else if group_drag.hovered() || group_drag.has_focus() {
        group_accent
    } else {
        palette.text_muted.gamma_multiply(0.56)
    };
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                grip_origin + egui::vec2(column as f32 * grip_gap, row as f32 * grip_gap),
                grip_dot,
                grip_color,
            );
        }
    }
    let label_font = fit_font_to_width(
        ui.painter(),
        &group_label,
        editor_theme::font::label(),
        label_rect.width() * 0.92,
    );
    let label_galley =
        ui.painter()
            .layout_no_wrap(group_label.clone(), label_font.clone(), palette.text);
    let label_hit = egui::Rect::from_center_size(label_rect.center(), label_galley.size())
        .expand(editor_theme::space::XXS)
        .intersect(label_rect);
    let label_response = ui
        .interact(
            label_hit,
            egui::Id::new(("generator-group-label", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Text)
        .on_hover_text("Double-click to rename this group");
    let rename_id = egui::Id::new(("generator-group-rename", group_id.get()));
    let mut editing_name = ui.data(|data| data.get_temp::<bool>(rename_id).unwrap_or(false));
    let newly_editing = label_response.double_clicked();
    if newly_editing {
        editing_name = true;
        ui.data_mut(|data| {
            data.insert_temp(rename_id, true);
            data.insert_temp(rename_id.with("draft"), group_label.clone());
        });
    }
    if editing_name {
        let mut draft = ui
            .data(|data| data.get_temp::<String>(rename_id.with("draft")))
            .unwrap_or_else(|| group_label.clone());
        let edit = ui.put(
            label_rect,
            egui::TextEdit::singleline(&mut draft)
                .id_salt(rename_id.with("field"))
                .font(label_font.clone())
                .horizontal_align(egui::Align::Center)
                .desired_width(label_rect.width())
                .frame(egui::Frame::NONE),
        );
        if newly_editing {
            edit.request_focus();
        }
        let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let commit = ui.input(|input| input.key_pressed(egui::Key::Enter))
            || (edit.lost_focus() && !newly_editing);
        if cancel {
            editing_name = false;
        } else if commit {
            if let Ok(mut editor) = state.params().editor_state.lock() {
                editor.set_group_name(group_id.get(), &draft);
            }
            editing_name = false;
            crate::editor_shell::request_structural_commit(ui);
        } else {
            ui.data_mut(|data| data.insert_temp(rename_id.with("draft"), draft));
        }
        ui.data_mut(|data| data.insert_temp(rename_id, editing_name));
    } else {
        ui.painter().text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            &group_label,
            label_font,
            if group_drag.dragged()
                || group_drag.hovered()
                || group_drag.has_focus()
                || label_response.hovered()
                || label_response.has_focus()
            {
                group_accent
            } else {
                palette.text
            },
        );
    }

    let remove_confirm_id = egui::Id::new(("generator-group-remove-confirm", group_id.get()));
    let mut remove_armed = module_count > 0
        && ui
            .data(|data| data.get_temp::<bool>(remove_confirm_id))
            .unwrap_or(false);
    let remove_response = can_remove_group.then(|| {
        ui.interact(
            remove_rect,
            egui::Id::new(("generator-group-remove", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if remove_armed {
            "Click again to remove this group and its modules"
        } else {
            "Remove this group and its modules"
        })
    });
    let toggle_collapse = collapse_response.clicked() || keyboard_activate(ui, &collapse_response);
    let mut remove = false;
    if let Some(response) = &remove_response {
        let activate = response.clicked() || keyboard_activate(ui, response);
        if module_count == 0 {
            remove = activate;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        } else if remove_armed && activate {
            remove = true;
            remove_armed = false;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        } else if activate {
            remove_armed = true;
            ui.data_mut(|data| data.insert_temp(remove_confirm_id, true));
        } else if remove_armed
            && (ui.input(|input| input.key_pressed(egui::Key::Escape))
                || ui.input(|input| {
                    input.pointer.primary_clicked()
                        && input
                            .pointer
                            .latest_pos()
                            .is_some_and(|pointer| !response.rect.contains(pointer))
                }))
        {
            remove_armed = false;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        }
        let pressed = response.is_pointer_button_down_on();
        if remove_armed || pressed {
            ui.painter().rect_filled(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                translucent(palette.danger, if pressed { 64 } else { 48 }),
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, palette.danger),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            remove_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            editor_theme::font::label(),
            if remove_armed || pressed || response.hovered() {
                palette.text
            } else {
                palette.text_muted
            },
        );
    }
    if remove_response.is_none() {
        ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
    }

    (
        controls,
        GroupOutputInteraction {
            remove,
            toggle_collapse,
            reorder,
            accent: selected_accent,
            output: None,
        },
    )
}

fn keyboard_activate(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        })
}
