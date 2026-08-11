use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_modulation::source_color;
use crate::modulators::state::LEGACY_MODULATION_SOURCES;
use crate::wave_curve::WaveCurveState;
use crate::{KurvParams, editor_theme};

use super::super::ModulatorReorder;
use super::super::envelope_editor::envelope_path;
use super::super::source::{
    envelope_curve_values, envelope_values, lfo_curve, lfo_params, source_is_envelope,
};

pub(in crate::editor_lfo) fn paint_modulator_drag_ghost(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    drag: ModulatorReorder,
) {
    let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) else {
        return;
    };
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
    let scale = 0.62_f32
        .min(screen.width() * 0.56 / drag.card_size.x.max(1.0))
        .min(screen.height() * 0.42 / drag.card_size.y.max(1.0));
    let size = drag.card_size * scale;
    let offset = egui::vec2(drag.header_height * scale, drag.header_height * scale);
    let mut rect = egui::Rect::from_min_size(pointer + offset, size);
    if rect.right() > screen.right() {
        rect = egui::Rect::from_min_size(pointer - egui::vec2(size.x + offset.x, -offset.y), size);
    }
    rect = rect.translate(egui::vec2(
        (screen.left() - rect.left()).max(0.0) - (rect.right() - screen.right()).max(0.0),
        (screen.top() - rect.top()).max(0.0) - (rect.bottom() - screen.bottom()).max(0.0),
    ));

    let palette = editor_theme::semantic();
    let color = source_color(drag.source_slot);
    let envelope = source_is_envelope(state, drag.source_slot);
    let label = format!(
        "{} {}",
        if envelope { "ENV" } else { "LFO" },
        drag.source_slot + 1
    );
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new(("modulator-reorder-ghost", drag.source_slot)),
    ));
    painter.rect_filled(rect, editor_theme::shape::CONTROL_RADIUS, palette.surface);
    painter.rect_stroke(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
        egui::StrokeKind::Inside,
    );

    let header_height = (drag.header_height * scale).min(rect.height());
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
    painter.rect_filled(
        header,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 26),
    );
    let text_inset = header.height() * 0.46;
    painter.text(
        egui::pos2(header.left() + text_inset, header.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        palette.text,
    );
    painter.text(
        egui::pos2(header.right() - text_inset, header.center().y),
        egui::Align2::RIGHT_CENTER,
        "MOVE",
        editor_theme::font::caption(),
        color,
    );
    if drag.collapsed || rect.height() <= header.height() {
        return;
    }

    let controls_height = header.height();
    let graph = egui::Rect::from_min_max(
        egui::pos2(rect.left(), header.bottom()),
        egui::pos2(rect.right(), rect.bottom() - controls_height),
    )
    .shrink2(egui::vec2(header.height() * 0.28, header.height() * 0.20));
    if !graph.is_positive() {
        return;
    }
    let points = if envelope {
        envelope_ghost_points(state, drag.source_slot, graph)
    } else {
        lfo_ghost_points(state, drag.source_slot, graph)
    };
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    ));
}

fn envelope_ghost_points(
    state: &PluginContext<KurvParams>,
    index: usize,
    plot: egui::Rect,
) -> Vec<egui::Pos2> {
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let curves = envelope_curve_values(state.params(), index);
    let weight = |seconds: f32| (seconds.max(0.0) + f32::EPSILON).sqrt();
    let weights = [weight(attack), weight(decay), 0.32, weight(release)];
    let total: f32 = weights.iter().sum();
    let attack_x = plot.left() + plot.width() * weights[0] / total;
    let decay_x = attack_x + plot.width() * weights[1] / total;
    let sustain_x = decay_x + plot.width() * weights[2] / total;
    let sustain_y = egui::lerp(plot.bottom()..=plot.top(), sustain.clamp(0.0, 1.0));
    let points = [
        plot.left_bottom(),
        egui::pos2(attack_x, plot.top()),
        egui::pos2(decay_x, sustain_y),
        egui::pos2(sustain_x, sustain_y),
        plot.right_bottom(),
    ];
    envelope_path(&points, curves)
}

fn lfo_ghost_points(
    state: &PluginContext<KurvParams>,
    index: usize,
    plot: egui::Rect,
) -> Vec<egui::Pos2> {
    let curve = if index < LEGACY_MODULATION_SOURCES {
        Some(lfo_curve(state.params(), index))
    } else {
        state.params().modulator_rack.curve(index)
    };
    let compiled = curve
        .and_then(WaveCurveState::try_curve_rt)
        .unwrap_or_default();
    let bipolar = if index < LEGACY_MODULATION_SOURCES {
        state.get_param(lfo_params(index).bipolar) >= 0.5
    } else {
        state.params().modulator_rack.config(index).bipolar
    };
    let segments = (plot.width() / editor_theme::space::SM).round().max(4.0) as usize;
    (0..=segments)
        .map(|point| {
            let phase = point as f32 / segments as f32;
            let value = compiled.eval(phase);
            let y = if bipolar {
                plot.center().y - value * plot.height() * 0.46
            } else {
                egui::lerp(plot.bottom()..=plot.top(), value.mul_add(0.5, 0.5))
            };
            egui::pos2(egui::lerp(plot.left()..=plot.right(), phase), y)
        })
        .collect()
}
