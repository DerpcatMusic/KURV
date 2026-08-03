use egui::{Color32, Pos2, Rect, RichText, Stroke, Vec2};
use nice_plug::prelude::*;
use nice_plug_egui::{
    EguiSettings, EguiState, KeyCapture, create_egui_editor, resizable_window::ResizableWindow,
    widgets::ParamSlider,
};
use std::sync::Arc;

use crate::shape_osc::{ShapeSettings, ShapeVaOscillator};
use crate::{PureVaDispersionParams, SpectralMode};

const EDITOR_WIDTH: u32 = 820;
const EDITOR_HEIGHT: u32 = 620;
const CYCLE_POINTS: usize = 256;
const ACCENT: Color32 = Color32::from_rgb(116, 226, 180);
const MUTED: Color32 = Color32::from_rgb(139, 151, 160);

struct EditorData {
    preview: ShapeVaOscillator,
}

pub fn build(params: Arc<PureVaDispersionParams>) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state.clone(),
        EditorData {
            preview: ShapeVaOscillator::default(),
        },
        EguiSettings::default(),
        |ctx, queue, state| {
            queue.set_key_capture(KeyCapture::IgnoreAll);
            state.preview.set_sample_rate(48_000.0);
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = Color32::from_rgb(16, 19, 22);
            visuals.window_fill = visuals.panel_fill;
            visuals.selection.bg_fill = ACCENT;
            visuals.widgets.active.bg_fill = Color32::from_rgb(43, 70, 59);
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(37, 48, 48);
            ctx.set_visuals(visuals);
        },
        move |ui, setter, _queue, state| {
            ResizableWindow::new("kurv-editor")
                .min_size(Vec2::new(580.0, 440.0))
                .show(ui, egui_state.as_ref(), |ui| {
                    draw_header(ui);
                    ui.add_space(10.0);
                    draw_cycle_view(ui, params.as_ref(), &mut state.preview);
                    ui.add_space(10.0);
                    draw_controls(ui, params.as_ref(), setter);
                });
        },
    )
}

pub fn default_editor_state() -> Arc<EguiState> {
    EguiState::from_size(EDITOR_WIDTH, EDITOR_HEIGHT)
}

fn draw_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("KURV").size(28.0).strong().color(ACCENT));
        ui.add_space(10.0);
        ui.label(RichText::new("SPECTRAL VA").size(12.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("16 VOICES").size(11.0).color(MUTED));
        });
    });
}

fn draw_cycle_view(
    ui: &mut egui::Ui,
    params: &PureVaDispersionParams,
    preview: &mut ShapeVaOscillator,
) {
    let height = if ui.available_height() < 520.0 {
        130.0
    } else {
        178.0
    };
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let frame = rect.shrink2(Vec2::new(12.0, 10.0));

    painter.rect_filled(rect, 6.0, Color32::from_rgb(10, 13, 15));
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, Color32::from_rgb(43, 50, 55)),
        egui::StrokeKind::Inside,
    );
    draw_grid(&painter, frame);

    preview.set_effect(params.spectral_effect.value().into());
    let settings = ShapeSettings {
        waveform: params.wave.value().into(),
        frequency_hz: params.drone_frequency.value(),
        pulse_width: params.pulse_width.value(),
        center_hz: params.center_hz.value(),
        spread_octaves: params.spread_octaves.value(),
        mix: params.mix.value(),
        sweep_phase: 0.0,
        keytrack: params.keytrack.value(),
        stereo_offset: 0.0,
    };
    let mut cycle = [0.0_f32; CYCLE_POINTS];
    preview.write_preview(settings, &mut cycle);

    let mut points = Vec::with_capacity(CYCLE_POINTS);
    for (index, sample) in cycle.iter().enumerate() {
        let x = frame.left() + frame.width() * index as f32 / CYCLE_POINTS as f32;
        let y = frame.center().y - sample.clamp(-1.0, 1.0) * frame.height() * 0.46;
        points.push(Pos2::new(x, y));
    }
    painter.line(points, Stroke::new(1.8, ACCENT));
    painter.text(
        frame.left_top(),
        egui::Align2::LEFT_TOP,
        effect_name(params.spectral_effect.value()),
        egui::FontId::monospace(10.0),
        MUTED,
    );
}

fn effect_name(effect: SpectralMode) -> &'static str {
    match effect {
        SpectralMode::PhaseDisperse => "PHASE DISPERSE",
        SpectralMode::HarmonicStretch => "HARMONIC STRETCH",
        SpectralMode::Formant => "FORMANT",
        SpectralMode::SpectralFold => "SPECTRAL FOLD",
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect) {
    let grid = Color32::from_rgb(29, 35, 39);
    painter.line_segment(
        [
            Pos2::new(rect.left(), rect.center().y),
            Pos2::new(rect.right(), rect.center().y),
        ],
        Stroke::new(1.0, Color32::from_rgb(47, 56, 62)),
    );
    for index in 1..8 {
        let x = rect.left() + rect.width() * index as f32 / 8.0;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, grid),
        );
    }
}

fn draw_controls(ui: &mut egui::Ui, params: &PureVaDispersionParams, setter: &ParamSetter) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if ui.available_width() >= 700.0 {
                ui.columns(2, |columns| {
                    oscillator_section(&mut columns[0], params, setter);
                    spectral_section(&mut columns[1], params, setter);
                });
                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    filter_section(&mut columns[0], params, setter);
                    envelope_section(&mut columns[1], params, setter);
                });
            } else {
                oscillator_section(ui, params, setter);
                ui.add_space(8.0);
                spectral_section(ui, params, setter);
                ui.add_space(8.0);
                filter_section(ui, params, setter);
                ui.add_space(8.0);
                envelope_section(ui, params, setter);
            }
            ui.add_space(8.0);
            section(ui, "MASTER", |ui| {
                control_row(ui, "Output", &params.output_db, setter);
                control_row(ui, "Audition", &params.drone, setter);
                control_row(ui, "Drone Pitch", &params.drone_frequency, setter);
            });
        });
}

fn oscillator_section(ui: &mut egui::Ui, params: &PureVaDispersionParams, setter: &ParamSetter) {
    section(ui, "OSCILLATOR", |ui| {
        control_row(ui, "Wave", &params.wave, setter);
        control_row(ui, "Pulse Width", &params.pulse_width, setter);
        control_row(ui, "Stereo", &params.stereo_offset_octaves, setter);
    });
}

fn spectral_section(ui: &mut egui::Ui, params: &PureVaDispersionParams, setter: &ParamSetter) {
    section(ui, "SPECTRAL", |ui| {
        control_row(ui, "Effect", &params.spectral_effect, setter);
        control_row(ui, "Amount", &params.mix, setter);
        control_row(ui, "Focus", &params.center_hz, setter);
        control_row(ui, "Shape", &params.spread_octaves, setter);
        control_row(ui, "Motion", &params.sweep_rate_hz, setter);
        control_row(ui, "Keytrack", &params.keytrack, setter);
    });
}

fn filter_section(ui: &mut egui::Ui, params: &PureVaDispersionParams, setter: &ParamSetter) {
    section(ui, "FILTER", |ui| {
        control_row(ui, "Cutoff", &params.cutoff_hz, setter);
        control_row(ui, "Resonance", &params.resonance, setter);
        control_row(ui, "Envelope", &params.filter_env, setter);
    });
}

fn envelope_section(ui: &mut egui::Ui, params: &PureVaDispersionParams, setter: &ParamSetter) {
    section(ui, "AMP ENVELOPE", |ui| {
        control_row(ui, "Attack", &params.attack, setter);
        control_row(ui, "Decay", &params.decay, setter);
        control_row(ui, "Sustain", &params.sustain, setter);
        control_row(ui, "Release", &params.release, setter);
    });
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new(title).size(11.0).strong().color(ACCENT));
        ui.add_space(4.0);
        body(ui);
    });
}

fn control_row<P: Param>(ui: &mut egui::Ui, label: &str, param: &P, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [82.0, 18.0],
            egui::Label::new(RichText::new(label).size(12.0).color(MUTED)),
        );
        ui.add(ParamSlider::for_param(param, setter).with_width(ui.available_width().max(110.0)));
    });
    ui.add_space(3.0);
}
