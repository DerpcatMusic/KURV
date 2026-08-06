use std::sync::{Arc, atomic::Ordering};

use truce::params::Params;
use truce_core::editor::{Editor, PluginContext, PluginContextReadF32, RawWindowHandle};
use truce_egui::EguiEditor;

use crate::editor_controls::{param_field_sized, param_knob, pitch_wheel};
use crate::pan_curve::PanShapeCurveData;
use crate::{KurvParams, P, editor_theme};

const EDITOR_SIZE: (u32, u32) = (1120, 720);
const EDITOR_MIN_SIZE: (u32, u32) = (560, 360);
const EDITOR_MAX_SIZE: (u32, u32) = (2240, 1440);

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
        crate::diagnostics::lifecycle("editor-close-enter");
        self.inner.close();
        crate::diagnostics::lifecycle("editor-close-return");
    }

    fn idle(&mut self) {
        self.inner.idle();
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        crate::diagnostics::trace("editor", "set-size-enter", width as f32, height as f32);
        if !self.inner.set_size(width, height) {
            crate::diagnostics::trace("editor", "set-size-rejected", width as f32, height as f32);
            return false;
        }
        if let Ok(mut state) = self.params.editor_state.lock() {
            state.width = width;
            state.height = height;
        }
        crate::diagnostics::trace("editor", "set-size-return", width as f32, height as f32);
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
    let mut inner = EguiEditor::new(params.clone(), size, crate::editor_shell::draw)
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

pub(crate) fn performance_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    ui.set_width(width);
    ui.set_height(height);
    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
    ui.label(
        egui::RichText::new("PERFORMANCE")
            .font(editor_theme::font::title())
            .color(editor_theme::semantic().text_muted),
    );
    let compact = width < 300.0 || height < 220.0;
    if compact {
        let params = [
            (P::Transpose, "TRANSPOSE"),
            (P::OctaveShift, "OCTAVE"),
            (P::MpeBendRange, "BEND"),
            (P::GlideTime, "GLIDE"),
            (P::VelocityAmount, "VELOCITY"),
            (P::PressureAmount, "PRESSURE"),
            (P::TimbreAmount, "TIMBRE"),
        ];
        let row_height = ((height - 48.0) / 5.0).max(18.0);
        ui.horizontal(|ui| {
            voice_mode_selector(ui, state, (width * 0.5 - 2.0).max(48.0));
            param_field_sized(
                ui,
                state,
                P::PitchBend,
                "PITCH",
                (width * 0.5 - 2.0).max(48.0),
                row_height,
            );
        });
        for row in params.chunks(2) {
            ui.horizontal(|ui| {
                for &(param, label) in row {
                    param_field_sized(
                        ui,
                        state,
                        param,
                        label,
                        (width * 0.5 - 2.0).max(32.0),
                        row_height,
                    );
                }
            });
        }
        return;
    }

    ui.horizontal(|ui| {
        pitch_wheel(ui, state);
        voice_mode_selector(ui, state, 72.0);
        param_field_sized(ui, state, P::Transpose, "TRANSPOSE", 62.0, 48.0);
        param_field_sized(ui, state, P::OctaveShift, "OCTAVE", 56.0, 48.0);
        param_field_sized(ui, state, P::MpeBendRange, "BEND", 62.0, 48.0);
    });
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("EXPRESSION RESPONSE")
            .font(editor_theme::font::title())
            .color(editor_theme::semantic().text_muted),
    );
    ui.horizontal(|ui| {
        param_knob(ui, state, P::VelocityAmount, "Velocity");
        param_knob(ui, state, P::PressureAmount, "Pressure");
        param_knob(ui, state, P::TimbreAmount, "Timbre");
        param_knob(ui, state, P::GlideTime, "Glide").on_hover_text("Used by LEGATO mode");
    });
}

fn voice_mode_selector(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, width: f32) {
    const MODES: [u8; 11] = [0, 1, 2, 4, 6, 8, 10, 12, 16, 24, 32];
    let current = state.params().voice_mode.value_u8();
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new("VOICE MODE")
                .font(egui::FontId::monospace(7.5))
                .color(editor_theme::semantic().text_muted),
        );
        egui::ComboBox::from_id_salt("performance-voice-mode")
            .selected_text(state.format_param(P::VoiceMode))
            .width(width)
            .show_ui(ui, |ui| {
                for mode in MODES {
                    let label = match mode {
                        0 => "MONO".to_owned(),
                        1 => "LEGATO".to_owned(),
                        voices => voices.to_string(),
                    };
                    if ui.selectable_label(current == mode, label).clicked() {
                        state.begin_edit(P::VoiceMode);
                        state.set_param(P::VoiceMode, f64::from(mode) / 32.0);
                        state.end_edit(P::VoiceMode);
                    }
                }
            });
    });
}

/// The header's double-click is the one unambiguous factory-reset gesture.
/// Defaults come from Truce's generated parameter metadata, so this remains
/// correct if a range's normalized representation changes later.
pub(crate) fn reset_to_defaults(state: &PluginContext<KurvParams>) {
    let parameters = [
        P::OutputDb,
        P::Osc1Enabled,
        P::Osc1Transpose,
        P::Osc1Cents,
        P::Osc1Level,
        P::Osc1Pan,
        P::Osc1WarpMode,
        P::Osc1WarpAmount,
        P::Shape,
        P::PulseWidth,
        P::Osc2Enabled,
        P::Osc2Shape,
        P::Osc2PulseWidth,
        P::Osc2Transpose,
        P::Osc2Cents,
        P::Osc2Level,
        P::Osc2Pan,
        P::Osc2WarpMode,
        P::Osc2WarpAmount,
        P::Osc2JitterMode,
        P::Osc2UnisonVoices,
        P::Osc2UnisonDetune,
        P::Osc2UnisonDetuneAmount,
        P::Osc2UnisonStereo,
        P::Osc2PhaseRandom,
        P::Osc2UnisonCurve,
        P::Osc2UnisonJitter,
        P::Osc2UnisonJitterRate,
        P::Osc2StereoAlternate,
        P::Osc2StereoX,
        P::Osc2UnisonWeight,
        P::Osc2PanShapeCenter,
        P::Osc2PanShapeLeft,
        P::Osc2PanShapeRight,
        P::Osc2PanShapeLeftCurve,
        P::Osc2PanShapeRightCurve,
        P::Osc2PanShapeLeftCurveTime,
        P::Osc2PanShapeRightCurveTime,
        P::Osc2PanShapeCenterX,
        P::Osc3Enabled,
        P::Osc3Shape,
        P::Osc3PulseWidth,
        P::Osc3Transpose,
        P::Osc3Cents,
        P::Osc3Level,
        P::Osc3Pan,
        P::Osc3WarpMode,
        P::Osc3WarpAmount,
        P::Osc3JitterMode,
        P::Osc3UnisonVoices,
        P::Osc3UnisonDetune,
        P::Osc3UnisonDetuneAmount,
        P::Osc3UnisonStereo,
        P::Osc3PhaseRandom,
        P::Osc3UnisonCurve,
        P::Osc3UnisonJitter,
        P::Osc3UnisonJitterRate,
        P::Osc3StereoAlternate,
        P::Osc3StereoX,
        P::Osc3UnisonWeight,
        P::Osc3PanShapeCenter,
        P::Osc3PanShapeLeft,
        P::Osc3PanShapeRight,
        P::Osc3PanShapeLeftCurve,
        P::Osc3PanShapeRightCurve,
        P::Osc3PanShapeLeftCurveTime,
        P::Osc3PanShapeRightCurveTime,
        P::Osc3PanShapeCenterX,
        P::Attack,
        P::Decay,
        P::Sustain,
        P::Release,
        P::Drone,
        P::DroneFrequency,
        P::PitchBend,
        P::SustainPedal,
        P::UnisonVoices,
        P::UnisonDetune,
        P::UnisonStereo,
        P::PhaseRandom,
        P::UnisonCurve,
        P::VelocityAmount,
        P::PressureAmount,
        P::TimbreAmount,
        P::MpeBendRange,
        P::UnisonSwarm,
        P::UnisonSwarmRate,
        P::UnisonSwarmMode,
        P::Transpose,
        P::OctaveShift,
        P::VoiceMode,
        P::GlideTime,
        P::StereoPattern,
        P::AttackCurve,
        P::DecayCurve,
        P::ReleaseCurve,
        P::AttackCurveTime,
        P::DecayCurveTime,
        P::ReleaseCurveTime,
        P::StereoAlternate,
        P::StereoX,
        P::UnisonWeight,
        P::Oversampling,
        P::UnisonDetuneAmount,
        P::PanShapeCenter,
        P::PanShapeEdge,
        P::PanShapeCurve,
        P::PanShapeCurveTime,
        P::Antialiasing,
        P::GeneratorEngine,
        P::PanShapeLeft,
        P::PanShapeRight,
        P::PanShapeLeftCurve,
        P::PanShapeRightCurve,
        P::PanShapeLeftCurveTime,
        P::PanShapeRightCurveTime,
        P::PanShapeCenterX,
        P::Lfo1Rate,
        P::Lfo1Mode,
        P::Lfo1Phase,
        P::Lfo1Sync,
        P::Lfo1Bipolar,
        P::Lfo2Rate,
        P::Lfo2Mode,
        P::Lfo2Phase,
        P::Lfo2Sync,
        P::Lfo2Bipolar,
        P::Lfo3Rate,
        P::Lfo3Mode,
        P::Lfo3Phase,
        P::Lfo3Sync,
        P::Lfo3Bipolar,
        P::Lfo4Rate,
        P::Lfo4Mode,
        P::Lfo4Phase,
        P::Lfo4Sync,
        P::Lfo4Bipolar,
        P::Mod1Source,
        P::Mod1Target,
        P::Mod1Amount,
        P::Mod2Source,
        P::Mod2Target,
        P::Mod2Amount,
        P::Mod3Source,
        P::Mod3Target,
        P::Mod3Amount,
        P::Mod4Source,
        P::Mod4Target,
        P::Mod4Amount,
        P::Mod5Source,
        P::Mod5Target,
        P::Mod5Amount,
        P::Mod6Source,
        P::Mod6Target,
        P::Mod6Amount,
        P::Mod7Source,
        P::Mod7Target,
        P::Mod7Amount,
        P::Mod8Source,
        P::Mod8Target,
        P::Mod8Amount,
    ];
    let infos = state.params().param_infos();
    for param in parameters {
        let Some(info) = infos.iter().find(|info| info.id == u32::from(param)) else {
            continue;
        };
        let normalized = info.range.normalize(info.default_plain).clamp(0.0, 1.0);
        state.begin_edit(param);
        state.set_param(param, normalized);
        state.end_edit(param);
    }
    state
        .params()
        .pan_shape_curve_state
        .replace(PanShapeCurveData::default());
    state
        .params()
        .osc2_pan_shape_curve_state
        .replace(PanShapeCurveData::default());
    state
        .params()
        .osc3_pan_shape_curve_state
        .replace(PanShapeCurveData::default());
    for curve in [
        &state.params().osc1_wave_curve_state,
        &state.params().osc2_wave_curve_state,
        &state.params().osc3_wave_curve_state,
        &state.params().lfo1_curve_state,
        &state.params().lfo2_curve_state,
        &state.params().lfo3_curve_state,
        &state.params().lfo4_curve_state,
    ] {
        curve.replace(crate::wave_curve::WaveCurveData::default());
    }
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
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(120.0), height.max(24.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let track_left = rect.left() + 30.0;
    let track_right = rect.right() - 58.0;
    let value = state.get_param(P::OutputDb);
    if response.double_clicked() {
        let default = state
            .params()
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(P::OutputDb))
            .map_or(value, |info| {
                info.range.normalize(info.default_plain) as f32
            });
        state.begin_edit(P::OutputDb);
        state.set_param(P::OutputDb, f64::from(default));
        state.end_edit(P::OutputDb);
    } else {
        if response.drag_started() {
            state.begin_edit(P::OutputDb);
        }
        if (response.drag_started() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let normalized =
                ((pointer.x - track_left) / (track_right - track_left)).clamp(0.0, 1.0);
            state.set_param(P::OutputDb, f64::from(normalized));
        }
        if response.drag_stopped() {
            state.end_edit(P::OutputDb);
        }
    }

    let left = state.get_meter(&state.params().meter_left).max(0.0);
    let right = state.get_meter(&state.params().meter_right).max(0.0);
    if left > 1.0e-4 || right > 1.0e-4 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
    let painter = ui.painter_at(rect);
    painter.text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        "OUT",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    let bar_height = (rect.height() * 0.16).clamp(4.0, 6.0);
    let bar_gap = 2.0;
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
        painter.rect_filled(bar, radius, editor_theme::semantic().control_hover);
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
            egui::pos2(marker_x, rect.top() + 6.0),
            egui::pos2(marker_x, rect.bottom() - 8.0),
        ],
        egui::Stroke::new(1.0_f32, editor_theme::semantic().text),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(marker_x, rect.bottom() - 8.0),
            egui::pos2(marker_x - 3.0, rect.bottom() - 4.0),
            egui::pos2(marker_x + 3.0, rect.bottom() - 4.0),
        ],
        editor_theme::semantic().text,
        egui::Stroke::NONE,
    ));
    painter.text(
        rect.right_center() - egui::vec2(5.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        state.format_param(P::OutputDb),
        egui::FontId::monospace(8.5),
        ui.visuals().text_color(),
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
