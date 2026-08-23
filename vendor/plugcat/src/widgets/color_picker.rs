//! Compact HSV/RGB color selection for plugin popups.
//!
//! The picker intentionally separates hue from the saturation/value plane and
//! gives both controls keyboard focus. Arrow keys edit the focused control;
//! Shift makes fine adjustments and Command/Ctrl makes coarse adjustments.

use std::hash::Hash;

use egui::ecolor::Hsva;
use egui::{
    Color32, DragValue, EventFilter, Key, Mesh, Popup, PopupCloseBehavior, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
};

use super::tokens::{DARK_TOKENS, LIGHT_TOKENS, WidgetTokens};

const PICKER_WIDTH: f32 = 184.0;
const SV_HEIGHT: f32 = 116.0;
const HUE_HEIGHT: f32 = 16.0;
const SV_STEPS: u32 = 12;
const HUE_STEPS: u32 = 12;

#[derive(Clone, Copy)]
struct PickerState {
    hsva: Hsva,
    rendered_color: Color32,
}

/// Show a compact opaque color picker using tokens inferred from the current UI mode.
///
/// The returned response is marked changed whenever pointer, keyboard, or RGB
/// entry changes `color`.
pub fn color_picker(ui: &mut Ui, id_salt: impl Hash, color: &mut Color32) -> Response {
    let tokens = if ui.visuals().dark_mode {
        DARK_TOKENS
    } else {
        LIGHT_TOKENS
    };
    color_picker_with_tokens(ui, id_salt, color, tokens)
}

/// Show a compact opaque color picker with explicit Plugcat widget tokens.
pub fn color_picker_with_tokens(
    ui: &mut Ui,
    id_salt: impl Hash,
    color: &mut Color32,
    tokens: WidgetTokens,
) -> Response {
    let state_id = ui.id().with(id_salt).with("plugcat-color-picker-state");
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<PickerState>(state_id))
        .filter(|state| state.rendered_color == *color)
        .unwrap_or_else(|| PickerState {
            hsva: Hsva::from(*color),
            rendered_color: *color,
        });
    state.hsva.a = 1.0;

    let mut changed = false;
    let inner = ui.vertical(|ui| {
        ui.set_width(PICKER_WIDTH);
        ui.spacing_mut().item_spacing.y = tokens.spacing.xs.max(3.0);

        header(ui, Color32::from(state.hsva), tokens);
        ui.label(
            egui::RichText::new("SATURATION · VALUE")
                .size(10.0)
                .color(tokens.colors.muted),
        );
        let sv_response = saturation_value_control(ui, &mut state.hsva, tokens);
        changed |= sv_response.changed();

        ui.label(
            egui::RichText::new("HUE")
                .size(10.0)
                .color(tokens.colors.muted),
        );
        let hue_response = hue_control(ui, &mut state.hsva, tokens);
        changed |= hue_response.changed();

        changed |= rgb_controls(ui, &mut state.hsva, tokens);
    });

    let next_color = Color32::from(state.hsva);
    if next_color != *color {
        *color = next_color;
        changed = true;
    }
    state.rendered_color = *color;
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));

    let mut response = inner.response;
    if changed {
        response.mark_changed();
    }
    response
}

/// Show a compact color-picker popup anchored to `trigger`.
///
/// Pointer clicks on the trigger are handled by [`Popup`]. Callers that offer
/// keyboard activation should toggle `Popup::default_response_id(trigger)` on
/// Enter/Space before calling this function. Returns `true` while the selected
/// color changes; the popup stays open during edits.
pub fn color_picker_popup(ui: &mut Ui, trigger: &Response, color: &mut Color32) -> bool {
    let tokens = if ui.visuals().dark_mode {
        DARK_TOKENS
    } else {
        LIGHT_TOKENS
    };
    color_picker_popup_with_tokens(ui, trigger, color, tokens)
}

/// Show a compact color-picker popup with explicit Plugcat widget tokens.
pub fn color_picker_popup_with_tokens(
    _ui: &mut Ui,
    trigger: &Response,
    color: &mut Color32,
    tokens: WidgetTokens,
) -> bool {
    Popup::menu(trigger)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(PICKER_WIDTH + tokens.spacing.sm * 2.0)
        .show(|ui| {
            ui.set_min_width(PICKER_WIDTH);
            color_picker_with_tokens(ui, trigger.id, color, tokens).changed()
        })
        .is_some_and(|shown| shown.inner)
}

fn header(ui: &mut Ui, color: Color32, tokens: WidgetTokens) {
    ui.horizontal(|ui| {
        let (preview, _) = ui.allocate_exact_size(egui::vec2(34.0, 20.0), Sense::hover());
        ui.painter()
            .rect_filled(preview, tokens.radius.control, color);
        ui.painter().rect_stroke(
            preview,
            tokens.radius.control,
            Stroke::new(tokens.stroke.control, tokens.colors.border),
            StrokeKind::Inside,
        );
        ui.label(
            egui::RichText::new(format!(
                "#{:02X}{:02X}{:02X}",
                color.r(),
                color.g(),
                color.b()
            ))
            .monospace()
            .color(tokens.colors.text),
        );
    });
}

fn saturation_value_control(ui: &mut Ui, hsva: &mut Hsva, tokens: WidgetTokens) -> Response {
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(PICKER_WIDTH, SV_HEIGHT), Sense::click_and_drag());
    response.widget_info(|| {
        WidgetInfo::slider(true, f64::from(hsva.s), "Saturation and value color plane")
    });

    let before = (hsva.s, hsva.v);
    if let Some(pointer) = response.interact_pointer_pos() {
        hsva.s = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsva.v = ((rect.bottom() - pointer.y) / rect.height()).clamp(0.0, 1.0);
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let step = keyboard_step(ui);
        ui.input(|input| {
            if input.key_pressed(Key::ArrowLeft) {
                hsva.s = (hsva.s - step).max(0.0);
            }
            if input.key_pressed(Key::ArrowRight) {
                hsva.s = (hsva.s + step).min(1.0);
            }
            if input.key_pressed(Key::ArrowDown) {
                hsva.v = (hsva.v - step).max(0.0);
            }
            if input.key_pressed(Key::ArrowUp) {
                hsva.v = (hsva.v + step).min(1.0);
            }
            if input.key_pressed(Key::Home) {
                hsva.s = 0.0;
                hsva.v = 0.0;
            }
            if input.key_pressed(Key::End) {
                hsva.s = 1.0;
                hsva.v = 1.0;
            }
        });
    }
    if before != (hsva.s, hsva.v) {
        response.mark_changed();
    }

    paint_sv_plane(ui, rect, *hsva, tokens, response.has_focus());
    response.on_hover_text("Saturation left/right · Value down/up · Arrow keys adjust")
}

fn hue_control(ui: &mut Ui, hsva: &mut Hsva, tokens: WidgetTokens) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(PICKER_WIDTH, HUE_HEIGHT),
        Sense::click_and_drag(),
    );
    response.widget_info(|| WidgetInfo::slider(true, f64::from(hsva.h), "Hue"));

    let before = hsva.h;
    if let Some(pointer) = response.interact_pointer_pos() {
        hsva.h = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
        let step = keyboard_step(ui);
        ui.input(|input| {
            if input.key_pressed(Key::ArrowLeft) {
                hsva.h = (hsva.h - step).max(0.0);
            }
            if input.key_pressed(Key::ArrowRight) {
                hsva.h = (hsva.h + step).min(1.0);
            }
            if input.key_pressed(Key::Home) {
                hsva.h = 0.0;
            }
            if input.key_pressed(Key::End) {
                hsva.h = 1.0;
            }
        });
    }
    if before != hsva.h {
        response.mark_changed();
    }

    paint_hue_bar(ui, rect, hsva.h, tokens, response.has_focus());
    response.on_hover_text("Hue · Left/right arrow keys adjust")
}

fn rgb_controls(ui: &mut Ui, hsva: &mut Hsva, tokens: WidgetTokens) -> bool {
    let color = Color32::from(*hsva);
    let mut rgb = [color.r(), color.g(), color.b()];
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tokens.spacing.xs.max(3.0);
        for (label, channel) in ["R", "G", "B"].into_iter().zip(&mut rgb) {
            changed |= ui
                .add_sized(
                    egui::vec2(58.0, 20.0),
                    DragValue::new(channel)
                        .range(0..=255)
                        .prefix(format!("{label} ")),
                )
                .on_hover_text(format!("{label} channel (0–255)"))
                .changed();
        }
    });
    if changed {
        *hsva = hsva_from_rgb_preserving_hue(rgb, hsva.h);
    }
    changed
}

fn paint_sv_plane(ui: &Ui, rect: egui::Rect, hsva: Hsva, tokens: WidgetTokens, focused: bool) {
    if !ui.is_rect_visible(rect) || !rect.is_positive() {
        return;
    }
    let mut mesh = Mesh::default();
    for y in 0..=SV_STEPS {
        for x in 0..=SV_STEPS {
            let saturation = x as f32 / SV_STEPS as f32;
            let value = 1.0 - y as f32 / SV_STEPS as f32;
            let point = egui::pos2(
                egui::lerp(rect.left()..=rect.right(), saturation),
                egui::lerp(rect.top()..=rect.bottom(), 1.0 - value),
            );
            mesh.colored_vertex(
                point,
                Hsva {
                    h: hsva.h,
                    s: saturation,
                    v: value,
                    a: 1.0,
                }
                .into(),
            );
            if x < SV_STEPS && y < SV_STEPS {
                let stride = SV_STEPS + 1;
                let top_left = y * stride + x;
                mesh.add_triangle(top_left, top_left + 1, top_left + stride);
                mesh.add_triangle(top_left + 1, top_left + stride, top_left + stride + 1);
            }
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter().rect_stroke(
        rect,
        tokens.radius.control,
        Stroke::new(
            if focused { 2.0 } else { tokens.stroke.control },
            if focused {
                tokens.colors.accent
            } else {
                tokens.colors.border
            },
        ),
        StrokeKind::Inside,
    );

    let center = egui::pos2(
        egui::lerp(rect.left()..=rect.right(), hsva.s),
        egui::lerp(rect.bottom()..=rect.top(), hsva.v),
    );
    let selected: Color32 = hsva.into();
    ui.painter().circle(
        center,
        5.5,
        selected,
        Stroke::new(2.0_f32, contrast_color(selected)),
    );
}

fn paint_hue_bar(ui: &Ui, rect: egui::Rect, hue: f32, tokens: WidgetTokens, focused: bool) {
    if !ui.is_rect_visible(rect) || !rect.is_positive() {
        return;
    }
    let mut mesh = Mesh::default();
    for step in 0..=HUE_STEPS {
        let amount = step as f32 / HUE_STEPS as f32;
        let x = egui::lerp(rect.left()..=rect.right(), amount);
        let color: Color32 = Hsva {
            h: amount,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        }
        .into();
        mesh.colored_vertex(egui::pos2(x, rect.top()), color);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
        if step < HUE_STEPS {
            let index = step * 2;
            mesh.add_triangle(index, index + 1, index + 2);
            mesh.add_triangle(index + 1, index + 2, index + 3);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter().rect_stroke(
        rect,
        tokens.radius.control,
        Stroke::new(
            if focused { 2.0 } else { tokens.stroke.control },
            if focused {
                tokens.colors.accent
            } else {
                tokens.colors.border
            },
        ),
        StrokeKind::Inside,
    );
    let x = egui::lerp(rect.left()..=rect.right(), hue);
    let marker = egui::Rect::from_center_size(
        egui::pos2(x, rect.center().y),
        Vec2::new(5.0, rect.height() + 2.0),
    );
    ui.painter().rect_stroke(
        marker,
        1.0,
        Stroke::new(2.0_f32, tokens.colors.text),
        StrokeKind::Inside,
    );
}

fn keyboard_step(ui: &Ui) -> f32 {
    ui.input(|input| {
        if input.modifiers.shift {
            1.0 / 255.0
        } else if input.modifiers.command || input.modifiers.ctrl {
            0.05
        } else {
            0.01
        }
    })
}

fn hsva_from_rgb_preserving_hue(rgb: [u8; 3], previous_hue: f32) -> Hsva {
    let mut hsva = Hsva::from(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    if hsva.s <= f32::EPSILON {
        hsva.h = previous_hue;
    }
    hsva.a = 1.0;
    hsva
}

fn contrast_color(color: Color32) -> Color32 {
    let luminance =
        0.299 * f32::from(color.r()) + 0.587 * f32::from(color.g()) + 0.114 * f32::from(color.b());
    if luminance > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_conversion_keeps_previous_hue_for_gray() {
        let gray = hsva_from_rgb_preserving_hue([80, 80, 80], 0.73);

        assert!((gray.h - 0.73).abs() < f32::EPSILON);
    }

    #[test]
    fn rgb_conversion_is_opaque_and_round_trips_channels() {
        let hsva = hsva_from_rgb_preserving_hue([23, 147, 211], 0.0);
        let restored = Color32::from(hsva);

        assert_eq!([restored.r(), restored.g(), restored.b()], [23, 147, 211]);
        assert_eq!(restored.a(), 255);
    }

    #[test]
    fn contrast_marker_switches_for_light_and_dark_colors() {
        assert_eq!(contrast_color(Color32::BLACK), Color32::WHITE);
        assert_eq!(contrast_color(Color32::WHITE), Color32::BLACK);
    }
}
