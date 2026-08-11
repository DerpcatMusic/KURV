use super::*;

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
    } else {
        response.on_hover_text("Modulator limit reached; remove a source to add another")
    };
    if insertion {
        ui.painter().rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Color32::from_rgba_unmultiplied(
                palette.primary.r(),
                palette.primary.g(),
                palette.primary.b(),
                22,
            ),
        );
    }
    let stroke_color = if insertion || (can_add && response.hovered()) {
        palette.primary
    } else if can_add {
        palette.grid
    } else {
        palette.grid.gamma_multiply(0.48)
    };
    let stroke = egui::Stroke::new(
        if open {
            editor_theme::shape::FOCUS_STROKE
        } else {
            editor_theme::shape::STROKE
        },
        stroke_color,
    );
    let outline = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &outline,
        stroke,
        rect.height() * 0.42,
        rect.height() * 0.30,
    ));
    ui.painter().text(
        rect.left_center() + egui::vec2(rect.height() * 0.5, 0.0),
        egui::Align2::LEFT_CENTER,
        if can_add {
            "+ ADD MODULATOR".to_owned()
        } else {
            format!("{MAX_MODULATION_SOURCES} MODULATORS · LIMIT")
        },
        editor_theme::font::label(),
        if insertion {
            palette.primary
        } else if can_add && (response.hovered() || open) {
            palette.text
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
                        if ui
                            .add_enabled(
                                free.is_some(),
                                egui::Button::new("1   LFO").min_size(egui::vec2(
                                    popup_width,
                                    editor_theme::title_height(ui),
                                )),
                            )
                            .clicked()
                            || (free.is_some() && lfo_key)
                        {
                            let index = free.expect("enabled only when an LFO slot is free");
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
                        if ui
                            .add_enabled(
                                free.is_some(),
                                egui::Button::new("2   ENVELOPE").min_size(egui::vec2(
                                    popup_width,
                                    editor_theme::title_height(ui),
                                )),
                            )
                            .clicked()
                            || (free.is_some() && envelope_key)
                        {
                            let index = free.expect("enabled only when a source slot is free");
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
