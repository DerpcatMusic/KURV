use super::*;
use crate::editor_widgets::{menu_choice, menu_section};

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
        let popup_width = (width * 0.62).min(screen.width());
        let popup_height = editor_theme::title_height(ui) * 2.0
            + editor_theme::space::XS
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
                        let free_standard = (0..MAX_MODULATION_SOURCES)
                            .find(|index| *active & (1_u64 << index) == 0);
                        let free_extended = (LEGACY_MODULATION_SOURCES..MAX_MODULATION_SOURCES)
                            .find(|index| *active & (1_u64 << index) == 0);
                        let macro_enabled =
                            free_extended.is_some() && macro_pack_mask(state, *active) == 0;
                        let item_width = (popup_width - editor_theme::space::SM) * 0.5;
                        let row_height = editor_theme::title_height(ui);
                        let mut chosen = None;
                        ui.columns(2, |columns| {
                            menu_section(&mut columns[0], "SHAPERS");
                            if menu_choice(
                                &mut columns[0],
                                "LFO",
                                free_standard.is_some(),
                                item_width,
                                row_height,
                                palette.primary,
                            ) {
                                chosen = Some(SourceKind::Lfo);
                            } else if menu_choice(
                                &mut columns[0],
                                "ENVELOPE",
                                free_standard.is_some(),
                                item_width,
                                row_height,
                                palette.primary,
                            ) {
                                chosen = Some(SourceKind::Envelope);
                            }
                            menu_section(&mut columns[1], "PERFORMANCE");
                            if menu_choice(
                                &mut columns[1],
                                "KEYTRACK",
                                free_extended.is_some(),
                                item_width,
                                row_height,
                                palette.primary,
                            ) {
                                chosen = Some(SourceKind::Keytrack);
                            } else if menu_choice(
                                &mut columns[1],
                                "MACROPACK",
                                macro_enabled,
                                item_width,
                                row_height,
                                palette.primary,
                            ) {
                                chosen = Some(SourceKind::Macro);
                            }
                        });
                        let free = match chosen {
                            Some(SourceKind::Lfo | SourceKind::Envelope) => free_standard,
                            Some(SourceKind::Keytrack | SourceKind::Macro) => free_extended,
                            Some(SourceKind::Button) | None => None,
                        };
                        if let (Some(kind), Some(index)) = (chosen, free) {
                            place_source_at_active_insertion(
                                state,
                                index,
                                *active,
                                presentation_insertion,
                            );
                            *active |= 1_u64 << index;
                            set_source_active(state, index, true, kind);
                            view.selected = index;
                            open = false;
                            view.add_menu = None;
                        }
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
