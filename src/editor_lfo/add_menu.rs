use super::*;
use crate::editor_widgets::menu_choice;

pub(super) fn draw_add_modulator(
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
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Add a modulator")
    } else {
        response.on_hover_text("Modulator limit reached; remove a source to add another")
    };
    let pressed = response.is_pointer_button_down_on();
    let hovered = can_add && response.hovered();
    let side = rect
        .height()
        .min(editor_theme::space::LG + editor_theme::space::XS)
        .max(editor_theme::space::MD);
    let visual = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    if insertion || open || pressed || hovered {
        ui.painter().rect_filled(
            visual,
            editor_theme::shape::CONTROL_RADIUS,
            if insertion {
                egui::Color32::from_rgba_unmultiplied(
                    palette.primary.r(),
                    palette.primary.g(),
                    palette.primary.b(),
                    if pressed { 40 } else { 24 },
                )
            } else if open || pressed {
                palette.control
            } else {
                egui::Color32::from_rgba_unmultiplied(
                    palette.control.r(),
                    palette.control.g(),
                    palette.control.b(),
                    80,
                )
            },
        );
    }
    let stroke_color = if insertion || open || hovered {
        palette.primary
    } else if can_add {
        palette.grid
    } else {
        palette.grid.gamma_multiply(0.48)
    };
    let stroke = egui::Stroke::new(
        if open || pressed {
            editor_theme::shape::FOCUS_STROKE
        } else {
            editor_theme::shape::STROKE
        },
        stroke_color,
    );
    let outline = [
        visual.left_top(),
        visual.right_top(),
        visual.right_bottom(),
        visual.left_bottom(),
        visual.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &outline,
        stroke,
        editor_theme::space::XS,
        editor_theme::space::XXS,
    ));
    ui.painter().text(
        visual.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        editor_theme::font::title(),
        if insertion || (can_add && (hovered || open || pressed)) {
            palette.primary
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
                    .stroke(egui::Stroke::new(editor_theme::shape::STROKE, palette.grid))
                    .inner_margin(egui::Margin::same(editor_theme::space::XS as i8))
                    .show(ui, |ui| {
                        ui.set_min_width(popup_width);
                        let free = (0..MAX_MODULATION_SOURCES)
                            .find(|index| *active & (1_u64 << index) == 0);
                        let lfo_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                        });
                        let lfo_chosen = menu_choice(
                            ui,
                            1,
                            "LFO",
                            free.is_some(),
                            popup_width,
                            editor_theme::title_height(ui),
                            palette.primary,
                        ) || (free.is_some() && lfo_key);
                        if lfo_chosen && let Some(index) = free {
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
                        let envelope_chosen = menu_choice(
                            ui,
                            2,
                            "ENVELOPE",
                            free.is_some(),
                            popup_width,
                            editor_theme::title_height(ui),
                            palette.primary,
                        ) || (free.is_some() && envelope_key);
                        if envelope_chosen && let Some(index) = free {
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
