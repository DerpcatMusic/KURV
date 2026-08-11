use truce_core::editor::PluginContext;

use crate::editor_modulation::{clear_source, source_color, source_handle};
use crate::modulators::state::SourceKind;
use crate::{KurvParams, editor_theme};

use super::controls::{collapsed_source_summary, draw_controls, draw_envelope_controls};
use super::envelope_editor::draw_envelope_curve;
use super::source::{set_source_active, source_is_envelope};
use super::spline_editor::{draw_curve, draw_in_rect};
use super::{ModulationUi, ModulatorReorder, first_presented_active_source, rack_item_visible};

mod drag_preview;

pub(super) use drag_preview::paint_modulator_drag_ghost;

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
            Some(if drag_rect.width() > header.height() * 8.0 {
                "DROP ON CONTROL".to_owned()
            } else {
                "DRAG".to_owned()
            })
        } else if source_response.hovered() {
            Some("DRAG TO MODULATE".to_owned())
        } else if collapsed {
            Some(collapsed_source_summary(state, index, envelope))
        } else {
            None
        };
        if let Some(text) = text {
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
