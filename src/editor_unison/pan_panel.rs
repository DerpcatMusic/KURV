//! Pan view switching, stereo-square editing, and host gestures.

mod painting;

use truce_core::editor::PluginContext;

use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::pan_curve::{PanShapeCurveData, PanShapeCurveState};
use crate::{KurvParams, editor_theme};

use super::pan_shape::{constrain_drag, custom_pan_shape_curve_view};
use super::{host_axes_context_menu, update_host_axis};
use painting::paint_stereo_square;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CompactPanView {
    #[default]
    PanXy,
    PanShape,
}

pub(crate) fn custom_pan_panel_view(
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
    let before = (
        config.unison_pan_center_x.to_bits(),
        config.unison_stereo_x.to_bits(),
        config.unison_stereo_alternate.to_bits(),
    );
    let palette = editor_theme::semantic();
    let frame = outer
        .rect
        .shrink(editor_theme::space::XXS.min(outer.rect.width().min(outer.rect.height()) * 0.025));
    let header_height = (editor_theme::font::LABEL_SIZE + editor_theme::space::XXS * 2.0)
        .min(frame.height() * 0.24);
    let header = egui::Rect::from_min_max(
        frame.min,
        egui::pos2(
            frame.right(),
            (frame.top() + header_height).min(frame.bottom()),
        ),
    );
    let content = egui::Rect::from_min_max(egui::pos2(frame.left(), header.bottom()), frame.max);
    let view_id = outer.id.with("pan-view");
    let current = ui
        .data(|data| data.get_temp::<CompactPanView>(view_id))
        .unwrap_or_default();
    let mut selected = current;
    let mut pan_shape_tab_response = None;
    let compact_tabs = header.width() * 0.5 < editor_theme::font::LABEL_SIZE * 6.5;
    for (view, label, compact_label, accent) in [
        (CompactPanView::PanXy, "PAN X/Y", "X/Y", palette.pan_shape),
        (
            CompactPanView::PanShape,
            "PAN SHAPE",
            "SHAPE",
            palette.pan_shape,
        ),
    ] {
        let tab = egui::Rect::from_min_max(
            egui::pos2(
                header.left()
                    + header.width()
                        * if matches!(view, CompactPanView::PanXy) {
                            0.0
                        } else {
                            0.5
                        },
                header.top(),
            ),
            egui::pos2(
                header.left()
                    + header.width()
                        * if matches!(view, CompactPanView::PanXy) {
                            0.5
                        } else {
                            1.0
                        },
                header.bottom(),
            ),
        );
        let response = ui
            .interact(tab, view_id.with(label), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if matches!(view, CompactPanView::PanShape) {
            pan_shape_tab_response = Some(response.clone());
        }
        if response.clicked() {
            selected = view;
        }
        let active = response.is_pointer_button_down_on();
        painter.text(
            tab.center(),
            egui::Align2::CENTER_CENTER,
            if compact_tabs { compact_label } else { label },
            editor_theme::font::caption(),
            if active {
                palette.text
            } else if selected == view {
                accent
            } else if response.hovered() {
                accent.gamma_multiply(0.82)
            } else {
                palette.text_muted.gamma_multiply(0.72)
            },
        );
        if selected == view {
            painter.line_segment(
                [tab.left_bottom(), tab.right_bottom()],
                egui::Stroke::new(editor_theme::shape::STROKE, accent),
            );
        }
    }
    if selected != current {
        ui.data_mut(|data| data.insert_temp(view_id, selected));
    }

    let mut curve_changed = false;
    match selected {
        CompactPanView::PanXy => {
            let response = custom_stereo_square_view(
                ui,
                &painter,
                content,
                outer.id.with("stereo-square"),
                &mut config.unison_stereo_x,
                &mut config.unison_stereo_alternate,
            );
            let x_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoPosition,
            );
            let y_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoAlternate,
            );
            let x = OscillatorControl::UnisonStereoPosition.normalized_value(*config);
            let y = OscillatorControl::UnisonStereoAlternate.normalized_value(*config);
            host_axes_context_menu(
                &response,
                state,
                &[("X · PATTERN", x_target, x), ("Y · BLEND", y_target, y)],
            );
            update_host_axis(
                ui,
                state,
                x_target,
                &response,
                x,
                before.1 != config.unison_stereo_x.to_bits(),
            );
            update_host_axis(
                ui,
                state,
                y_target,
                &response,
                y,
                before.2 != config.unison_stereo_alternate.to_bits(),
            );
        }
        CompactPanView::PanShape => {
            if !pan_shape_curve.is_initialized() {
                pan_shape_curve.replace(PanShapeCurveData::from_legacy(
                    0.0,
                    1.0,
                    1.0,
                    config.unison_pan_curve,
                    config.unison_pan_curve,
                    0.5,
                    0.5,
                ));
            }
            let split_gap = editor_theme::compact_gap(ui);
            let available_width = (content.width() - split_gap).max(1.0);
            let curve_width = available_width * 0.5;
            let curve_rect = egui::Rect::from_min_size(
                content.min,
                egui::vec2(curve_width.max(1.0), content.height()),
            );
            let xy_rect = egui::Rect::from_min_max(
                egui::pos2(curve_rect.right() + split_gap, content.top()),
                content.max,
            );
            painter.line_segment(
                [
                    egui::pos2(curve_rect.right() + split_gap * 0.5, content.top()),
                    egui::pos2(curve_rect.right() + split_gap * 0.5, content.bottom()),
                ],
                egui::Stroke::new(
                    editor_theme::shape::STROKE,
                    palette.grid.gamma_multiply(0.34),
                ),
            );
            let (changed, curve_response) = custom_pan_shape_curve_view(
                ui,
                &painter,
                curve_rect,
                outer.id.with("pan-shape"),
                pan_shape_curve,
                &mut config.unison_pan_center_x,
            );
            curve_changed |= changed;
            let target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonPanCenter,
            );
            let normalized = OscillatorControl::UnisonPanCenter.normalized_value(*config);
            if let Some(tab) = pan_shape_tab_response.as_ref() {
                tab.context_menu(|ui| {
                    crate::editor_modulation::host_automation_menu(ui, state, target, normalized);
                });
            }
            update_host_axis(
                ui,
                state,
                target,
                &curve_response,
                normalized,
                before.0 != config.unison_pan_center_x.to_bits(),
            );
            let xy_response = custom_stereo_square_view(
                ui,
                &painter,
                xy_rect,
                outer.id.with("pan-shape-stereo-square"),
                &mut config.unison_stereo_x,
                &mut config.unison_stereo_alternate,
            );
            let x_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoPosition,
            );
            let y_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoAlternate,
            );
            let x = OscillatorControl::UnisonStereoPosition.normalized_value(*config);
            let y = OscillatorControl::UnisonStereoAlternate.normalized_value(*config);
            host_axes_context_menu(
                &xy_response,
                state,
                &[("X · PATTERN", x_target, x), ("Y · BLEND", y_target, y)],
            );
            update_host_axis(
                ui,
                state,
                x_target,
                &xy_response,
                x,
                before.1 != config.unison_stereo_x.to_bits(),
            );
            update_host_axis(
                ui,
                state,
                y_target,
                &xy_response,
                y,
                before.2 != config.unison_stereo_alternate.to_bits(),
            );
        }
    }
    before
        != (
            config.unison_pan_center_x.to_bits(),
            config.unison_stereo_x.to_bits(),
            config.unison_stereo_alternate.to_bits(),
        )
        || curve_changed
}

#[derive(Clone, Copy)]
struct StereoSquare {
    rect: egui::Rect,
}

impl StereoSquare {
    fn new(rect: egui::Rect) -> Self {
        Self { rect }
    }

    fn point(self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(
            egui::lerp(self.rect.left()..=self.rect.right(), x.clamp(0.0, 1.0)),
            egui::lerp(self.rect.bottom()..=self.rect.top(), y.clamp(0.0, 1.0)),
        )
    }

    fn axes_at(self, point: egui::Pos2) -> (f32, f32) {
        let x =
            ((point.x - self.rect.left()) / self.rect.width().max(f32::EPSILON)).clamp(0.0, 1.0);
        let y =
            ((self.rect.bottom() - point.y) / self.rect.height().max(f32::EPSILON)).clamp(0.0, 1.0);
        (x, y)
    }

    fn snap(self, axes: (f32, f32), enabled: bool, radius: f32) -> (f32, f32) {
        if !enabled {
            return axes;
        }
        let candidates = [
            (0.0, 1.0),
            (1.0, 1.0),
            (0.0, 0.0),
            (1.0, 0.0),
            (0.5, 1.0),
            (0.5, 0.0),
            (0.0, 0.5),
            (1.0, 0.5),
            (0.5, 0.5),
        ];
        let point = self.point(axes.0, axes.1);
        candidates
            .into_iter()
            .filter(|candidate| self.point(candidate.0, candidate.1).distance(point) <= radius)
            .min_by(|left, right| {
                self.point(left.0, left.1)
                    .distance_sq(point)
                    .total_cmp(&self.point(right.0, right.1).distance_sq(point))
            })
            .unwrap_or(axes)
    }
}

fn stereo_square_plot(rect: egui::Rect) -> egui::Rect {
    rect.shrink(
        (editor_theme::font::CAPTION_SIZE * 0.54).min(rect.width().min(rect.height()) * 0.12),
    )
}

fn custom_stereo_square_view(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    id: egui::Id,
    x: &mut f32,
    y: &mut f32,
) -> egui::Response {
    let plot = stereo_square_plot(rect);
    let response = ui.interact(plot, id, egui::Sense::CLICK | egui::Sense::DRAG);
    response
        .clone()
        .on_hover_text("X selects stereo pattern; Y blends alternate/pair with random/shape");
    let active = response.dragged() || response.is_pointer_button_down_on();
    let point = StereoSquare::new(plot).point(*x, *y);
    let point_hovered = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|pointer| pointer.distance(point) <= editor_theme::title_height(ui) * 0.55);
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = if active {
                egui::CursorIcon::Grabbing
            } else if point_hovered {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
    if (response.drag_started_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary))
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let anchor = pointer - response.drag_delta();
        let (constrained, snapping) = ui.input(|input| {
            (
                constrain_drag(anchor, pointer, input.modifiers.alt),
                !input.modifiers.shift,
            )
        });
        (*x, *y) = StereoSquare::new(plot).snap(
            StereoSquare::new(plot).axes_at(constrained),
            snapping,
            editor_theme::title_height(ui) * 0.35,
        );
    }
    *x = (*x).clamp(0.0, 1.0);
    *y = (*y).clamp(0.0, 1.0);
    paint_stereo_square(
        painter,
        plot,
        StereoSquare::new(plot).point(*x, *y),
        response.hovered(),
        active,
    );
    response
}
