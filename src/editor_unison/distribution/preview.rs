//! Preview generation and frame-local caching.

use crate::pan_curve::PanShapeSegmentsRt;
use crate::voices::PanShapeSettings;
use crate::voices::{
    MAX_UNISON, SwarmMode, UnisonAlignmentMode, UnisonSettings, fill_oscillator_unison_layout,
    fill_unison_jitter_offsets_mode, unison_static_pitch_cents,
};

#[derive(Clone, PartialEq)]
struct CompactUnisonPreviewKey {
    plot: egui::Rect,
    voices: u8,
    range: u32,
    amount: u32,
    curve: u32,
    width: u32,
    phase_random: u32,
    alignment: u32,
    alignment_mode: u8,
    pan_center_x: u32,
    stereo_x: u32,
    stereo_alternate: u32,
    pan_segments: (PanShapeSegmentsRt, PanShapeSegmentsRt),
}

#[derive(Clone)]
struct CompactUnisonPreview {
    key: CompactUnisonPreviewKey,
    points: [egui::Pos2; MAX_UNISON],
}

pub(super) fn compact_unison_preview_points(
    ui: &egui::Ui,
    preview_id: egui::Id,
    config: &crate::generators::OscillatorConfig,
    plot: egui::Rect,
    pan_segments: (PanShapeSegmentsRt, PanShapeSegmentsRt),
    time: f32,
    jitter_active: bool,
) -> [egui::Pos2; MAX_UNISON] {
    let preview_key = CompactUnisonPreviewKey {
        plot,
        voices: config.unison_voices.clamp(1, MAX_UNISON as u8),
        range: config.unison_range.to_bits(),
        amount: config.unison_amount.to_bits(),
        curve: config.unison_curve.to_bits(),
        width: config.unison_width.to_bits(),
        phase_random: config.phase_random.to_bits(),
        alignment: config.unison_alignment.to_bits(),
        alignment_mode: config.unison_alignment_mode,
        pan_center_x: config.unison_pan_center_x.to_bits(),
        stereo_x: config.unison_stereo_x.to_bits(),
        stereo_alternate: config.unison_stereo_alternate.to_bits(),
        pan_segments,
    };
    let cached = (!jitter_active)
        .then(|| ui.data(|data| data.get_temp::<CompactUnisonPreview>(preview_id)))
        .flatten()
        .filter(|preview| preview.key == preview_key);
    cached.map_or_else(
        || {
            let points = generate_preview_points(config, plot, pan_segments, time);
            if !jitter_active {
                ui.data_mut(|data| {
                    data.insert_temp(
                        preview_id,
                        CompactUnisonPreview {
                            key: preview_key,
                            points,
                        },
                    );
                });
            }
            points
        },
        |preview| preview.points,
    )
}

fn generate_preview_points(
    config: &crate::generators::OscillatorConfig,
    plot: egui::Rect,
    pan_segments: (PanShapeSegmentsRt, PanShapeSegmentsRt),
    time: f32,
) -> [egui::Pos2; MAX_UNISON] {
    let voices_u8 = config.unison_voices.clamp(1, MAX_UNISON as u8);
    let voices = usize::from(voices_u8);
    let detune_field = config.unison_range * 100.0 * config.unison_amount;
    let full_scale = (config.unison_range * 100.0 * (1.0 + config.unison_jitter)).max(1.0);
    let mut jitter_offsets = [0.0_f32; MAX_UNISON];
    fill_unison_jitter_offsets_mode(
        &mut jitter_offsets[..voices],
        0.618_034,
        config.unison_jitter,
        time,
        SwarmMode::from_index(config.unison_jitter_mode),
    );
    let pan_shape = PanShapeSettings::default()
        .with_center_x(config.unison_pan_center_x)
        .with_segments(pan_segments);
    let spatial_settings = UnisonSettings::new(
        voices_u8,
        config.unison_range * 100.0,
        config.unison_width,
        config.phase_random,
        config.unison_curve,
    )
    .with_stereo_square(config.unison_stereo_alternate, config.unison_stereo_x)
    .with_pan_shape(pan_shape);
    let mut detune_positions = [0.0_f32; MAX_UNISON];
    let mut lane_left = [0.0_f32; MAX_UNISON];
    let mut lane_right = [0.0_f32; MAX_UNISON];
    fill_oscillator_unison_layout(
        spatial_settings,
        &mut detune_positions,
        &mut lane_left,
        &mut lane_right,
    );
    let alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
    let mut points = [egui::Pos2::ZERO; MAX_UNISON];
    for (index, point) in points[..voices].iter_mut().enumerate() {
        let detune = unison_static_pitch_cents(
            detune_positions[index],
            config.unison_range * 100.0,
            config.unison_amount,
            config.unison_alignment,
            alignment_mode,
        );
        let jitter = jitter_offsets[index] * detune_field;
        let left_energy = lane_left[index] * lane_left[index];
        let right_energy = lane_right[index] * lane_right[index];
        let pan = (right_energy - left_energy) / (right_energy + left_energy).max(f32::EPSILON);
        *point = egui::pos2(
            ((detune + jitter) / full_scale).mul_add(plot.width() * 0.46, plot.center().x),
            (-pan).mul_add(plot.height() * 0.38, plot.center().y),
        );
    }
    points
}
