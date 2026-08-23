use super::*;

use crate::modulators::state::GATE_STEP_COUNT;

/// Fixed-capacity 16-step editor. Click toggles a step; vertical drag sets that
/// step's deterministic trigger probability. The editor only publishes the
/// resulting fixed-size config on the UI thread.
pub(super) fn draw_gate_editor(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let palette = editor_theme::semantic();
    let color = source_color(index);
    let inset = editor_theme::graph_inset(ui);
    let plot = rect.shrink(inset);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, editor_theme::shape::CONTROL_RADIUS, palette.well);

    let meter = lfo_phase_meter(state, index).rem_euclid(1.0);
    let current_step = ((meter * GATE_STEP_COUNT as f32).floor() as usize).min(GATE_STEP_COUNT - 1);
    let gap = editor_theme::space::XXS;
    let step_width = (plot.width() - gap * (GATE_STEP_COUNT - 1) as f32) / GATE_STEP_COUNT as f32;
    let label_height = editor_theme::font::CAPTION_SIZE + editor_theme::space::XXS;
    let bar_area = egui::Rect::from_min_max(
        plot.min,
        egui::pos2(plot.right(), plot.bottom() - label_height),
    );
    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;

    for step in 0..GATE_STEP_COUNT {
        let left = bar_area.left() + step as f32 * (step_width + gap);
        let step_rect = egui::Rect::from_min_max(
            egui::pos2(left, bar_area.top()),
            egui::pos2(left + step_width.max(1.0), bar_area.bottom()),
        );
        let id = ui.id().with(("gate-step", index, step));
        let response = ui
            .interact(step_rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(format!(
                "Step {} · {}% probability\nClick to toggle; drag vertically to set probability; right-click resets",
                step + 1,
                config.gate_probabilities[step]
            ));
        let keyboard_toggle = response.has_focus()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
            });
        if response.clicked() || keyboard_toggle {
            config.gate_pattern ^= 1_u16 << step;
            changed = true;
        }
        if response.secondary_clicked() {
            config.gate_pattern |= 1_u16 << step;
            config.gate_probabilities[step] = 100;
            changed = true;
        } else if response.dragged()
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let probability = ((step_rect.bottom() - pointer.y) / step_rect.height() * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8;
            config.gate_pattern |= 1_u16 << step;
            if config.gate_probabilities[step] != probability {
                config.gate_probabilities[step] = probability;
                changed = true;
            }
        }

        let enabled = config.gate_pattern & (1_u16 << step) != 0;
        let probability = f32::from(config.gate_probabilities[step]) * 0.01;
        let fill_height = if enabled {
            (step_rect.height() * probability).max(editor_theme::shape::STROKE)
        } else {
            editor_theme::shape::STROKE
        };
        let fill = egui::Rect::from_min_max(
            egui::pos2(step_rect.left(), step_rect.bottom() - fill_height),
            step_rect.right_bottom(),
        );
        painter.rect_filled(
            step_rect,
            editor_theme::shape::CONTROL_RADIUS,
            if step == current_step {
                palette.control_hover
            } else {
                palette.control
            },
        );
        painter.rect_filled(
            fill,
            editor_theme::shape::CONTROL_RADIUS,
            if enabled {
                color.gamma_multiply(0.35 + probability * 0.65)
            } else {
                palette.disabled
            },
        );
        painter.rect_stroke(
            step_rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(
                if step == current_step {
                    editor_theme::shape::FOCUS_STROKE
                } else {
                    editor_theme::shape::STROKE
                },
                if step == current_step {
                    color
                } else {
                    palette.grid
                },
            ),
            egui::StrokeKind::Inside,
        );
        if step % 4 == 0 {
            painter.text(
                egui::pos2(step_rect.center().x, plot.bottom()),
                egui::Align2::CENTER_BOTTOM,
                (step + 1).to_string(),
                egui::FontId::proportional(editor_theme::font::CAPTION_SIZE),
                palette.text_muted,
            );
        }
    }

    if changed {
        state.params().modulator_rack.set_config(index, config);
        editor_theme::request_display_repaint(ui);
    }
}
