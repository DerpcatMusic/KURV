use crate::editor_theme;
use crate::generators::GroupId;

use super::super::translucent;
use super::{GeneratorInsertionTarget, rack_item_visible};

#[derive(Clone, Copy)]
pub(super) enum GeneratorAddAction {
    Oscillator,
    Filter,
    Group,
}

fn insertion_menu_id(target: GeneratorInsertionTarget) -> egui::Id {
    match target {
        GeneratorInsertionTarget::Group(insertion) => {
            egui::Id::new(("generator-stack-insert-menu", insertion))
        }
        GeneratorInsertionTarget::Module(group, insertion) => {
            egui::Id::new(("generator-module-insert-menu", group, insertion))
        }
    }
}

fn root_menu_id() -> egui::Id {
    egui::Id::new("generator-add-menu-root")
}

fn group_menu_id(group_id: GroupId) -> egui::Id {
    egui::Id::new(("generator-group-add-menu", group_id.get()))
}

fn menu_open(ui: &egui::Ui, menu_id: egui::Id) -> bool {
    ui.data(|data| data.get_temp::<bool>(menu_id).unwrap_or(false))
}

pub(super) fn insertion_open(ui: &egui::Ui, target: GeneratorInsertionTarget) -> bool {
    menu_open(ui, insertion_menu_id(target))
}

pub(super) fn root_open(ui: &egui::Ui) -> bool {
    menu_open(ui, root_menu_id())
}

pub(super) fn group_open(ui: &egui::Ui, group_id: GroupId) -> bool {
    menu_open(ui, group_menu_id(group_id))
}

pub(super) fn show_root(
    ui: &mut egui::Ui,
    can_add_oscillator: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let menu_id = root_menu_id();
    let open = menu_open(ui, menu_id);
    let (id, button_rect) = ui.allocate_space(egui::vec2(
        ui.available_width(),
        editor_theme::title_height(ui),
    ));
    if !rack_item_visible(ui, button_rect) && !open {
        return None;
    }
    let response = ui
        .interact(button_rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_add_button(ui, button_rect, &response, false, false, open);
    show_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        false,
        can_add_group,
        false,
    )
}

pub(super) fn show_insertion(
    ui: &mut egui::Ui,
    target: GeneratorInsertionTarget,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let menu_id = insertion_menu_id(target);
    let open = menu_open(ui, menu_id);
    let (button_rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), editor_theme::title_height(ui)),
        egui::Sense::click(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_add_button(
        ui,
        button_rect,
        &response,
        true,
        matches!(target, GeneratorInsertionTarget::Module(_, _)),
        open,
    );
    show_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        can_add_filter,
        can_add_group,
        matches!(target, GeneratorInsertionTarget::Module(_, _)),
    )
}

pub(super) fn show_group(
    ui: &mut egui::Ui,
    group_id: GroupId,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let menu_id = group_menu_id(group_id);
    let open = menu_open(ui, menu_id);
    let (id, button_rect) = ui.allocate_space(egui::vec2(
        ui.available_width(),
        editor_theme::title_height(ui),
    ));
    if !rack_item_visible(ui, button_rect) && !open {
        return None;
    }
    let response = ui
        .interact(button_rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_add_button(ui, button_rect, &response, false, true, open);
    show_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        can_add_filter,
        can_add_group,
        true,
    )
}

fn paint_add_button(
    ui: &egui::Ui,
    button_rect: egui::Rect,
    response: &egui::Response,
    insertion: bool,
    contained: bool,
    open: bool,
) {
    let palette = editor_theme::semantic();
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let visual_rect = if contained {
        button_rect.shrink2(egui::vec2(
            editor_theme::space::SM,
            editor_theme::space::XXS,
        ))
    } else {
        button_rect
    };
    if insertion || open || pressed {
        ui.painter().rect_filled(
            visual_rect,
            editor_theme::shape::CONTROL_RADIUS,
            if insertion {
                translucent(palette.primary, if pressed { 34 } else { 22 })
            } else if open || pressed {
                palette.control
            } else {
                palette.control
            },
        );
    }
    let stroke = egui::Stroke::new(
        if pressed || open {
            editor_theme::shape::FOCUS_STROKE
        } else {
            editor_theme::shape::STROKE
        },
        if insertion || hovered || open {
            palette.primary
        } else {
            palette.grid
        },
    );
    let dash = editor_theme::space::SM;
    let gap = editor_theme::space::XS;
    if contained {
        for edge in [
            [visual_rect.left_top(), visual_rect.right_top()],
            [visual_rect.left_bottom(), visual_rect.right_bottom()],
        ] {
            ui.painter()
                .add(egui::Shape::dashed_line(&edge, stroke, dash, gap));
        }
    } else {
        let outline = [
            visual_rect.left_top(),
            visual_rect.right_top(),
            visual_rect.right_bottom(),
            visual_rect.left_bottom(),
            visual_rect.left_top(),
        ];
        ui.painter()
            .add(egui::Shape::dashed_line(&outline, stroke, dash, gap));
    }
    ui.painter().text(
        egui::pos2(
            visual_rect.left() + editor_theme::space::SM,
            visual_rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        "+ ADD MODULE",
        editor_theme::font::label(),
        if insertion {
            palette.primary
        } else if hovered || open || pressed {
            palette.text
        } else {
            palette.text_muted
        },
    );
}

fn show_popup(
    ui: &mut egui::Ui,
    menu_id: egui::Id,
    button_rect: egui::Rect,
    response: &egui::Response,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_group: bool,
    show_filter: bool,
) -> Option<GeneratorAddAction> {
    let mut action = None;
    let mut open = menu_open(ui, menu_id);
    let clicked = response.clicked()
        || ui.input(|input| {
            input.pointer.primary_clicked()
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|pointer| response.rect.contains(pointer))
        });
    if clicked {
        open = !open;
    }
    if open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }

    if open {
        let frame_margin = (ui.spacing().item_spacing.x * 0.5).round() as i8;
        let row_height = ui.spacing().interact_size.y * 0.9;
        let popup_width = (button_rect.width() * 0.24)
            .clamp(ui.spacing().interact_size.x * 5.0, button_rect.width());
        let popup_height = row_height * if show_filter { 3.0 } else { 2.0 }
            + editor_theme::font::caption().size
            + editor_theme::space::SM
            + f32::from(frame_margin) * 2.0;
        let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
        let popup_x = button_rect.left().clamp(
            screen.left(),
            (screen.right() - popup_width).max(screen.left()),
        );
        let popup_y = if button_rect.bottom() + popup_height <= screen.bottom() {
            button_rect.bottom()
        } else {
            (button_rect.top() - popup_height).max(screen.top())
        };
        let popup = egui::Area::new(menu_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(popup_x, popup_y))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(editor_theme::semantic().surface)
                    .stroke(egui::Stroke::new(
                        editor_theme::shape::STROKE,
                        editor_theme::semantic().grid,
                    ))
                    .inner_margin(egui::Margin::same(frame_margin))
                    .show(ui, |ui| {
                        ui.set_min_width(popup_width);
                        let oscillator_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                        });
                        let filter_key = show_filter
                            && ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                            });
                        let group_key = ui.input_mut(|input| {
                            input.consume_key(
                                egui::Modifiers::NONE,
                                if show_filter {
                                    egui::Key::Num3
                                } else {
                                    egui::Key::Num2
                                },
                            )
                        });
                        let oscillator = ui
                            .add_enabled(
                                can_add_oscillator,
                                egui::Button::new("1   OSCILLATOR")
                                    .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_oscillator && oscillator_key);
                        let filter = show_filter
                            && (ui
                                .add_enabled(
                                    can_add_filter,
                                    egui::Button::new("2   FILTER")
                                        .min_size(egui::vec2(popup_width, row_height)),
                                )
                                .clicked()
                                || (can_add_filter && filter_key));
                        let group = ui
                            .add_enabled(
                                can_add_group,
                                egui::Button::new(if show_filter {
                                    "3   GROUP"
                                } else {
                                    "2   GROUP"
                                })
                                .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_group && group_key);
                        if oscillator {
                            action = Some(GeneratorAddAction::Oscillator);
                        } else if filter {
                            action = Some(GeneratorAddAction::Filter);
                        } else if group {
                            action = Some(GeneratorAddAction::Group);
                        }
                        ui.label(
                            egui::RichText::new(if show_filter {
                                "KEYS 1 / 2 / 3"
                            } else {
                                "KEYS 1 / 2"
                            })
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                        );
                    });
            });
        if ui.input(|input| {
            input.pointer.primary_clicked()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    !button_rect.contains(pointer) && !popup.response.rect.contains(pointer)
                })
        }) {
            open = false;
        }
    }
    if action.is_some() {
        open = false;
    }
    ui.data_mut(|data| data.insert_temp(menu_id, open));
    action
}
