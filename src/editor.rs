use std::sync::{Arc, atomic::Ordering};

use truce::params::Params;
use truce_core::editor::{Editor, PluginContext, PluginContextReadF32, RawWindowHandle};
use truce_egui::EguiEditor;

use crate::editor_controls::{
    fit_font_to_width, mod_wheel_sized, param_field_sized, pitch_wheel_sized,
};
use crate::pan_curve::PanShapeCurveData;
use crate::{KurvParams, P, editor_theme};

const EDITOR_SIZE: (u32, u32) = (1120, 720);
const EDITOR_MIN_SIZE: (u32, u32) = (960, 480);
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
    ui.set_min_size(egui::vec2(width, height));
    let gap = editor_theme::compact_gap(ui);
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    performance_heading(ui, "PERFORMANCE");

    let body_size = egui::vec2(
        ui.available_width().max(1.0),
        ui.available_height().max(1.0),
    );
    ui.allocate_ui_with_layout(
        body_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let painter = ui.painter();
            let label_width = |label: &str| {
                painter
                    .layout_no_wrap(
                        label.to_owned(),
                        editor_theme::font::caption(),
                        editor_theme::semantic().text_muted,
                    )
                    .size()
                    .x
            };
            let rail_gap = editor_theme::space::XS;
            let section_gap = gap.max(editor_theme::shape::STROKE);
            let rail_min_width = editor_theme::space::LG + editor_theme::space::MD;
            let desired_rail_width = label_width("PITCH")
                .max(label_width("MOD") + editor_theme::space::SM)
                .max(rail_min_width);
            let column_share = ((body_size.x - section_gap - rail_gap).max(1.0) / 5.0)
                .max(editor_theme::shape::STROKE);
            let rail_width = desired_rail_width.min(column_share);
            let strip_width = rail_width * 2.0 + rail_gap;
            ui.spacing_mut().item_spacing.x = section_gap;
            ui.allocate_ui_with_layout(
                egui::vec2(strip_width, body_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| wheel_strip(ui, state, body_size.y, rail_width),
            );
            let fields = egui::vec2(
                (body_size.x - strip_width - section_gap).max(1.0),
                body_size.y,
            );
            ui.allocate_ui_with_layout(fields, egui::Layout::top_down(egui::Align::Min), |ui| {
                performance_field_grid(ui, state, fields.y)
            });
        },
    );
}

fn wheel_strip(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32, width: f32) {
    let gap = editor_theme::space::XS;
    ui.spacing_mut().item_spacing.x = gap;
    pitch_wheel_sized(ui, state, width, height);
    mod_wheel_sized(ui, state, width, height);
}

fn performance_heading(ui: &mut egui::Ui, label: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), editor_theme::title_height(ui)),
        egui::Sense::hover(),
    );
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
}

fn performance_field_grid(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32) {
    let gap = editor_theme::compact_gap(ui);
    let width = ui.available_width();
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    let row_height = ((height - gap * 2.0).max(0.0) / 3.0).max(editor_theme::shape::STROKE);
    let field_width = ((width - gap * 2.0).max(0.0) / 3.0).max(editor_theme::shape::STROKE);
    let row_size = egui::vec2(width, row_height);
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            voice_mode_selector(ui, state, field_width, row_height);
            performance_param_field(ui, state, P::Transpose, "SEMI", field_width, row_height);
            performance_param_field(ui, state, P::OctaveShift, "OCT", field_width, row_height);
        },
    );
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            performance_param_field(ui, state, P::GlideTime, "GLIDE", field_width, row_height)
                .on_hover_text("Used by LEGATO mode");
            performance_param_field(
                ui,
                state,
                P::PitchBendRange,
                "PB RANGE",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::MpeBendRange,
                "MPE RANGE",
                field_width,
                row_height,
            );
        },
    );
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            performance_param_field(
                ui,
                state,
                P::VelocityAmount,
                "VEL AMT",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::PressureAmount,
                "PRESS AMT",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::TimbreAmount,
                "TIMBRE AMT",
                field_width,
                row_height,
            );
        },
    );
}

fn performance_track(rect: egui::Rect) -> egui::Rect {
    let track_width = (rect.width() - editor_theme::space::MD * 2.0)
        .max(editor_theme::space::LG)
        .min(rect.width());
    egui::Rect::from_center_size(
        egui::pos2(
            rect.center().x,
            rect.bottom() - editor_theme::space::XXS - editor_theme::shape::STROKE,
        ),
        egui::vec2(track_width, editor_theme::shape::STROKE),
    )
}

fn performance_param_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    width: f32,
    height: f32,
) -> egui::Response {
    let size = egui::vec2(width.max(1.0), height.max(1.0));
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let indicator_height = editor_theme::shape::STROKE * 2.0;
    let widget_clip = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(
            rect.right(),
            (rect.bottom() - indicator_height).max(rect.top()),
        ),
    )
    .intersect(ui.clip_rect());
    let mut field_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("performance-field", u32::from(id)))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    field_ui.set_clip_rect(widget_clip);
    let response = param_field_sized(&mut field_ui, state, id, label, width, height);

    let palette = editor_theme::semantic();
    let active = response.is_pointer_button_down_on() || response.dragged();
    let show_surface = response.hovered() || active || response.has_focus();
    let visuals = editor_theme::control_visuals(
        response.enabled(),
        response.hovered(),
        active,
        response.has_focus(),
        palette.primary,
    );
    if show_surface {
        let footer = egui::Rect::from_min_max(
            egui::pos2(rect.left(), widget_clip.bottom()),
            rect.right_bottom(),
        );
        ui.painter().rect_filled(footer, 0.0, visuals.fill);
        ui.painter()
            .line_segment([rect.left_bottom(), rect.right_bottom()], visuals.stroke);
    }

    let track = performance_track(rect);
    let value = state.get_param(id).clamp(0.0, 1.0);
    let anchor = if matches!(id, P::Transpose | P::OctaveShift) {
        0.5
    } else {
        0.0
    };
    let value_x = egui::lerp(track.left()..=track.right(), value);
    let anchor_x = egui::lerp(track.left()..=track.right(), anchor);
    ui.painter().line_segment(
        [track.left_center(), track.right_center()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.42),
        ),
    );
    ui.painter().line_segment(
        [
            egui::pos2(value_x.min(anchor_x), track.center().y),
            egui::pos2(value_x.max(anchor_x), track.center().y),
        ],
        egui::Stroke::new(indicator_height, visuals.indicator),
    );
    response
}

fn voice_mode_selector(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let split_height = editor_theme::font::CAPTION_SIZE
        + editor_theme::font::VALUE_SIZE
        + editor_theme::compact_gap(ui)
        + editor_theme::shape::STROKE;
    let split_label = rect.height() >= split_height;
    let palette = editor_theme::semantic();
    let mut combo_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("performance-voice-mode-field")
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    combo_ui.spacing_mut().interact_size.y = rect.height();
    combo_ui.spacing_mut().button_padding = egui::vec2(editor_theme::space::XXS, 0.0);
    combo_ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    combo_ui.visuals_mut().widgets.hovered.weak_bg_fill = egui::Color32::TRANSPARENT;
    combo_ui.visuals_mut().widgets.active.weak_bg_fill = egui::Color32::TRANSPARENT;
    combo_ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
    combo_ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
    combo_ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
    let value_text = voice_mode_text(state.params().voice_mode.value_u8());
    let combo_width = combo_ui.available_width();
    let response = voice_mode_combo(&mut combo_ui, state, combo_width, &value_text);
    let visuals = editor_theme::control_visuals(
        response.enabled(),
        response.hovered(),
        response.is_pointer_button_down_on(),
        response.has_focus(),
        palette.primary,
    );
    let painter = ui.painter_at(rect);
    if response.hovered() || response.is_pointer_button_down_on() || response.has_focus() {
        painter.rect(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            visuals.fill,
            visuals.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let text_width = (rect.width() - editor_theme::space::SM * 2.0).max(1.0);
    if split_label {
        let label_font = fit_font_to_width(
            &painter,
            "VOICES",
            editor_theme::font::caption(),
            text_width,
        );
        let value_font = fit_font_to_width(
            &painter,
            &value_text,
            editor_theme::font::value(),
            text_width,
        );
        let label = painter.layout_no_wrap("VOICES".to_owned(), label_font, visuals.label);
        let value = painter.layout_no_wrap(value_text, value_font, visuals.value);
        let gap = editor_theme::compact_gap(ui);
        let content_height = label.size().y + gap + value.size().y;
        let top = rect.center().y - content_height * 0.5;
        painter.galley(
            egui::pos2(rect.center().x - label.size().x * 0.5, top),
            label,
            visuals.label,
        );
        painter.galley(
            egui::pos2(
                rect.center().x - value.size().x * 0.5,
                top + editor_theme::font::CAPTION_SIZE + gap,
            ),
            value,
            visuals.value,
        );
    } else {
        let text = format!("VOICES {value_text}");
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            &text,
            fit_font_to_width(&painter, &text, editor_theme::font::value(), text_width),
            visuals.value,
        );
    }
}

fn voice_mode_text(mode: u8) -> String {
    match mode {
        0 => "MONO".to_owned(),
        1 => "LEGATO".to_owned(),
        voices => voices.to_string(),
    }
}

fn voice_mode_combo(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    selected: &str,
) -> egui::Response {
    const MODES: [u8; 11] = [0, 1, 2, 4, 6, 8, 10, 12, 16, 24, 32];
    let current = state.params().voice_mode.value_u8();
    let response = egui::ComboBox::from_id_salt("performance-voice-mode")
        .selected_text(
            egui::RichText::new(selected)
                .font(editor_theme::font::value())
                .color(egui::Color32::TRANSPARENT),
        )
        .width(width.max(editor_theme::shape::STROKE))
        .show_ui(ui, |ui| {
            for mode in MODES {
                let label = voice_mode_text(mode);
                if ui.selectable_label(current == mode, label).clicked() {
                    state.begin_edit(P::VoiceMode);
                    state.set_param(P::VoiceMode, f64::from(mode) / 32.0);
                    state.end_edit(P::VoiceMode);
                }
            }
        })
        .response;
    response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, response.enabled(), "VOICES");
        info.current_text_value = Some(selected.to_owned());
        info
    });
    response.on_hover_text("Maximum voice count and mono/legato behavior")
}

/// The header's double-click is the one unambiguous factory-reset gesture.
/// Defaults come from Truce's generated parameter metadata, so this remains
/// correct if a range's normalized representation changes later.
pub(crate) fn reset_to_defaults(state: &PluginContext<KurvParams>) {
    state.generator_stack.reset_default();
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor.collapsed_group_ids.clear();
        editor.collapsed_modulators = 0;
    }
    let parameters = [
        P::OutputDb,
        P::Osc1Enabled,
        P::Osc1Transpose,
        P::Osc1Cents,
        P::Osc1Level,
        P::Osc1Pan,
        P::Osc1PhasePosition,
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
        P::Osc2PhasePosition,
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
        P::Osc3PhasePosition,
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
        P::Lfo1RateMode,
        P::Lfo2RateMode,
        P::Lfo3RateMode,
        P::Lfo4RateMode,
        P::Lfo5Rate,
        P::Lfo5Mode,
        P::Lfo5Phase,
        P::Lfo5Sync,
        P::Lfo5Bipolar,
        P::Lfo5RateMode,
        P::Lfo6Rate,
        P::Lfo6Mode,
        P::Lfo6Phase,
        P::Lfo6Sync,
        P::Lfo6Bipolar,
        P::Lfo6RateMode,
        P::Lfo7Rate,
        P::Lfo7Mode,
        P::Lfo7Phase,
        P::Lfo7Sync,
        P::Lfo7Bipolar,
        P::Lfo7RateMode,
        P::Lfo8Rate,
        P::Lfo8Mode,
        P::Lfo8Phase,
        P::Lfo8Sync,
        P::Lfo8Bipolar,
        P::Lfo8RateMode,
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
        P::Mod9Source,
        P::Mod9Target,
        P::Mod9Amount,
        P::Mod10Source,
        P::Mod10Target,
        P::Mod10Amount,
        P::Mod11Source,
        P::Mod11Target,
        P::Mod11Amount,
        P::Mod12Source,
        P::Mod12Target,
        P::Mod12Amount,
        P::Mod13Source,
        P::Mod13Target,
        P::Mod13Amount,
        P::Mod14Source,
        P::Mod14Target,
        P::Mod14Amount,
        P::Mod15Source,
        P::Mod15Target,
        P::Mod15Amount,
        P::Mod16Source,
        P::Mod16Target,
        P::Mod16Amount,
        P::Lfo1Active,
        P::Lfo2Active,
        P::Lfo3Active,
        P::Lfo4Active,
        P::Lfo5Active,
        P::Lfo6Active,
        P::Lfo7Active,
        P::Lfo8Active,
        P::PitchBendRange,
        P::ModWheel,
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
        &state.params().lfo5_curve_state,
        &state.params().lfo6_curve_state,
        &state.params().lfo7_curve_state,
        &state.params().lfo8_curve_state,
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
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Output trim: drag horizontally; double-click to reset.");
    let mut value = state.get_param(P::OutputDb);
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
        value = default;
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
            value = normalized;
        }
        if response.drag_stopped() {
            state.end_edit(P::OutputDb);
        }
    }

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
