//! Unison distribution interaction coordinator.

use truce_core::editor::PluginContext;

use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::pan_curve::PanShapeCurveState;
use crate::voices::{MAX_UNISON, UnisonAlignmentMode};
use crate::{KurvParams, editor_theme};

use super::{host_axes_context_menu, update_host_axis};
mod layout;
mod painting;
mod preview;
mod selector;

use layout::compact_unison_layout;
use painting::paint_compact_distribution;
use preview::compact_unison_preview_points;
use selector::{compact_alignment_mode_combo, paint_vertical_selector_state};

pub(crate) use selector::{paint_vertical_selector, vertical_selector_value};

pub(crate) fn custom_unison_distribution_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    width: f32,
    height: f32,
    config: &mut crate::generators::OscillatorConfig,
    pan_shape_curve: &PanShapeCurveState,
) -> bool {
    let (outer, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::hover(),
    );
    let (header, _, unison_plot, alignment_rail) = compact_unison_layout(outer.rect);
    let before = (
        config.unison_amount.to_bits(),
        config.unison_curve.to_bits(),
        config.unison_alignment.to_bits(),
        config.unison_alignment_mode,
        config.unison_pan_curve.to_bits(),
        config.unison_pan_center_x.to_bits(),
        config.unison_stereo_x.to_bits(),
        config.unison_stereo_alternate.to_bits(),
    );
    let palette = editor_theme::semantic();
    let alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
    let mode_width = (editor_theme::font::VALUE_SIZE * 6.0)
        .min((alignment_rail.left() - header.left()).max(1.0) * 0.42);
    let mode_rect = egui::Rect::from_min_max(
        egui::pos2(
            (alignment_rail.left() - editor_theme::space::XXS - mode_width).max(header.left()),
            header.top(),
        ),
        egui::pos2(
            (alignment_rail.left() - editor_theme::space::XXS).max(header.left()),
            header.bottom(),
        ),
    );
    if let Some(mode) = compact_alignment_mode_combo(
        ui,
        mode_rect,
        outer.id.with("alignment-mode"),
        alignment_mode,
    ) {
        config.unison_alignment_mode = mode.index();
    }

    let distribution_id = outer.id.with("distribution");
    let response = ui.interact(
        unison_plot,
        distribution_id,
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    response.clone().on_hover_text(
        "Drag horizontally for detune; vertically for distribution curve. Double-click to reset.",
    );
    let distribution_hovered = response.hovered();
    let distribution_active = response.dragged() || response.is_pointer_button_down_on();
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = if distribution_active {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
    if (response.drag_started() || response.dragged())
        && let Some(position) = response.interact_pointer_pos()
    {
        config.unison_amount =
            ((position.x - unison_plot.left()) / unison_plot.width()).clamp(0.0, 1.0);
        config.unison_curve = ((unison_plot.bottom() - position.y) / unison_plot.height())
            .clamp(0.0, 1.0)
            .mul_add(2.0, -1.0);
    } else if response.double_clicked() {
        config.unison_amount = 1.0;
        config.unison_curve = 0.432_959_4;
    }

    let alignment_response = ui
        .interact(
            alignment_rail,
            outer.id.with("alignment-amount"),
            egui::Sense::CLICK | egui::Sense::DRAG,
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Unison alignment amount. Double-click to reset.");
    if alignment_response.double_clicked() {
        config.unison_alignment = crate::generators::OscillatorConfig::default().unison_alignment;
    } else if (alignment_response.drag_started()
        || alignment_response.dragged()
        || alignment_response.clicked())
        && let Some(pointer) = alignment_response.interact_pointer_pos()
    {
        config.unison_alignment = vertical_selector_value(alignment_rail, pointer);
    }
    let amount_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::UnisonAmount);
    let curve_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::UnisonCurve);
    let alignment_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::UnisonAlignment);
    let amount = OscillatorControl::UnisonAmount.normalized_value(*config);
    let curve = OscillatorControl::UnisonCurve.normalized_value(*config);
    let alignment = OscillatorControl::UnisonAlignment.normalized_value(*config);
    host_axes_context_menu(
        &response,
        state,
        &[
            ("X · DETUNE", amount_target, amount),
            ("Y · DISTRIBUTION", curve_target, curve),
        ],
    );
    crate::editor_modulation::host_automation_destination(
        ui,
        state,
        alignment_target,
        &alignment_response,
        alignment,
    );
    update_host_axis(
        ui,
        state,
        amount_target,
        &response,
        amount,
        before.0 != config.unison_amount.to_bits(),
    );
    update_host_axis(
        ui,
        state,
        curve_target,
        &response,
        curve,
        before.1 != config.unison_curve.to_bits(),
    );
    update_host_axis(
        ui,
        state,
        alignment_target,
        &alignment_response,
        alignment,
        before.2 != config.unison_alignment.to_bits(),
    );
    paint_vertical_selector_state(
        &painter,
        alignment_rail,
        config.unison_alignment,
        palette.unison,
        alignment_response.hovered(),
        alignment_response.dragged() || alignment_response.is_pointer_button_down_on(),
    );

    let voices_u8 = config.unison_voices.clamp(1, MAX_UNISON as u8);
    let voices = usize::from(voices_u8);
    let jitter_active = config.unison_jitter > f32::EPSILON;
    let time = if jitter_active {
        ui.input(|input| input.time) as f32 * normalized_unison_rate(config.unison_rate)
    } else {
        0.0
    };
    if jitter_active {
        editor_theme::request_display_repaint(ui);
    }
    let weights = [1.0_f32; MAX_UNISON];
    let pan_segments = pan_shape_curve.segments_rt();
    let preview_id = outer.id.with("unison-preview");
    let points = compact_unison_preview_points(
        ui,
        preview_id,
        config,
        unison_plot,
        pan_segments,
        time,
        jitter_active,
    );
    paint_compact_distribution(
        &painter,
        unison_plot,
        &points[..voices],
        &weights[..voices],
        1.0,
        egui::pos2(
            egui::lerp(
                unison_plot.left()..=unison_plot.right(),
                config.unison_amount,
            ),
            egui::lerp(
                unison_plot.bottom()..=unison_plot.top(),
                config.unison_curve.mul_add(0.5, 0.5),
            ),
        ),
        1.0,
        distribution_hovered,
        distribution_active,
    );
    before
        != (
            config.unison_amount.to_bits(),
            config.unison_curve.to_bits(),
            config.unison_alignment.to_bits(),
            config.unison_alignment_mode,
            config.unison_pan_curve.to_bits(),
            config.unison_pan_center_x.to_bits(),
            config.unison_stereo_x.to_bits(),
            config.unison_stereo_alternate.to_bits(),
        )
}

pub(crate) fn normalized_unison_rate(normalized: f32) -> f32 {
    0.02 * 5_000.0_f32.powf(normalized.clamp(0.0, 1.0))
}
