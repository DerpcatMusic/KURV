use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use truce::params::Params;
use truce_core::editor::{Editor, PluginContext, PluginContextReadF32, RawWindowHandle};
use truce_egui::EguiEditor;

use crate::{KurvParams, P, editor_theme};

mod factory_reset;

pub(crate) use factory_reset::reset_to_defaults;

const EDITOR_SIZE: (u32, u32) = (1120, 720);
const EDITOR_MIN_SIZE: (u32, u32) = (960, 480);
const EDITOR_MAX_SIZE: (u32, u32) = (2240, 1440);
const EDITOR_PHASE_IDLE: u8 = 0;
const EDITOR_PHASE_DRAW: u8 = 1;
static EDITOR_PHASE: AtomicU8 = AtomicU8::new(EDITOR_PHASE_IDLE);

struct EditorDrawGuard;

impl Drop for EditorDrawGuard {
    fn drop(&mut self) {
        EDITOR_PHASE.store(EDITOR_PHASE_IDLE, Ordering::Release);
    }
}

fn draw_with_phase(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    EDITOR_PHASE.store(EDITOR_PHASE_DRAW, Ordering::Release);
    let _guard = EditorDrawGuard;
    crate::editor_shell::draw(ui, state);
}

struct PersistedEditor {
    inner: EguiEditor<KurvParams>,
    params: Arc<KurvParams>,
}

impl Editor for PersistedEditor {
    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn open(&mut self, parent: RawWindowHandle, context: PluginContext) {
        crate::diagnostics::lifecycle("editor-open-enter");
        self.inner.open(parent, context);
        crate::diagnostics::lifecycle("editor-open-return");
    }

    fn close(&mut self) {
        crate::diagnostics::trace(
            "editor",
            "close-phase",
            f32::from(EDITOR_PHASE.load(Ordering::Acquire)),
            0.0,
        );
        crate::diagnostics::lifecycle("editor-close-enter");
        self.inner.close();
        crate::diagnostics::lifecycle("editor-close-return");
    }

    fn idle(&mut self) {
        self.inner.idle();
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if self.inner.size() == (width, height) {
            return true;
        }
        if !self.inner.set_size(width, height) {
            crate::diagnostics::trace("editor", "set-size-rejected", width as f32, height as f32);
            return false;
        }
        if let Ok(mut state) = self.params.editor_state.lock() {
            state.width = width;
            state.height = height;
        }
        true
    }

    fn can_resize(&self) -> bool {
        self.inner.can_resize()
    }

    fn can_maximize(&self) -> bool {
        self.inner.can_maximize()
    }

    fn min_size(&self) -> (u32, u32) {
        Editor::min_size(&self.inner)
    }

    fn max_size(&self) -> (u32, u32) {
        Editor::max_size(&self.inner)
    }

    fn size_increment(&self) -> Option<(u32, u32)> {
        self.inner.size_increment()
    }

    fn aspect_ratio(&self) -> Option<(u32, u32)> {
        Editor::aspect_ratio(&self.inner)
    }

    fn prefers_pow2(&self) -> bool {
        Editor::prefers_pow2(&self.inner)
    }

    fn set_scale_factor(&mut self, factor: f64) {
        if factor.is_finite() && factor > 0.0 {
            self.params
                .editor_host_scale_bits
                .store(factor.to_bits(), Ordering::Relaxed);
        }
        self.inner.set_scale_factor(factor);
    }

    fn set_uses_system_scale(&mut self, yes: bool) {
        self.inner.set_uses_system_scale(yes);
    }

    fn state_changed(&mut self) {
        crate::diagnostics::lifecycle("editor-state-changed-enter");
        self.inner.state_changed();
        crate::diagnostics::lifecycle("editor-state-changed-return");
    }

    fn screenshot(&mut self, params: Arc<dyn Params>) -> Option<(Vec<u8>, u32, u32)> {
        self.inner.screenshot(params)
    }
}

impl Drop for PersistedEditor {
    fn drop(&mut self) {
        crate::diagnostics::lifecycle("editor-drop-enter");
    }
}

pub fn create(params: Arc<KurvParams>) -> Box<dyn Editor> {
    crate::diagnostics::lifecycle("editor-create-enter");
    let size = params.editor_state.lock().map_or(EDITOR_SIZE, |state| {
        (
            state.width.clamp(EDITOR_MIN_SIZE.0, EDITOR_MAX_SIZE.0),
            state.height.clamp(EDITOR_MIN_SIZE.1, EDITOR_MAX_SIZE.1),
        )
    });
    let mut inner = EguiEditor::new(params.clone(), size, draw_with_phase)
        .with_visuals(truce_egui::theme::dark())
        .resizable(true)
        .min_size(EDITOR_MIN_SIZE)
        .max_size(EDITOR_MAX_SIZE)
        .prefers_pow2(false);
    let host_scale_bits = params.editor_host_scale_bits.load(Ordering::Relaxed);
    if host_scale_bits != 0 {
        inner.set_scale_factor(f64::from_bits(host_scale_bits));
    }
    let editor = Box::new(PersistedEditor { inner, params });
    crate::diagnostics::lifecycle("editor-create-return");
    editor
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Truce normalized parameters are bounded to 0..1 before entering egui's f32 coordinates"
)]
pub(crate) fn output_meter(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    let metrics = editor_theme::metrics(ui);
    let palette = editor_theme::semantic();
    let label = ui.painter().layout_no_wrap(
        "OUT".to_owned(),
        editor_theme::font::label(),
        palette.text_muted,
    );
    let value_text = state.format_param(P::OutputDb);
    let measured_value =
        ui.painter()
            .layout_no_wrap(value_text, editor_theme::font::value(), palette.text);
    let value_width = measured_value
        .size()
        .x
        .max(metrics.output_trim_min_track * 0.72);
    let inset = metrics.spacing_scale.xs;
    let gap = metrics.spacing_scale.sm;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            width.max(editor_theme::shape::STROKE),
            height.max(editor_theme::shape::STROKE),
        ),
        egui::Sense::hover(),
    );
    let content_left = rect.left() + inset;
    let content_right = rect.right() - inset;
    let show_label = content_right - content_left
        >= label.size().x + value_width + gap * 2.0 + metrics.output_trim_min_track;
    let track_left = if show_label {
        content_left + label.size().x + gap
    } else {
        content_left
    };
    let track_right =
        (content_right - value_width - gap).max(track_left + editor_theme::shape::STROKE);
    let response = ui
        .interact(
            egui::Rect::from_x_y_ranges(track_left..=track_right, rect.y_range()),
            ui.id().with("output-trim"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Output trim: drag vertically. Hold Shift for fine control; double-click to reset.",
        );
    let modulation_gesture =
        crate::editor_modulation::owns_gesture(ui, state, P::OutputDb, &response);
    let value = if modulation_gesture {
        state.get_param(P::OutputDb)
    } else {
        crate::editor_controls::update_parameter_drag(
            ui,
            state,
            P::OutputDb,
            "Output trim",
            &response,
        )
    };

    let left = state.get_meter(&state.params().meter_left).max(0.0);
    let right = state.get_meter(&state.params().meter_right).max(0.0);
    if left > 1.0e-4 || right > 1.0e-4 {
        editor_theme::request_display_repaint(ui);
    }
    let painter = ui.painter_at(rect);
    let value_label = painter.layout_no_wrap(
        state.format_param(P::OutputDb),
        editor_theme::font::value(),
        palette.text,
    );
    if show_label {
        painter.galley(
            egui::pos2(content_left, rect.center().y - label.size().y * 0.5),
            label,
            palette.text_muted,
        );
    }
    let bar_height = metrics.spacing_scale.xs;
    let bar_gap = editor_theme::compact_gap(ui);
    let bar_center = rect.center().y;
    let first_y = bar_center - bar_gap * 0.5 - bar_height;
    for (y, level) in [first_y, first_y + bar_height + bar_gap]
        .into_iter()
        .zip([left, right])
    {
        let bar = egui::Rect::from_min_max(
            egui::pos2(track_left, y),
            egui::pos2(track_right, y + bar_height),
        );
        let radius = bar_height * 0.5;
        painter.rect_filled(
            bar,
            radius,
            if response.hovered() {
                palette.control_hover
            } else {
                palette.control
            },
        );
        let db = 20.0 * level.max(1.0e-6).log10();
        let normalized = ((db + 48.0) / 48.0).clamp(0.0, 1.0);
        let fill = egui::Rect::from_min_max(
            bar.min,
            egui::pos2(
                egui::lerp(bar.left()..=bar.right(), normalized),
                bar.bottom(),
            ),
        );
        painter.rect_filled(
            fill,
            radius,
            if level >= 0.999 {
                editor_theme::palette().warning
            } else {
                editor_theme::palette().accent
            },
        );
    }
    let marker_x = egui::lerp(track_left..=track_right, value);
    painter.line_segment(
        [
            egui::pos2(marker_x, rect.top() + metrics.spacing_scale.xs),
            egui::pos2(marker_x, rect.bottom() - metrics.spacing_scale.sm),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, palette.text),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(marker_x, rect.bottom() - metrics.spacing_scale.sm),
            egui::pos2(
                marker_x - metrics.unit,
                rect.bottom() - metrics.spacing_scale.xs,
            ),
            egui::pos2(
                marker_x + metrics.unit,
                rect.bottom() - metrics.spacing_scale.xs,
            ),
        ],
        palette.text,
        egui::Stroke::NONE,
    ));
    painter.galley(
        egui::pos2(
            rect.right() - inset - value_label.size().x,
            rect.center().y - value_label.size().y * 0.5,
        ),
        value_label,
        if response.dragged() {
            palette.primary
        } else {
            palette.text
        },
    );
    crate::editor_modulation::destination(
        ui,
        state,
        P::OutputDb,
        &response,
        value,
        egui::Rect::from_x_y_ranges(track_left..=track_right, rect.y_range()),
        crate::editor_modulation::TrackAxis::Horizontal,
    );
    response
}

#[cfg(test)]
mod tests {
    use crate::editor_controls::{accumulate_drag, magnetic_shape_snap};

    #[test]
    fn knob_drag_accumulates_every_pointer_frame() {
        let value = [-3.0_f32; 10].into_iter().fold(0.25, accumulate_drag);
        assert!((value - 0.45).abs() < 1.0e-6);
    }

    #[test]
    fn shape_knob_snaps_only_near_canonical_shapes() {
        assert!((magnetic_shape_snap(0.34) - (1.0 / 3.0)).abs() < 1.0e-6);
        assert!((magnetic_shape_snap(0.42) - 0.42).abs() < 1.0e-6);
    }
}
