use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::editor_controls::fit_font_to_width;
use crate::editor_theme;
use crate::editor_widgets::icon_font_ready;
use crate::generators::{GroupId, GroupOutput};

use super::super::translucent;
use super::GroupOutputInteraction;

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
    _group_size: egui::Vec2,
    collapsed: bool,
    output: GroupOutput,
    group_accent: egui::Color32,
) -> (egui::Rect, GroupOutputInteraction) {
    let palette = editor_theme::semantic();
    let inset = rect.shrink2(egui::vec2(
        editor_theme::shape::STROKE,
        editor_theme::shape::STROKE,
    ));
    let default_group_label = format!("Group {}", group_index + 1);
    let group_label = state
        .params()
        .editor_state
        .lock()
        .ok()
        .and_then(|editor| editor.group_name(group_id.get()).map(str::to_owned))
        .unwrap_or_else(|| default_group_label.clone());
    let action_count = if can_remove_group { 4.0 } else { 3.0 };
    let identity_width = (inset.width() * 0.24)
        .clamp(
            editor_theme::title_height(ui) * 7.0,
            editor_theme::title_height(ui) * 11.0,
        )
        .min(inset.width() * 0.38);
    let action_cell = (inset.height() * 0.72).min(identity_width / (action_count + 1.35));
    let identity = egui::Rect::from_min_size(inset.min, egui::vec2(identity_width, inset.height()));
    let identity_ink = editor_theme::on_accent(group_accent);
    let shoulder = identity.height() * 0.82;
    let mut tab_shape = vec![identity.left_top()];
    for step in 0..=12 {
        let t = step as f32 / 12.0;
        let eased = t * t * (3.0 - 2.0 * t);
        tab_shape.push(egui::pos2(
            identity.right() + shoulder * (1.0 - eased),
            egui::lerp(identity.top()..=identity.bottom(), t),
        ));
    }
    tab_shape.push(identity.left_bottom());
    ui.painter().add(egui::Shape::convex_polygon(
        tab_shape,
        group_accent,
        egui::Stroke::NONE,
    ));
    let controls = egui::Rect::from_min_max(
        egui::pos2(identity.right() + shoulder * 0.88, inset.top()),
        inset.max,
    );
    let remove_width = if can_remove_group { action_cell } else { 0.0 };
    let collapse_rect =
        egui::Rect::from_min_size(identity.min, egui::vec2(action_cell, identity.height()));
    let accent_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), identity.top()),
        egui::pos2(collapse_rect.right() + action_cell, identity.bottom()),
    );
    let power_rect = egui::Rect::from_min_max(
        egui::pos2(accent_rect.right(), identity.top()),
        egui::pos2(accent_rect.right() + action_cell, identity.bottom()),
    );
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(power_rect.right(), identity.top()),
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
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, identity_ink),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().rect_filled(
        collapse_rect.shrink(editor_theme::space::XS),
        editor_theme::shape::CONTROL_RADIUS,
        palette.masthead_ink,
    );
    let group_drag = ui
        .interact(
            label_rect,
            egui::Id::new(("generator-group-drag", group_id.get())),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(
            "Drag the group name to move; hold Ctrl to duplicate; double-click to rename",
        );
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
        ui.ctx()
            .set_cursor_icon(if ui.input(|input| input.modifiers.ctrl) {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Grabbing
            });
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
    if accent_response.hovered() || accent_response.is_pointer_button_down_on() {
        ui.painter().rect_filled(
            accent_button,
            editor_theme::shape::CONTROL_RADIUS,
            translucent(identity_ink, 32),
        );
    }
    if icon_font_ready(ui) {
        ui.painter().text(
            accent_button.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::PALETTE,
            editor_theme::font::title(),
            identity_ink,
        );
    }
    if accent_response.has_focus() {
        ui.painter().rect_stroke(
            accent_button,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, identity_ink),
            egui::StrokeKind::Inside,
        );
    }
    let mut selected = group_accent;
    let selected_accent = plugcat::widgets::color_picker_popup(ui, &accent_response, &mut selected)
        .then_some(selected);
    if icon_font_ready(ui) {
        ui.painter().text(
            collapse_rect.center(),
            egui::Align2::CENTER_CENTER,
            if collapsed {
                egui_phosphor::regular::FOLDER
            } else {
                egui_phosphor::regular::FOLDER_OPEN
            },
            editor_theme::font::title(),
            group_accent,
        );
    }
    let painted_group_label = group_label.to_uppercase();
    let label_font = fit_font_to_width(
        ui.painter(),
        &painted_group_label,
        editor_theme::font::title(),
        label_rect.width() * 0.92,
    );
    let label_origin = label_rect.left_center() + egui::vec2(editor_theme::space::XS, 0.0);
    let rename_id = egui::Id::new(("generator-group-rename", group_id.get()));
    let mut editing_name = ui.data(|data| data.get_temp::<bool>(rename_id).unwrap_or(false));
    let newly_editing = group_drag.double_clicked();
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
                .horizontal_align(egui::Align::Min)
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
            label_origin,
            egui::Align2::LEFT_CENTER,
            &painted_group_label,
            label_font,
            identity_ink,
        );
    }

    let mut updated_output = output;
    super::draw_envelope_power(ui, power_rect, group_id, &mut updated_output, identity_ink);

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
            if icon_font_ready(ui) {
                egui_phosphor::regular::X
            } else {
                ""
            },
            editor_theme::font::label(),
            if remove_armed || pressed || response.hovered() {
                identity_ink
            } else {
                identity_ink.gamma_multiply(0.62)
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
            output: (updated_output != output).then_some(updated_output),
        },
    )
}

fn keyboard_activate(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        })
}
