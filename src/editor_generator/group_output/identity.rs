use crate::editor_controls::fit_font_to_width;
use crate::editor_theme;
use crate::generators::{GroupId, GroupOutput};

use super::super::drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
use super::super::translucent;
use super::GroupOutputInteraction;
use super::controls::output_pair_label;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_group_identity(
    ui: &mut egui::Ui,
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
    let group_label = if collapsed {
        format!(
            "G{} · {module_count} MODULE{}",
            group_index + 1,
            if module_count == 1 { "" } else { "S" }
        )
    } else {
        format!("G{}", group_index + 1)
    };
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
    let action_count = if can_remove_group { 3.0 } else { 2.0 };
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
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(drag_rect.right(), identity.top()),
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
                &format!("GROUP {}", group_index + 1),
                &output_pair_label(output.pair),
                GeneratorDragGhostKind::Group { module_count },
            );
        }
    }
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
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Double-click to expand this group"
        } else {
            "Double-click to collapse this group"
        });
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
    let keyboard_activate = |response: &egui::Response| {
        response.has_focus()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
            })
    };
    let toggle_collapse = collapse_response.clicked()
        || keyboard_activate(&collapse_response)
        || label_response.double_clicked();
    let mut remove = false;
    if let Some(response) = &remove_response {
        let activate = response.clicked() || keyboard_activate(response);
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
            && ((!response.hovered() && !response.has_focus())
                || ui.input(|input| input.key_pressed(egui::Key::Escape)))
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
        },
    )
}
