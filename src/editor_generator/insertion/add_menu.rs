use super::super::translucent;
use super::{GeneratorInsertionTarget, rack_item_visible};
use crate::editor_theme;
use crate::editor_widgets::menu_choice;

const ACTIVE_INSERTION_MENU_ID: &str = "generator-active-insertion-menu";

#[derive(Clone, Copy)]
pub(super) enum GeneratorAddAction {
    Oscillator,
    Resynth,
    Noise,
    Filter,
    Aux,
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

fn menu_open(ui: &egui::Ui, menu_id: egui::Id) -> bool {
    ui.data(|data| data.get_temp::<bool>(menu_id).unwrap_or(false))
}

pub(super) fn insertion_open(ui: &egui::Ui, target: GeneratorInsertionTarget) -> bool {
    ui.data(|data| {
        data.get_temp::<GeneratorInsertionTarget>(egui::Id::new(ACTIVE_INSERTION_MENU_ID))
            == Some(target)
    }) && menu_open(ui, insertion_menu_id(target))
}

pub(super) fn clear_insertion_open(ui: &egui::Ui) {
    ui.data_mut(|data| {
        data.remove::<GeneratorInsertionTarget>(egui::Id::new(ACTIVE_INSERTION_MENU_ID));
    });
}

pub(super) fn root_open(ui: &egui::Ui) -> bool {
    menu_open(ui, root_menu_id())
}

pub(super) fn show_root(
    ui: &mut egui::Ui,
    can_add_oscillator: bool,
    can_add_aux: bool,
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
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Add a group or module");
    paint_add_button(ui, button_rect, &response, false, false, open);
    show_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        false,
        can_add_aux,
        can_add_group,
        false,
    )
}

pub(super) fn show_group_footer_add(
    ui: &mut egui::Ui,
    group: u64,
    plus: egui::Rect,
    accent: egui::Color32,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_aux: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let menu_id = egui::Id::new(("generator-group-footer-add", group));
    let open = menu_open(ui, menu_id);
    let response = ui
        .interact(plus, menu_id.with("hit"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Add a module to this group");
    if response.hovered() || response.clicked() || response.is_pointer_button_down_on() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let hovered = response.hovered() || open || response.is_pointer_button_down_on();
    let color = if hovered {
        accent
    } else {
        accent.gamma_multiply(0.82)
    };
    let surface = editor_theme::semantic().surface;
    ui.painter().rect_filled(
        plus,
        0.0,
        egui::Color32::from_rgb(surface.r(), surface.g(), surface.b()),
    );
    ui.painter().rect_stroke(
        plus,
        0.0,
        egui::Stroke::new(editor_theme::shape::STROKE, color),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        plus.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        editor_theme::font::title(),
        color,
    );
    show_popup(
        ui,
        menu_id,
        plus,
        &response,
        can_add_oscillator,
        can_add_filter,
        can_add_aux,
        can_add_group,
        true,
    )
}

pub(super) fn show_insertion(
    ui: &mut egui::Ui,
    target: GeneratorInsertionTarget,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_aux: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let menu_id = insertion_menu_id(target);
    let active_id = egui::Id::new(ACTIVE_INSERTION_MENU_ID);
    if ui.data(|data| data.get_temp::<GeneratorInsertionTarget>(active_id)) != Some(target) {
        ui.data_mut(|data| data.remove::<bool>(menu_id));
    }
    let open = menu_open(ui, menu_id);
    let (button_rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), editor_theme::title_height(ui)),
        egui::Sense::click(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Add a module here");
    paint_add_button(
        ui,
        button_rect,
        &response,
        true,
        matches!(target, GeneratorInsertionTarget::Module(_, _)),
        open,
    );
    let action = show_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        can_add_filter,
        can_add_aux,
        can_add_group,
        matches!(target, GeneratorInsertionTarget::Module(_, _)),
    );
    let open = menu_open(ui, menu_id);
    ui.data_mut(|data| {
        if open {
            data.insert_temp(active_id, target);
        } else if data.get_temp::<GeneratorInsertionTarget>(active_id) == Some(target) {
            data.remove::<GeneratorInsertionTarget>(active_id);
        }
    });
    action
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
    let side = button_rect
        .height()
        .min(editor_theme::space::LG + editor_theme::space::XS)
        .max(editor_theme::space::MD);
    let visual_rect = if insertion {
        button_rect.shrink2(egui::vec2(
            editor_theme::space::XXS,
            editor_theme::shape::STROKE,
        ))
    } else {
        egui::Rect::from_center_size(button_rect.center(), egui::Vec2::splat(side))
    };
    if insertion || open || pressed || hovered {
        ui.painter().rect_filled(
            visual_rect,
            editor_theme::shape::CONTROL_RADIUS,
            if insertion {
                translucent(palette.primary, if pressed { 40 } else { 24 })
            } else if open || pressed {
                palette.control
            } else {
                translucent(palette.control, 80)
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
    let dash = editor_theme::space::XS;
    let gap = editor_theme::space::XXS;
    let outline = [
        visual_rect.left_top(),
        visual_rect.right_top(),
        visual_rect.right_bottom(),
        visual_rect.left_bottom(),
        visual_rect.left_top(),
    ];
    ui.painter()
        .add(egui::Shape::dashed_line(&outline, stroke, dash, gap));
    ui.painter().text(
        visual_rect.center(),
        egui::Align2::CENTER_CENTER,
        if insertion {
            if contained {
                "+  ADD MODULE"
            } else {
                "+  ADD GROUP"
            }
        } else {
            "+"
        },
        if insertion {
            editor_theme::font::caption()
        } else {
            editor_theme::font::title()
        },
        if insertion || hovered || open || pressed {
            palette.primary
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
    can_add_aux: bool,
    can_add_group: bool,
    show_filter: bool,
) -> Option<GeneratorAddAction> {
    let mut action = None;
    let mut open = menu_open(ui, menu_id);
    if response.clicked() {
        if !open {
            let anchor = ui
                .input(|input| input.pointer.latest_pos())
                .unwrap_or(button_rect.left_bottom());
            ui.data_mut(|data| data.insert_temp(menu_id.with("anchor"), anchor));
        }
        open = !open;
    }
    if open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }

    if open {
        let frame_margin = (ui.spacing().item_spacing.x * 0.5).round() as i8;
        let row_height = ui.spacing().interact_size.y * 0.9;
        let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
        let popup_width = (ui.spacing().interact_size.x * 6.0)
            .min(screen.width() * 0.32)
            .max(ui.spacing().interact_size.x * 4.0);
        let popup_height = row_height * if show_filter { 6.0 } else { 5.0 }
            + editor_theme::font::caption().size
            + editor_theme::space::SM
            + f32::from(frame_margin) * 2.0;
        let anchor = ui
            .data(|data| data.get_temp::<egui::Pos2>(menu_id.with("anchor")))
            .unwrap_or(button_rect.left_bottom());
        let popup_x = anchor.x.clamp(
            screen.left(),
            (screen.right() - popup_width).max(screen.left()),
        );
        let below = anchor.y + editor_theme::space::XXS;
        let popup_y = if below + popup_height <= screen.bottom() {
            below
        } else {
            (anchor.y - popup_height - editor_theme::space::XXS).max(screen.top())
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
                        let resynth_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                        });
                        let filter_key = show_filter
                            && ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, egui::Key::Num4)
                            });
                        let aux_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num5)
                        });
                        let group_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num6)
                        });
                        let oscillator = menu_choice(
                            ui,
                            1,
                            "OSCILLATOR",
                            can_add_oscillator,
                            popup_width,
                            row_height,
                            editor_theme::semantic().primary,
                        ) || (can_add_oscillator && oscillator_key);
                        let resynth = menu_choice(
                            ui,
                            2,
                            "RESYNTH",
                            can_add_oscillator,
                            popup_width,
                            row_height,
                            editor_theme::semantic().pan_shape,
                        ) || (can_add_oscillator && resynth_key);
                        let noise_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num3)
                        });
                        let noise = menu_choice(
                            ui,
                            3,
                            "NOISE",
                            can_add_oscillator,
                            popup_width,
                            row_height,
                            editor_theme::semantic().unison,
                        ) || (can_add_oscillator && noise_key);
                        let filter = show_filter
                            && (menu_choice(
                                ui,
                                4,
                                "FILTER",
                                can_add_filter,
                                popup_width,
                                row_height,
                                editor_theme::semantic().primary,
                            ) || (can_add_filter && filter_key));
                        let aux = menu_choice(
                            ui,
                            5,
                            "AUX",
                            can_add_aux,
                            popup_width,
                            row_height,
                            editor_theme::semantic().unison,
                        ) || (can_add_aux && aux_key);
                        let group_ordinal = 6;
                        let group = menu_choice(
                            ui,
                            group_ordinal,
                            "GROUP",
                            can_add_group,
                            popup_width,
                            row_height,
                            editor_theme::semantic().primary,
                        ) || (can_add_group && group_key);
                        if oscillator {
                            action = Some(GeneratorAddAction::Oscillator);
                        } else if resynth {
                            action = Some(GeneratorAddAction::Resynth);
                        } else if noise {
                            action = Some(GeneratorAddAction::Noise);
                        } else if filter {
                            action = Some(GeneratorAddAction::Filter);
                        } else if aux {
                            action = Some(GeneratorAddAction::Aux);
                        } else if group {
                            action = Some(GeneratorAddAction::Group);
                        }
                        ui.label(
                            egui::RichText::new(if show_filter {
                                "KEYS 1 / 2 / 3 / 4 / 5 / 6"
                            } else {
                                "KEYS 1 / 2 / 3 / 5 / 6"
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
