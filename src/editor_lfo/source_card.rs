use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_modulation::{clear_source, source_color, source_handle};
use crate::modulators::state::{LEGACY_MODULATION_SOURCES, SourceKind};
use crate::wave_curve::WaveCurveState;
use crate::{KurvParams, editor_theme};

use super::controls::{collapsed_source_summary, draw_controls, draw_envelope_controls};
use super::envelope_editor::{draw_envelope_curve, envelope_path};
use super::source::{
    envelope_curve_values, envelope_values, lfo_curve, lfo_params, set_source_active,
    source_is_envelope, source_value_meter,
};
use super::spline_editor::{draw_curve, draw_in_rect};
use super::{ModulationUi, ModulatorReorder, first_presented_active_source, rack_item_visible};

pub(super) fn draw_source_module(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    view: &mut ModulationUi,
    active: &mut u64,
    index: usize,
    presentation_index: usize,
    collapsed: bool,
    width: f32,
    height: f32,
    keep_interaction_alive: bool,
) {
    let mut collapsed = collapsed;
    let header_height = collapsed_module_height(ui);
    let shown_height = if collapsed { header_height } else { height };
    let (_, rect) = ui.allocate_space(egui::vec2(width, shown_height));
    if !rack_item_visible(ui, rect) && !keep_interaction_alive {
        return;
    }
    let palette = editor_theme::semantic();
    let color = source_color(index);
    let mut selected = view.selected == index;
    let envelope = source_is_envelope(state, index);
    let source_label = format!("{} {}", if envelope { "ENV" } else { "LFO" }, index + 1);
    let card_hovered = ui.rect_contains_pointer(rect);
    if card_hovered && ui.input(|input| input.pointer.primary_clicked()) {
        view.selected = index;
        selected = true;
    }
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
    let action_size = header.height();
    let collapse_rect = egui::Rect::from_center_size(
        header.left_center() + egui::vec2(action_size * 0.5, 0.0),
        egui::Vec2::splat(action_size),
    );
    let remove_rect = egui::Rect::from_center_size(
        header.right_center() - egui::vec2(action_size * 0.5, 0.0),
        egui::Vec2::splat(action_size),
    );
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), header.top()),
        egui::pos2(remove_rect.left(), header.bottom()),
    );
    let grip_width =
        (editor_theme::space::XS + editor_theme::space::SM).min(drag_rect.width() * 0.34);
    let grip_rect = egui::Rect::from_min_max(
        drag_rect.min,
        egui::pos2(drag_rect.left() + grip_width, drag_rect.bottom()),
    );
    let source_label_width = ui
        .painter()
        .layout_no_wrap(source_label.clone(), editor_theme::font::label(), color)
        .size()
        .x;
    let source_width = (source_label_width + action_size * 0.72 + editor_theme::space::XS * 2.0)
        .min((drag_rect.width() - grip_width).max(0.0));
    let source_rect = egui::Rect::from_min_max(
        egui::pos2(grip_rect.right(), drag_rect.top()),
        egui::pos2(grip_rect.right() + source_width, drag_rect.bottom()),
    );
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(source_rect.right(), drag_rect.top()),
        drag_rect.max,
    );
    let header_id = ui.id().with(("lfo-module", index));
    let header_response = ui
        .interact(title_rect, header_id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Select this modulator; double-click to collapse");
    let source_response = ui.interact(
        source_rect,
        header_id.with("source"),
        egui::Sense::click_and_drag(),
    );
    let grip_response = ui
        .interact(
            grip_rect,
            header_id.with("reorder"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to reorder this modulator card");
    let collapse = ui
        .interact(
            collapse_rect,
            header_id.with("collapse"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Expand modulator"
        } else {
            "Collapse modulator"
        });
    let remove = ui
        .interact(remove_rect, header_id.with("remove"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove modulator and its routes");
    if grip_response.drag_started() {
        view.selected = index;
        selected = true;
        view.reorder = Some(ModulatorReorder {
            source_slot: index,
            presentation_insertion: presentation_index,
            card_size: rect.size(),
            header_height,
            collapsed,
        });
    }
    if grip_response.dragged()
        && let Some(drag) = view
            .reorder
            .as_mut()
            .filter(|drag| drag.source_slot == index)
    {
        drag.card_size = rect.size();
        drag.header_height = header_height;
        drag.collapsed = collapsed;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        editor_theme::request_display_repaint(ui);
    }
    if source_response.is_pointer_button_down_on()
        || source_response.drag_started()
        || source_response.clicked()
    {
        view.selected = index;
        selected = true;
    }
    let source_active = source_response.dragged() || source_response.is_pointer_button_down_on();
    let reorder_active = view.reorder.is_some_and(|drag| drag.source_slot == index);
    let dot_radius = editor_theme::shape::STROKE;
    let grip_spacing = editor_theme::space::XXS;
    let origin = grip_rect.center() - egui::vec2(grip_spacing * 0.5, grip_spacing);
    let grip_color = if reorder_active {
        palette.text
    } else if grip_response.hovered() {
        color
    } else {
        palette.text_muted.gamma_multiply(0.56)
    };
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                origin + egui::vec2(column as f32 * grip_spacing, row as f32 * grip_spacing),
                dot_radius,
                grip_color,
            );
        }
    }
    source_handle(ui, state, index, &source_label, &source_response)
        .on_hover_text("Drag this source onto a highlighted parameter");
    let keyboard_activate = ui
        .input(|input| input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space));
    if collapse.clicked()
        || (collapse.has_focus() && keyboard_activate)
        || header_response.double_clicked()
    {
        collapsed = !collapsed;
        set_modulator_collapsed(state, index, collapsed);
        editor_theme::request_display_repaint(ui);
    }
    if remove.clicked()
        || (remove.has_focus()
            && (keyboard_activate || ui.input(|input| input.key_pressed(egui::Key::Delete))))
    {
        clear_source(state, (index + 1) as u8);
        *active &= !(1_u64 << index);
        set_source_active(state, index, false, SourceKind::Lfo);
        view.selected = if *active == 0 {
            0
        } else {
            first_presented_active_source(state, *active).unwrap_or_default()
        };
        return;
    }
    if header_response.clicked() {
        view.selected = index;
    }
    let marker_center = collapse_rect.center();
    let marker_size = collapse_rect.height() * 0.30;
    let marker_points = if collapsed {
        vec![
            marker_center + egui::vec2(-marker_size * 0.36, -marker_size * 0.56),
            marker_center + egui::vec2(-marker_size * 0.36, marker_size * 0.56),
            marker_center + egui::vec2(marker_size * 0.52, 0.0),
        ]
    } else {
        vec![
            marker_center + egui::vec2(-marker_size * 0.56, -marker_size * 0.36),
            marker_center + egui::vec2(marker_size * 0.56, -marker_size * 0.36),
            marker_center + egui::vec2(0.0, marker_size * 0.52),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        marker_points,
        if collapse.hovered() || collapse.is_pointer_button_down_on() || collapse.has_focus() {
            color
        } else {
            palette.text_muted
        },
        egui::Stroke::NONE,
    ));
    if title_rect.width() > header.height() * 5.0 {
        let text = if source_active {
            if drag_rect.width() > header.height() * 8.0 {
                "DROP ON CONTROL".to_owned()
            } else {
                "DRAG".to_owned()
            }
        } else if source_response.hovered() {
            "DRAG TO MODULATE".to_owned()
        } else if collapsed {
            collapsed_source_summary(state, index, envelope)
        } else if envelope {
            format!(
                "{:.0}%",
                source_value_meter(state, index).clamp(0.0, 1.0) * 100.0
            )
        } else {
            format!("{:+.2}", source_value_meter(state, index).clamp(-1.0, 1.0))
        };
        let text_font = editor_theme::font::caption();
        let text_width = ui
            .painter()
            .layout_no_wrap(text.clone(), text_font.clone(), palette.text_muted)
            .size()
            .x;
        if text_width + editor_theme::space::XS * 2.0 < title_rect.width() {
            ui.painter().text(
                title_rect.right_center() - egui::vec2(editor_theme::space::XS, 0.0),
                egui::Align2::RIGHT_CENTER,
                text,
                text_font,
                if source_active {
                    palette.text
                } else if source_response.hovered() {
                    color
                } else {
                    palette.text_muted
                },
            );
        }
    }
    ui.painter().text(
        remove_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::caption(),
        if remove.hovered() || remove.is_pointer_button_down_on() || remove.has_focus() {
            palette.danger
        } else if selected || card_hovered {
            palette.text_muted
        } else {
            palette.text_muted.gamma_multiply(0.44)
        },
    );
    if collapsed {
        paint_reorder_origin(ui, source_rect, None, reorder_active, color);
        return;
    }

    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left(), header.bottom()),
        rect.right_bottom(),
    );
    let gap = editor_theme::compact_gap(ui).min(body.width() * 0.02);
    let controls_width = (body.width() * 0.20)
        .max(header_height * 4.0)
        .min(body.width() * 0.30);
    let controls = egui::Rect::from_min_max(
        egui::pos2(body.right() - controls_width, body.top()),
        body.right_bottom(),
    );
    let graph = egui::Rect::from_min_max(
        body.min,
        egui::pos2((controls.left() - gap).max(body.left()), body.bottom()),
    );
    draw_in_rect(ui, graph, ("source-graph", index), |ui| {
        if envelope {
            draw_envelope_curve(ui, state, index, graph.width(), graph.height());
        } else {
            draw_curve(ui, state, index, graph.width(), graph.height());
        }
    });
    draw_in_rect(ui, controls, ("source-controls", index), |ui| {
        if envelope {
            draw_envelope_controls(ui, state, index, controls.width(), controls.height());
        } else {
            draw_controls(ui, state, index, controls.width(), controls.height());
        }
    });
    paint_reorder_origin(ui, source_rect, Some(body), reorder_active, color);
}

pub(super) fn expanded_module_height(ui: &egui::Ui) -> f32 {
    let metric_row_height = editor_theme::font::CAPTION_SIZE
        + editor_theme::font::VALUE_SIZE
        + editor_theme::compact_gap(ui)
        + editor_theme::shape::STROKE;
    collapsed_module_height(ui) + metric_row_height * 5.0 + editor_theme::space::XS * 2.0
}

pub(super) fn collapsed_module_height(ui: &egui::Ui) -> f32 {
    editor_theme::font::VALUE_SIZE
        + editor_theme::compact_gap(ui) * 2.0
        + editor_theme::shape::STROKE * 2.0
}

fn set_modulator_collapsed(state: &PluginContext<KurvParams>, index: usize, collapsed: bool) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        let bit = 1_u64 << index;
        if collapsed {
            editor.collapsed_modulators |= bit;
        } else {
            editor.collapsed_modulators &= !bit;
        }
    }
}

fn paint_reorder_origin(
    ui: &egui::Ui,
    identity: egui::Rect,
    body: Option<egui::Rect>,
    active: bool,
    color: egui::Color32,
) {
    if !active {
        return;
    }
    if let Some(body) = body {
        ui.painter()
            .rect_filled(body, 0.0, egui::Color32::from_black_alpha(72));
    }
    ui.painter().line_segment(
        [identity.left_bottom(), identity.right_bottom()],
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    );
}

pub(super) fn paint_modulator_drag_ghost(
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
