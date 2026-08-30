//! Compact two-panel RESYNTH editor.

mod source_io;

use std::{sync::Arc, time::Duration};

use truce_core::editor::PluginContext;

use crate::{
    KurvParams, editor_controls, editor_shell, editor_theme,
    editor_widgets::with_child,
    generators::{ModuleId, OscillatorConfig, OscillatorSlot},
    modulators::routing::{ModulationRouteTarget, OscillatorControl},
    oscillators::{
        AlgorithmVisualCache, GrainDirection, ResynthAlgorithm, ResynthControls,
        ResynthVisualModel, SourceWaveBin,
    },
    resynth_state::ResynthTelemetrySnapshot,
};

const POLL: Duration = Duration::from_millis(40);
#[cfg(test)]
use source_io::{ImportWorkerPermit, create_unique_export_temp};
use source_io::{handle_import, take_export_result};

/// Adapter seam for the immutable, worker-produced source-time visuals owned by
/// the state layer. The editor never builds or mutates this cache while painting.
type SourceVisualCache = ResynthVisualModel;

fn source_visual_cache(
    source: &crate::resynth_state::ResynthSlotState,
) -> Option<Arc<SourceVisualCache>> {
    source.visual_model()
}

pub(crate) fn draw_resynth_body(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    plot: egui::Rect,
    slot: OscillatorSlot,
    module_id: ModuleId,
    config: &mut OscillatorConfig,
) -> bool {
    if let Some(message) = take_export_result(module_id) {
        set_status(ui, module_id, message);
    }
    let palette = editor_theme::semantic();
    let source = state.resynth_assets.slot_arc(slot.index());
    let summary = source
        .as_deref()
        .and_then(crate::resynth_state::ResynthSlotState::source_summary);
    let selected = summary
        .as_ref()
        .map_or(ResynthAlgorithm::Grain, |value| value.selected);
    let visual_model = source.as_deref().and_then(source_visual_cache);
    let artifact_visual = source
        .as_deref()
        .and_then(crate::resynth_state::ResynthSlotState::algorithm_visual_snapshot);
    let visual_stale = summary
        .as_ref()
        .is_some_and(|value| value.desired_revision != value.sounding_revision);
    let artifact_visual_current = artifact_visual.as_ref().is_some_and(|visual| {
        !visual_stale
            && visual.algorithm == selected
            && summary
                .as_ref()
                .is_some_and(|value| visual.revision == value.sounding_revision)
    });
    let artifact_cache = artifact_visual
        .as_ref()
        .filter(|_| artifact_visual_current)
        .map(|visual| visual.cache.as_ref());
    let telemetry = source
        .as_deref()
        .map(crate::resynth_state::ResynthSlotState::telemetry_snapshot);
    if source.is_some() {
        // The first snapshot arms the fixed audio-callback interest lease; a
        // follow-up paint is required to observe the first publication even
        // when the editor was otherwise idle.
        editor_theme::request_display_repaint(ui);
        ui.ctx().request_repaint_after(POLL);
    }
    let build_pending = summary.as_ref().is_some_and(|value| {
        value.desired_revision != value.sounding_revision && !value.build_failed
    });
    let has_committed_source = source
        .as_deref()
        .is_some_and(crate::resynth_state::ResynthSlotState::has_source);
    let completion_id = egui::Id::new(("resynth-sounding-revision", module_id.get()));
    if let Some(status) = summary.as_ref() {
        let previous = ui.data(|data| data.get_temp::<u64>(completion_id));
        if previous.is_none() {
            ui.data_mut(|data| data.insert_temp(completion_id, status.sounding_revision));
        } else if previous != Some(status.sounding_revision) {
            ui.data_mut(|data| data.insert_temp(completion_id, status.sounding_revision));
            editor_shell::request_structural_commit(ui);
        }
        if status.desired_revision != status.sounding_revision && !status.build_failed {
            ui.ctx().request_repaint_after(POLL);
        }
    }
    let mut changed = false;
    let mut controls = summary
        .as_ref()
        .map_or_else(ResynthControls::default, |value| value.controls);

    let source_response = ui
        .interact(
            plot,
            egui::Id::new(("resynth-source-drop", module_id.get())),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let source_response = if summary.is_none() {
        source_response.on_hover_text("Drop one WAV, FLAC, AIFF, OGG, or MP3")
    } else {
        source_response
    };
    paint_compact_source(
        ui,
        plot,
        summary.as_ref(),
        visual_model.as_deref(),
        artifact_cache,
        visual_stale,
        selected,
        telemetry.as_ref(),
        config.level,
        controls,
    );
    if has_committed_source {
        changed |= update_source_timeline(ui, &source_response, module_id, plot, &mut controls);
        if changed && let Some(source) = source.as_deref() {
            source.apply_live_controls(controls);
        }
    }
    let toolbar = egui::Rect::from_min_max(
        plot.left_top(),
        egui::pos2(plot.right(), plot.top() + editor_theme::title_height(ui)),
    );
    let mut browse_response = None;
    with_child(
        ui,
        toolbar,
        ("resynth-readouts", module_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.horizontal(|ui| {
                let title_reserve = ui
                    .painter()
                    .layout_no_wrap(
                        "RESYNTH".to_owned(),
                        editor_theme::font::title(),
                        palette.primary,
                    )
                    .size()
                    .x
                    + editor_theme::space::SM;
                ui.add_space(title_reserve);
                for algorithm in ResynthAlgorithm::VISIBLE {
                    let eligible = algorithm == ResynthAlgorithm::Grain
                        || summary.as_ref().is_some_and(|value| {
                            value.root_override_hz.is_some() || value.estimated_root_hz.is_some()
                        });
                    let response = ui
                        .add_enabled_ui(eligible && has_committed_source, |ui| {
                            ui.selectable_label(
                                selected == algorithm,
                                egui::RichText::new(algorithm.label()).color(palette.unison),
                            )
                        })
                        .inner
                        .on_disabled_hover_text("A stable detected pitch is required")
                        .on_hover_text(match algorithm {
                            ResynthAlgorithm::Sample => "Legacy compatibility playback",
                            ResynthAlgorithm::Grain => {
                                "Time-stretched source playback with independently scheduled grains"
                            }
                            ResynthAlgorithm::Rich => {
                                "Source-filter resynthesis: pitch moves, formants stay"
                            }
                        });
                    if response.clicked()
                        && selected != algorithm
                        && let Some(slot_state) = source.as_ref()
                        && slot_state.request_algorithm(algorithm).is_some()
                    {
                        set_status(ui, module_id, format!("Building {}", algorithm.label()));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    browse_response = Some(
                        ui.add_enabled(
                            !build_pending,
                            egui::Button::new(egui_phosphor::regular::FOLDER_OPEN).frame(false),
                        )
                        .on_hover_text("Load a sample"),
                    );
                    let status = summary.as_ref().map_or_else(String::new, |value| {
                        if value.build_failed {
                            "failed".to_owned()
                        } else if value.desired_revision != value.sounding_revision {
                            format!("{}%", value.progress_percent)
                        } else {
                            String::new()
                        }
                    });
                    if !status.is_empty() {
                        ui.label(
                            egui::RichText::new(status)
                                .font(editor_theme::font::caption())
                                .color(if build_pending {
                                    palette.unison
                                } else {
                                    palette.text_muted
                                }),
                        );
                    }
                    if let Some(summary) = summary.as_ref() {
                        ui.label(
                            egui::RichText::new(truncate(&summary.file_name, 20))
                                .font(editor_theme::font::caption())
                                .color(palette.text_muted),
                        );
                    }
                });
            });
        },
    );
    handle_import(
        ui,
        state,
        &source_response,
        browse_response.as_ref().unwrap_or(&source_response),
        build_pending,
        slot,
        module_id,
        controls,
    );
    changed
}

fn paint_compact_source(
    ui: &egui::Ui,
    rect: egui::Rect,
    summary: Option<&crate::resynth_state::ResynthSourceSummary>,
    visual: Option<&SourceVisualCache>,
    artifact_visual: Option<&AlgorithmVisualCache>,
    _cache_stale: bool,
    algorithm: ResynthAlgorithm,
    telemetry: Option<&ResynthTelemetrySnapshot>,
    level: f32,
    controls: ResynthControls,
) {
    let palette = editor_theme::semantic();
    let accent = algorithm_accent(algorithm);
    match summary {
        Some(source) => {
            if algorithm == ResynthAlgorithm::Rich
                && let Some(cache) = artifact_visual
            {
                paint_rich_zones(ui, rect, cache, telemetry, accent, level);
            } else {
                paint_source_waveform(ui, rect, visual, accent, level);
            }
            if let Some(cache) = artifact_visual {
                match algorithm {
                    ResynthAlgorithm::Sample => {
                        if let Some(receipt) = cache.sample_loop() {
                            paint_sample_loop_region(ui, rect, receipt);
                        }
                    }
                    ResynthAlgorithm::Grain => {
                        paint_grain_candidates(ui, rect, cache.grain_candidates());
                        paint_pitch_curve(ui, rect, cache.pitch_curve());
                        paint_live_grains(ui, rect, telemetry, source.controls.grain_size);
                    }
                    ResynthAlgorithm::Rich => {
                        paint_pitch_curve(ui, rect, cache.pitch_curve());
                    }
                }
            } else if algorithm == ResynthAlgorithm::Grain {
                paint_live_grains(ui, rect, telemetry, source.controls.grain_size);
            }
            paint_source_timeline(ui, rect, controls, accent);
        }
        None => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "DROP AUDIO",
                editor_theme::font::label(),
                palette.primary,
            );
        }
    }
}

fn paint_source_timeline(
    ui: &egui::Ui,
    rect: egui::Rect,
    controls: ResynthControls,
    accent: egui::Color32,
) {
    let (start, end) = controls.loop_bounds();
    let start_x = egui::lerp(rect.x_range(), start);
    let end_x = egui::lerp(rect.x_range(), end);
    let shade = editor_theme::semantic().surface.gamma_multiply(0.62);
    ui.painter().rect_filled(
        egui::Rect::from_min_max(rect.min, egui::pos2(start_x, rect.bottom())),
        0.0,
        shade,
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_max(egui::pos2(end_x, rect.top()), rect.max),
        0.0,
        shade,
    );
    for x in [start_x, end_x] {
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.86)),
        );
        ui.painter().circle_filled(
            egui::pos2(x, rect.top() + editor_theme::space::SM),
            editor_theme::space::XS,
            accent,
        );
    }
    let playhead_x = egui::lerp(rect.x_range(), controls.position.clamp(start, end));
    ui.painter().line_segment(
        [
            egui::pos2(playhead_x, rect.top()),
            egui::pos2(playhead_x, rect.bottom()),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE * 1.5, accent),
    );
}

fn update_source_timeline(
    ui: &egui::Ui,
    response: &egui::Response,
    module_id: ModuleId,
    rect: egui::Rect,
    controls: &mut ResynthControls,
) -> bool {
    let Some(pointer) = response.interact_pointer_pos() else {
        return false;
    };
    let (loop_start, loop_end) = controls.loop_bounds();
    let position = ((pointer.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
    let drag_id = egui::Id::new(("resynth-timeline-target", module_id.get()));
    if response.drag_started() || response.clicked() {
        let start_x = egui::lerp(rect.x_range(), loop_start);
        let end_x = egui::lerp(rect.x_range(), loop_end);
        let radius = editor_theme::space::MD;
        let target = if (pointer.x - start_x).abs() <= radius {
            1_u8
        } else if (pointer.x - end_x).abs() <= radius {
            2_u8
        } else {
            0_u8
        };
        ui.data_mut(|data| data.insert_temp(drag_id, target));
    }
    if !(response.dragged() || response.clicked()) {
        return false;
    }
    controls.loop_start = loop_start;
    controls.loop_end = loop_end;
    match ui.data(|data| data.get_temp::<u8>(drag_id)).unwrap_or(0) {
        1 => controls.loop_start = position.min(controls.loop_end - 0.001),
        2 => controls.loop_end = position.max(controls.loop_start + 0.001),
        _ => controls.position = position.clamp(controls.loop_start, controls.loop_end),
    }
    true
}

#[cfg(test)]
const ROOT_NOTE_MIN: i32 = 36;
#[cfg(test)]
const ROOT_NOTE_MAX: i32 = 95;
const DEFAULT_ROOT_MIDI: i32 = 48;

#[cfg(test)]
fn hz_to_midi_note(hz: f32) -> i32 {
    if !hz.is_finite() || hz <= 0.0 {
        return DEFAULT_ROOT_MIDI;
    }
    (69.0 + 12.0 * (hz / 440.0).log2())
        .round()
        .clamp(ROOT_NOTE_MIN as f32, ROOT_NOTE_MAX as f32) as i32
}

#[cfg(test)]
fn midi_note_label(note: i32) -> String {
    const NOTE_NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    let note = note.clamp(ROOT_NOTE_MIN, ROOT_NOTE_MAX);
    format!(
        "{}{}",
        NOTE_NAMES[note.rem_euclid(12) as usize],
        note.div_euclid(12) - 1
    )
}

fn paint_sample_loop_region(
    ui: &egui::Ui,
    rect: egui::Rect,
    receipt: crate::oscillators::SampleLoopVisualMetadata,
) {
    if receipt.source_total_frames == 0 || receipt.source_span_frames == 0 {
        return;
    }
    let palette = editor_theme::semantic();
    let start = receipt.start_normalized().clamp(0.0, 1.0);
    let end = receipt.end_normalized().clamp(start, 1.0);
    let loop_rect = egui::Rect::from_min_max(
        egui::pos2(egui::lerp(rect.x_range(), start), rect.top()),
        egui::pos2(egui::lerp(rect.x_range(), end), rect.bottom()),
    );
    ui.painter()
        .rect_filled(loop_rect, 0.0, palette.primary.gamma_multiply(0.10));
    for x in [loop_rect.left(), loop_rect.right()] {
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, palette.primary.gamma_multiply(0.8)),
        );
    }
    let seam_start = (end
        - receipt.crossfade_frames as f32 / receipt.source_total_frames.max(1) as f32)
        .clamp(start, end);
    let seam = egui::Rect::from_min_max(
        egui::pos2(egui::lerp(rect.x_range(), seam_start), rect.top()),
        loop_rect.right_bottom(),
    );
    ui.painter()
        .rect_filled(seam, 0.0, palette.unison.gamma_multiply(0.13));
}

fn paint_live_grains(
    ui: &egui::Ui,
    rect: egui::Rect,
    telemetry: Option<&ResynthTelemetrySnapshot>,
    grain_size: f32,
) {
    let Some(telemetry) = telemetry.filter(|frame| !frame.stale) else {
        return;
    };
    let palette = editor_theme::semantic();
    let width = (rect.width() * (0.028 + grain_size * 0.08)).clamp(8.0, 36.0);
    for lane in telemetry.grain_lanes {
        if !lane.active {
            continue;
        }
        let x = egui::lerp(rect.x_range(), lane.position.clamp(0.0, 1.0));
        let y = (-lane.pan).mul_add(rect.height() * 0.38, rect.center().y);
        let remaining = (1.0 - lane.progress).clamp(0.12, 1.0);
        let height = (rect.height() * 0.22 * remaining * lane.gain.max(0.18)).max(4.0);
        let capsule = egui::Rect::from_center_size(
            egui::pos2(x, y),
            egui::vec2(width * remaining.max(0.35), height),
        );
        let pitch = (lane.pitch / 24.0).clamp(-1.0, 1.0);
        let fill = if pitch >= 0.0 {
            mix_rgb(palette.unison, palette.envelope, pitch)
                .gamma_multiply((0.28 + lane.gain * 0.62).min(0.92))
        } else {
            mix_rgb(palette.unison, palette.primary, -pitch)
                .gamma_multiply((0.28 + lane.gain * 0.62).min(0.92))
        };
        ui.painter()
            .rect_filled(capsule, capsule.height() * 0.5, fill);
        ui.painter().rect_stroke(
            capsule,
            capsule.height() * 0.5,
            egui::Stroke::new(1.0_f32, fill.gamma_multiply(1.15)),
            egui::StrokeKind::Middle,
        );
    }
}

fn mix_rgb(from: egui::Color32, to: egui::Color32, amount: f32) -> egui::Color32 {
    let amount = amount.clamp(0.0, 1.0);
    egui::Color32::from_rgb(
        (f32::from(from.r()) + (f32::from(to.r()) - f32::from(from.r())) * amount) as u8,
        (f32::from(from.g()) + (f32::from(to.g()) - f32::from(from.g())) * amount) as u8,
        (f32::from(from.b()) + (f32::from(to.b()) - f32::from(from.b())) * amount) as u8,
    )
}

fn paint_rich_zones(
    ui: &egui::Ui,
    rect: egui::Rect,
    cache: &AlgorithmVisualCache,
    _telemetry: Option<&ResynthTelemetrySnapshot>,
    accent: egui::Color32,
    level: f32,
) {
    const DISPLAY_BINS: usize = 56;
    const DISPLAY_COLUMNS: usize = 96;
    const DB_FLOOR: f32 = -96.0;

    let palette = editor_theme::semantic();
    let timeline = cache.rich_timeline_db();
    let waveform_height = (rect.height() * 0.28).max(18.0);
    let spectrum_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.bottom() - waveform_height),
    );
    let waveform_rect =
        egui::Rect::from_min_max(egui::pos2(rect.left(), spectrum_rect.bottom()), rect.max);
    let column_width = spectrum_rect.width() / DISPLAY_COLUMNS as f32;
    let bin_height = spectrum_rect.height() / DISPLAY_BINS as f32;
    for column in 0..DISPLAY_COLUMNS {
        let frame = column as f32 * (timeline.len() - 1) as f32 / (DISPLAY_COLUMNS - 1) as f32;
        let first = frame.floor() as usize;
        let second = (first + 1).min(timeline.len() - 1);
        let mix = frame.fract();
        for bin in 0..DISPLAY_BINS {
            let start = bin * timeline[first].len() / DISPLAY_BINS;
            let end = ((bin + 1) * timeline[first].len() / DISPLAY_BINS)
                .max(start + 1)
                .min(timeline[first].len());
            let db = (start..end).fold(DB_FLOOR, |db, index| {
                db.max(egui::lerp(
                    timeline[first][index]..=timeline[second][index],
                    mix,
                ))
            });
            let energy = ((db - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0).powi(2);
            let x = spectrum_rect.left() + column as f32 * column_width;
            let y = spectrum_rect.bottom() - (bin + 1) as f32 * bin_height;
            let cell = egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(column_width.max(0.5), bin_height.max(0.5)),
            );
            ui.painter().rect_filled(
                cell,
                0.0,
                mix_rgb(palette.well, accent, energy).gamma_multiply(0.18_f32 + energy * 0.78_f32),
            );
        }
    }
    for frame in 1..timeline.len() {
        let x = egui::lerp(
            spectrum_rect.x_range(),
            frame as f32 / timeline.len() as f32,
        );
        ui.painter().line_segment(
            [
                egui::pos2(x, spectrum_rect.top()),
                egui::pos2(x, spectrum_rect.bottom()),
            ],
            egui::Stroke::new(0.5_f32, palette.grid.gamma_multiply(0.38)),
        );
    }
    ui.painter().line_segment(
        [waveform_rect.left_top(), waveform_rect.right_top()],
        egui::Stroke::new(1.0_f32, palette.grid.gamma_multiply(0.72)),
    );
    paint_waveform_bins(ui, waveform_rect, cache.rich_waveform(), accent, level);
}

fn paint_pitch_curve(ui: &egui::Ui, rect: egui::Rect, curve: &[f32]) {
    if curve.len() < 2 || curve.iter().all(|value| *value <= f32::EPSILON) {
        return;
    }
    let palette = editor_theme::semantic();
    let denominator = curve.len().saturating_sub(1).max(1) as f32;
    let mut previous = None;
    for (index, value) in curve.iter().copied().enumerate() {
        if value <= f32::EPSILON {
            previous = None;
            continue;
        }
        let pos = egui::pos2(
            egui::lerp(rect.x_range(), index as f32 / denominator),
            egui::lerp(rect.y_range(), 1.0 - value.clamp(0.0, 1.0)),
        );
        if let Some(last) = previous {
            ui.painter().line_segment(
                [last, pos],
                egui::Stroke::new(1.15_f32, palette.unison.gamma_multiply(0.85)),
            );
        }
        previous = Some(pos);
    }
}

fn paint_grain_candidates(ui: &egui::Ui, rect: egui::Rect, candidates: &[f32]) {
    let palette = editor_theme::semantic();
    for position in candidates.iter().copied() {
        let x = egui::lerp(rect.x_range(), position.clamp(0.0, 1.0));
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.bottom() - 10.0),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, palette.unison.gamma_multiply(0.7)),
        );
    }
}

fn paint_source_waveform(
    ui: &egui::Ui,
    rect: egui::Rect,
    visual: Option<&SourceVisualCache>,
    color: egui::Color32,
    level: f32,
) {
    let palette = editor_theme::semantic();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.center().y),
            egui::pos2(rect.right(), rect.center().y),
        ],
        egui::Stroke::new(1.0_f32, palette.grid.gamma_multiply(0.32)),
    );
    let Some(visual) = visual else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "ANALYZING SOURCE…",
            editor_theme::font::caption(),
            palette.text_muted,
        );
        return;
    };
    paint_waveform_bins(ui, rect, visual.waveform(), color, level);
}

fn paint_waveform_bins(
    ui: &egui::Ui,
    rect: egui::Rect,
    waveform: &[SourceWaveBin],
    color: egui::Color32,
    level: f32,
) {
    let count = waveform.len();
    if count == 0 {
        return;
    }
    let denominator = count.saturating_sub(1).max(1) as f32;
    let half_height = rect.height() * 0.42;
    for index in 0..count {
        let x = egui::lerp(rect.x_range(), index as f32 / denominator);
        let min = waveform[index].min.clamp(-1.0, 1.0) * level;
        let max = waveform[index].max.clamp(-1.0, 1.0) * level;
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.center().y - max * half_height),
                egui::pos2(x, rect.center().y - min * half_height),
            ],
            egui::Stroke::new(0.9_f32, color.gamma_multiply(0.46)),
        );
    }
    let rms = waveform
        .iter()
        .take(count)
        .enumerate()
        .map(|(index, bin)| {
            egui::pos2(
                egui::lerp(rect.x_range(), index as f32 / denominator),
                rect.bottom() - bin.rms.clamp(0.0, 1.0) * level * rect.height() * 0.34,
            )
        })
        .collect::<Vec<_>>();
    if rms.len() > 1 {
        ui.painter()
            .add(egui::Shape::line(rms, egui::Stroke::new(1.2_f32, color)));
    }
}

#[cfg(test)]
mod visualization_tests {
    use super::{
        DEFAULT_ROOT_MIDI, ImportWorkerPermit, create_unique_export_temp, format_pitch,
        hz_to_midi_note, midi_note_label,
    };

    #[test]
    fn import_worker_budget_rejects_concurrent_analysis() {
        let first = ImportWorkerPermit::try_acquire().expect("first import permit");
        assert!(ImportWorkerPermit::try_acquire().is_err());
        drop(first);
        assert!(ImportWorkerPermit::try_acquire().is_ok());
    }

    #[test]
    fn unique_export_temp_preserves_preexisting_predictable_sibling() {
        let directory = std::env::temp_dir().join(format!(
            "kurv-export-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("directory");
        let destination = directory.join("source.wav");
        let predictable = directory.join("source.wav.tmp");
        std::fs::write(&predictable, b"sentinel").expect("sentinel");
        let (temporary, file) = create_unique_export_temp(&destination).expect("unique temp");
        assert_ne!(temporary.path, predictable);
        drop(file);
        drop(temporary);
        assert_eq!(std::fs::read(&predictable).expect("read"), b"sentinel");
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn pitch_label_reports_note_hz_and_cents() {
        assert_eq!(format_pitch(440.0), "A4 · 440.00 Hz · +0.0¢");
        assert!(format_pitch(445.0).starts_with("A4 · 445.00 Hz · +19."));
        assert_eq!(format_pitch(f32::NAN), "UNKNOWN");
    }

    #[test]
    fn root_note_picker_maps_c3_and_a4() {
        assert_eq!(midi_note_label(DEFAULT_ROOT_MIDI), "C3");
        assert_eq!(hz_to_midi_note(crate::dsp::midi_note_hz(69.0)), 69);
        assert_eq!(midi_note_label(69), "A4");
        assert!((crate::dsp::midi_note_hz(69.0) - 440.0).abs() < 1.0e-3);
    }
}

pub(crate) fn draw_grain_shape_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    module_id: ModuleId,
    plot: egui::Rect,
) -> bool {
    let Some(source) = state.resynth_assets.slot_arc(slot.index()) else {
        return false;
    };
    let mut controls = source
        .source_summary()
        .map(|summary| summary.controls)
        .or_else(|| source.rt_grain_controls())
        .unwrap_or_default();
    if controls.grain_density < 1.0 {
        controls.grain_density = 1.0;
    }
    let detail_height = (plot.height() * 0.46)
        .max(editor_theme::font::VALUE_SIZE * 4.0)
        .min(plot.height() * 0.72);
    let envelope = egui::Rect::from_min_max(
        plot.min,
        egui::pos2(plot.right(), plot.bottom() - detail_height),
    );
    let details = egui::Rect::from_min_max(egui::pos2(plot.left(), envelope.bottom()), plot.max);
    let grain_curve = match slot.index() {
        0 => &state.params().osc1_grain_curve_state,
        1 => &state.params().osc2_grain_curve_state,
        _ => &state.params().osc3_grain_curve_state,
    };
    crate::editor_lfo::draw_curve_state_in_rect(
        ui,
        grain_curve,
        envelope,
        ("grain-envelope", slot.index()),
        editor_theme::semantic().unison,
        &crate::wave_curve::default_grain_curve(),
    );
    let mut changed = false;
    let defaults = ResynthControls::default();
    let cell_width = details.width() / 4.0;
    let cell_height = details.height() / 2.0;
    for (index, metric) in [
        GrainDetailMetric::Rate,
        GrainDetailMetric::Length,
        GrainDetailMetric::Pitch,
        GrainDetailMetric::Tune,
        GrainDetailMetric::Position,
        GrainDetailMetric::Pan,
        GrainDetailMetric::Normalize,
        GrainDetailMetric::Reverse,
    ]
    .into_iter()
    .enumerate()
    {
        let cell = egui::Rect::from_min_size(
            egui::pos2(
                details.left() + (index % 4) as f32 * cell_width,
                details.top() + (index / 4) as f32 * cell_height,
            ),
            egui::vec2(cell_width, cell_height),
        );
        with_child(
            ui,
            cell,
            ("grain-detail", slot.index(), index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                changed |= grain_detail_readout(
                    ui,
                    state,
                    module_id,
                    slot,
                    &mut controls,
                    defaults,
                    metric,
                    cell,
                );
            },
        );
    }
    if changed {
        source.apply_live_controls(controls);
    }
    changed
}

#[derive(Clone, Copy)]
enum GrainDetailMetric {
    Rate,
    Length,
    Pitch,
    Tune,
    Position,
    Pan,
    Normalize,
    Reverse,
}

fn grain_detail_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    controls: &mut ResynthControls,
    defaults: ResynthControls,
    metric: GrainDetailMetric,
    cell: egui::Rect,
) -> bool {
    match metric {
        GrainDetailMetric::Rate => grain_detail_value(
            ui,
            &mut controls.grain_density,
            1.0..=2_000.0,
            1.0,
            defaults.grain_density,
            "RATE",
            grain_rate_text,
            cell,
        ),
        GrainDetailMetric::Length => grain_paired_readout(
            ui,
            slot,
            "LENGTH",
            &mut controls.grain_size,
            0.0..=1.0,
            0.01,
            defaults.grain_size,
            grain_size_text,
            &mut controls.grain_timing,
            0.0..=1.0,
            0.01,
            defaults.grain_timing,
            grain_spread_percent_text,
            cell,
        ),
        GrainDetailMetric::Pitch => grain_paired_readout(
            ui,
            slot,
            "PITCH",
            &mut controls.grain_pitch,
            -24.0..=24.0,
            0.1,
            defaults.grain_pitch,
            grain_pitch_text,
            &mut controls.grain_pitch_spread,
            0.0..=24.0,
            0.005,
            defaults.grain_pitch_spread,
            grain_pitch_spread_text,
            cell,
        ),
        GrainDetailMetric::Tune => grain_modular_percent_readout(
            ui,
            state,
            module_id,
            slot,
            "TUNE",
            "",
            &mut controls.grain_tune,
            defaults.grain_tune,
            OscillatorControl::GrainTune,
            cell,
            false,
        ),
        GrainDetailMetric::Position => grain_paired_readout(
            ui,
            slot,
            "POSITION",
            &mut controls.position,
            0.0..=1.0,
            0.01,
            defaults.position,
            grain_percent_text,
            &mut controls.grain_spray,
            0.0..=1.0,
            0.01,
            defaults.grain_spray,
            grain_spread_percent_text,
            cell,
        ),
        GrainDetailMetric::Pan => grain_paired_readout(
            ui,
            slot,
            "PAN",
            &mut controls.grain_pan,
            -1.0..=1.0,
            0.01,
            defaults.grain_pan,
            grain_pan_text,
            &mut controls.grain_pan_spread,
            0.0..=1.0,
            0.01,
            defaults.grain_pan_spread,
            grain_spread_percent_text,
            cell,
        ),
        GrainDetailMetric::Normalize => grain_detail_scalar(
            ui,
            &mut controls.grain_normalize,
            defaults.grain_normalize,
            "NORM",
            cell,
        ),
        GrainDetailMetric::Reverse => grain_detail_scalar(
            ui,
            &mut controls.grain_reverse,
            defaults.grain_reverse,
            "REVERSE",
            cell,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn grain_detail_value(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    label: &str,
    format: fn(f32) -> String,
    cell: egui::Rect,
) -> bool {
    let text = format(*value);
    let (rect, response, changed) =
        grain_scalar_drag(ui, value, range, speed, default, label, &text, cell.size());
    paint_grain_metric_readout(
        ui,
        rect,
        label,
        &format(*value),
        response.hovered(),
        response.is_pointer_button_down_on() || response.dragged(),
    );
    changed
}

fn grain_detail_scalar(
    ui: &mut egui::Ui,
    value: &mut f32,
    default: f32,
    label: &str,
    cell: egui::Rect,
) -> bool {
    let text = grain_percent_text(*value);
    let (rect, response, changed) = grain_scalar_drag(
        ui,
        value,
        0.0..=1.0,
        0.01,
        default,
        label,
        &text,
        cell.size(),
    );
    paint_grain_metric_readout(
        ui,
        rect,
        label,
        &grain_percent_text(*value),
        response.hovered(),
        response.is_pointer_button_down_on() || response.dragged(),
    );
    changed
}

pub(crate) fn draw_algorithm_controls_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    module_id: ModuleId,
    readouts: egui::Rect,
) -> bool {
    let Some(source) = state.resynth_assets.slot_arc(slot.index()) else {
        return false;
    };
    let Some(summary) = source.source_summary() else {
        return false;
    };
    match summary.selected {
        ResynthAlgorithm::Grain | ResynthAlgorithm::Sample => draw_grain_controls_panel(
            ui,
            state,
            slot,
            module_id,
            readouts,
            &source,
            summary.controls,
        ),
        ResynthAlgorithm::Rich => draw_rich_controls_panel(
            ui,
            state,
            slot,
            module_id,
            readouts,
            &source,
            summary.controls,
            summary.desired_revision != summary.sounding_revision && !summary.build_failed,
        ),
    }
}

pub(crate) fn draw_resynth_primary_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    module_id: ModuleId,
    readouts: egui::Rect,
    config: &mut OscillatorConfig,
) -> bool {
    let Some(source) = state.resynth_assets.slot_arc(slot.index()) else {
        return false;
    };
    let mut controls = source
        .source_summary()
        .map(|summary| summary.controls)
        .or_else(|| source.rt_grain_controls())
        .unwrap_or_default();
    let defaults = ResynthControls::default();
    let cell_width = readouts.width() / 5.0;
    let cells: [egui::Rect; 5] = std::array::from_fn(|index| {
        egui::Rect::from_min_max(
            egui::pos2(readouts.left() + cell_width * index as f32, readouts.top()),
            egui::pos2(
                if index == 4 {
                    readouts.right()
                } else {
                    readouts.left() + cell_width * (index + 1) as f32
                },
                readouts.bottom(),
            ),
        )
    });
    let mut config_changed = false;
    for (index, control) in [
        OscillatorControl::Level,
        OscillatorControl::Transpose,
        OscillatorControl::Cents,
    ]
    .into_iter()
    .enumerate()
    {
        with_child(
            ui,
            cells[index],
            ("resynth-primary", slot.index(), index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                config_changed |= resynth_config_readout(
                    ui,
                    state,
                    module_id,
                    slot,
                    config,
                    control,
                    cells[index],
                );
            },
        );
    }
    let mut controls_changed = false;
    with_child(
        ui,
        cells[3],
        ("resynth-primary", slot.index(), 3),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            controls_changed |= grain_modular_percent_readout(
                ui,
                state,
                module_id,
                slot,
                "STEREO",
                "",
                &mut controls.grain_stereo,
                defaults.grain_stereo,
                OscillatorControl::GrainStereo,
                cells[3],
                true,
            );
        },
    );
    with_child(
        ui,
        cells[4],
        ("resynth-primary", slot.index(), 4),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            controls_changed |=
                grain_direction_readout(ui, slot, &mut controls, defaults, cells[4].size(), true);
        },
    );
    if controls_changed {
        source.apply_live_controls(controls);
    }
    config_changed || controls_changed
}

fn resynth_config_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut OscillatorConfig,
    control: OscillatorControl,
    cell: egui::Rect,
) -> bool {
    let before = *config;
    let defaults = OscillatorConfig::for_engine(config.engine);
    let (label, text, response, mut changed) = match control {
        OscillatorControl::Level => {
            let text = grain_percent_text(config.level);
            let (_, response, changed) = grain_scalar_drag(
                ui,
                &mut config.level,
                0.0..=1.0,
                0.01,
                defaults.level,
                "LEVEL",
                &text,
                cell.size(),
            );
            ("LEVEL", grain_percent_text(config.level), response, changed)
        }
        OscillatorControl::Transpose => {
            let text = format!("{:+.0}", config.transpose);
            let (_, response, changed) = grain_scalar_drag(
                ui,
                &mut config.transpose,
                -48.0..=48.0,
                0.125,
                defaults.transpose,
                "SEMI",
                &text,
                cell.size(),
            );
            (
                "SEMI",
                format!("{:+.0}", config.transpose),
                response,
                changed,
            )
        }
        OscillatorControl::Cents => {
            let text = format!("{:+.1}", config.cents);
            let (_, response, changed) = grain_scalar_drag(
                ui,
                &mut config.cents,
                -100.0..=100.0,
                0.5,
                defaults.cents,
                "CENT",
                &text,
                cell.size(),
            );
            ("CENT", format!("{:+.1}", config.cents), response, changed)
        }
        _ => unreachable!(),
    };
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
        *config = before;
        changed = false;
    }
    let normalized = control.normalized_value(*config);
    if let Some((_, param, _)) =
        crate::editor_modulation::host_automation_binding(ui, state, target)
    {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, &response, normalized, changed,
        );
        changed = false;
    }
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        &response,
        normalized,
        cell,
        crate::editor_modulation::TrackAxis::Horizontal,
        1.0,
    );
    paint_primary_metric_readout(
        ui,
        cell,
        label,
        &text,
        response.hovered(),
        response.is_pointer_button_down_on() || response.dragged(),
    );
    changed
}

fn draw_grain_controls_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    module_id: ModuleId,
    readouts: egui::Rect,
    source: &Arc<crate::resynth_state::ResynthSlotState>,
    mut controls: ResynthControls,
) -> bool {
    let defaults = ResynthControls::default();
    let mut changed = false;
    let cell_width = readouts.width() / 6.0;
    for (index, metric) in [
        GrainMetric::Density,
        GrainMetric::Size,
        GrainMetric::Pitch,
        GrainMetric::Tune,
        GrainMetric::Stereo,
        GrainMetric::Direction,
    ]
    .into_iter()
    .enumerate()
    {
        let cell = egui::Rect::from_min_size(
            egui::pos2(readouts.left() + index as f32 * cell_width, readouts.top()),
            egui::vec2(cell_width, readouts.height()),
        );
        with_child(
            ui,
            cell,
            ("grain-metric", slot.index(), index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                changed |= grain_metric_readout(
                    ui,
                    state,
                    module_id,
                    slot,
                    &mut controls,
                    defaults,
                    metric,
                    cell,
                );
            },
        );
    }
    if changed {
        source.apply_live_controls(controls);
    }
    changed
}

fn draw_rich_controls_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    module_id: ModuleId,
    readouts: egui::Rect,
    source: &Arc<crate::resynth_state::ResynthSlotState>,
    mut controls: ResynthControls,
    _build_pending: bool,
) -> bool {
    let defaults = ResynthControls::default();
    let mut changed = false;
    let cell_width = readouts.width() / 6.0;
    for index in 0..6 {
        let cell = egui::Rect::from_min_size(
            egui::pos2(readouts.left() + index as f32 * cell_width, readouts.top()),
            egui::vec2(cell_width, readouts.height()),
        );
        with_child(
            ui,
            cell,
            ("rich-metric", slot.index(), index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let metric_changed = match index {
                    0 => rich_scalar_readout(
                        ui,
                        state,
                        module_id,
                        slot,
                        Some(OscillatorControl::RichBalance),
                        cell,
                        "BAL",
                        &mut controls.rich_balance,
                        -1.0..=1.0,
                        0.01,
                        defaults.rich_balance,
                        |value| format!("{value:+.2}"),
                    ),
                    1 => rich_scalar_readout(
                        ui,
                        state,
                        module_id,
                        slot,
                        Some(OscillatorControl::RichFormant),
                        cell,
                        "FORMANT",
                        &mut controls.rich_formant_semitones,
                        -24.0..=24.0,
                        0.1,
                        defaults.rich_formant_semitones,
                        |value| format!("{value:+.1}"),
                    ),
                    2 => rich_scalar_readout(
                        ui,
                        state,
                        module_id,
                        slot,
                        Some(OscillatorControl::RichAir),
                        cell,
                        "AIR",
                        &mut controls.rich_air_db,
                        -12.0..=12.0,
                        0.1,
                        defaults.rich_air_db,
                        |value| format!("{value:+.1} dB"),
                    ),
                    3 => rich_scalar_readout(
                        ui,
                        state,
                        module_id,
                        slot,
                        Some(OscillatorControl::RichDiffuse),
                        cell,
                        "DIFFUSE",
                        &mut controls.rich_diffuse,
                        0.0..=1.0,
                        0.01,
                        defaults.rich_diffuse,
                        grain_percent_text,
                    ),
                    4 => rich_scalar_readout(
                        ui,
                        state,
                        module_id,
                        slot,
                        Some(OscillatorControl::GrainTune),
                        cell,
                        "TUNE",
                        &mut controls.grain_tune,
                        0.0..=1.0,
                        0.01,
                        defaults.grain_tune,
                        grain_percent_text,
                    ),
                    _ => rich_scalar_readout(
                        ui,
                        state,
                        module_id,
                        slot,
                        Some(OscillatorControl::RichDynamic),
                        cell,
                        "DYNAMIC",
                        &mut controls.rich_dynamic,
                        0.0..=1.0,
                        0.01,
                        defaults.rich_dynamic,
                        grain_percent_text,
                    ),
                };
                changed |= metric_changed;
            },
        );
    }
    if changed {
        source.apply_live_controls(controls);
    }
    changed
}

fn rich_scalar_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    modulation: Option<OscillatorControl>,
    cell: egui::Rect,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    format: impl Fn(f32) -> String,
) -> bool {
    let before = *value;
    let minimum = *range.start();
    let span = (*range.end() - minimum).max(f32::EPSILON);
    let text = format(*value);
    let (rect, response, changed) =
        grain_scalar_drag(ui, value, range, speed, default, label, &text, cell.size());
    paint_grain_metric_readout(
        ui,
        rect,
        label,
        &format(*value),
        response.hovered(),
        response.is_pointer_button_down_on() || response.dragged(),
    );
    let Some(control) = modulation else {
        return changed;
    };
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    let normalized = ((*value - minimum) / span).clamp(0.0, 1.0);
    if !control.supports_internal_modulation() {
        if let Some((_, param, _)) =
            crate::editor_modulation::host_automation_binding(ui, state, target)
        {
            crate::editor_modulation::update_host_automation_gesture(
                state, param, &response, normalized, changed,
            );
        }
        crate::editor_modulation::host_automation_destination(
            ui, state, target, &response, normalized,
        );
        return changed;
    }
    let owns = crate::editor_modulation::modular_owns_gesture(ui, state, target, &response);
    if owns {
        *value = before;
    }
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        &response,
        normalized,
        rect,
        crate::editor_modulation::TrackAxis::Horizontal,
        1.0,
    );
    changed && !owns
}

#[allow(dead_code)]
fn rich_seed_readout(
    ui: &mut egui::Ui,
    cell: egui::Rect,
    slot: OscillatorSlot,
    seed: &mut u64,
) -> bool {
    let response = ui
        .interact(
            cell,
            egui::Id::new(("rich-seed", slot.index())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Generate a new deterministic spectral texture");
    paint_grain_metric_readout(
        ui,
        cell,
        "SEED",
        egui_phosphor::regular::DICE_FIVE,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    if response.clicked() {
        *seed = randomized_seed(*seed);
        true
    } else {
        false
    }
}

#[derive(Clone, Copy)]
enum GrainMetric {
    Density,
    Size,
    Pitch,
    Tune,
    Stereo,
    Direction,
}

fn grain_metric_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    controls: &mut ResynthControls,
    defaults: ResynthControls,
    metric: GrainMetric,
    cell: egui::Rect,
) -> bool {
    let size = cell.size();
    match metric {
        GrainMetric::Direction => {
            grain_direction_readout(ui, slot, controls, defaults, size, false)
        }
        GrainMetric::Density => grain_paired_readout(
            ui,
            slot,
            "RATE",
            &mut controls.grain_density,
            1.0..=2_000.0,
            1.0,
            defaults.grain_density,
            grain_rate_text,
            &mut controls.grain_timing,
            0.0..=1.0,
            0.01,
            defaults.grain_timing,
            grain_spread_percent_text,
            cell,
        ),
        GrainMetric::Size => {
            let value_text = grain_size_text(controls.grain_size);
            let (rect, response, changed) = grain_scalar_drag(
                ui,
                &mut controls.grain_size,
                0.0..=1.0,
                0.01,
                defaults.grain_size,
                "LENGTH",
                &value_text,
                size,
            );
            paint_grain_metric_readout(
                ui,
                rect,
                "LENGTH",
                &grain_size_text(controls.grain_size),
                response.hovered(),
                response.is_pointer_button_down_on() || response.dragged(),
            );
            changed
        }
        GrainMetric::Pitch => grain_paired_readout(
            ui,
            slot,
            "PITCH",
            &mut controls.grain_pitch,
            -24.0..=24.0,
            0.1,
            defaults.grain_pitch,
            grain_pitch_text,
            &mut controls.grain_pitch_spread,
            0.0..=24.0,
            0.1,
            defaults.grain_pitch_spread,
            grain_pitch_spread_text,
            cell,
        ),
        GrainMetric::Tune => grain_modular_percent_readout(
            ui,
            state,
            module_id,
            slot,
            "TUNE",
            egui_phosphor::regular::MUSIC_NOTE,
            &mut controls.grain_tune,
            defaults.grain_tune,
            OscillatorControl::GrainTune,
            cell,
            false,
        ),
        GrainMetric::Stereo => grain_modular_percent_readout(
            ui,
            state,
            module_id,
            slot,
            "STEREO",
            "",
            &mut controls.grain_stereo,
            defaults.grain_stereo,
            OscillatorControl::GrainStereo,
            cell,
            false,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn grain_modular_percent_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    label: &str,
    icon: &str,
    value: &mut f32,
    default: f32,
    control: OscillatorControl,
    cell: egui::Rect,
    primary: bool,
) -> bool {
    let before = *value;
    let text = if icon.is_empty() {
        grain_percent_text(*value)
    } else {
        format!("{icon} {:.0}%", *value * 100.0)
    };
    let (rect, response, changed) = grain_scalar_drag(
        ui,
        value,
        0.0..=1.0,
        0.01,
        default,
        label,
        &text,
        cell.size(),
    );
    let text = if icon.is_empty() {
        grain_percent_text(*value)
    } else {
        format!("{icon} {:.0}%", *value * 100.0)
    };
    if primary {
        paint_primary_metric_readout(
            ui,
            rect,
            label,
            &text,
            response.hovered(),
            response.is_pointer_button_down_on() || response.dragged(),
        );
    } else {
        paint_grain_metric_readout(
            ui,
            rect,
            label,
            &text,
            response.hovered(),
            response.is_pointer_button_down_on() || response.dragged(),
        );
    }
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    let owns = crate::editor_modulation::modular_owns_gesture(ui, state, target, &response);
    if owns {
        *value = before;
    }
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        &response,
        *value,
        rect,
        crate::editor_modulation::TrackAxis::Horizontal,
        1.0,
    );
    changed && !owns
}

fn grain_pair_drag(
    ui: &mut egui::Ui,
    slot: OscillatorSlot,
    label: &str,
    side: u8,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    value_text: &str,
    rect: egui::Rect,
) -> (egui::Response, bool) {
    let _ = value_text;
    let response = ui
        .interact(
            rect,
            egui::Id::new(("grain-pair", slot.index(), label, side)),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Drag vertically to change. Hold Shift for fine control; double-click to reset.",
        );
    let changed =
        editor_controls::update_custom_value_drag(ui, &response, value, range, speed, default);
    (response, changed)
}

fn grain_paired_readout(
    ui: &mut egui::Ui,
    slot: OscillatorSlot,
    label: &str,
    value: &mut f32,
    value_range: std::ops::RangeInclusive<f32>,
    value_speed: f32,
    value_default: f32,
    value_format: fn(f32) -> String,
    random: &mut f32,
    random_range: std::ops::RangeInclusive<f32>,
    random_speed: f32,
    random_default: f32,
    random_format: fn(f32) -> String,
    cell: egui::Rect,
) -> bool {
    let mut value_text = value_format(*value);
    let mut random_text = random_format(*random);
    let left = egui::Rect::from_min_max(cell.min, egui::pos2(cell.center().x, cell.bottom()));
    let right = egui::Rect::from_min_max(egui::pos2(cell.center().x, cell.top()), cell.max);
    let (value_response, value_changed) = grain_pair_drag(
        ui,
        slot,
        label,
        0,
        value,
        value_range,
        value_speed,
        value_default,
        &value_text,
        left,
    );
    let (random_response, random_changed) = grain_pair_drag(
        ui,
        slot,
        label,
        1,
        random,
        random_range,
        random_speed,
        random_default,
        &random_text,
        right,
    );
    if value_changed {
        value_text = value_format(*value);
    }
    if random_changed {
        random_text = random_format(*random);
    }
    paint_grain_paired_readout(
        ui,
        cell,
        label,
        &value_text,
        &random_text,
        value_response.hovered(),
        random_response.hovered(),
        value_response.is_pointer_button_down_on() || value_response.dragged(),
        random_response.is_pointer_button_down_on() || random_response.dragged(),
    );
    value_changed || random_changed
}

fn grain_rate_text(value: f32) -> String {
    format!("{value:.0} Hz")
}

fn grain_size_text(value: f32) -> String {
    format!("{:.0} ms", 5.0 * 200.0_f32.powf(value.clamp(0.0, 1.0)))
}

fn grain_percent_text(value: f32) -> String {
    format!("{:.0} %", value * 100.0)
}

fn grain_spread_percent_text(value: f32) -> String {
    format!("±{:.0} %", value * 100.0)
}

fn grain_pitch_text(value: f32) -> String {
    format!("{value:+.1}")
}

fn grain_pitch_spread_text(value: f32) -> String {
    format!("±{:.1}¢", value * 100.0)
}

fn grain_pan_text(value: f32) -> String {
    if value.abs() < 0.005 {
        "C".to_owned()
    } else {
        format!(
            "{} {:.0}",
            if value < 0.0 { "L" } else { "R" },
            value.abs() * 100.0
        )
    }
}

fn grain_play_label(direction: GrainDirection) -> &'static str {
    match direction {
        GrainDirection::Backward => "BACK",
        GrainDirection::PingPong => "PONG",
        GrainDirection::Hold | GrainDirection::Forward => "FWD",
    }
}

fn grain_direction_readout(
    ui: &mut egui::Ui,
    slot: OscillatorSlot,
    controls: &mut ResynthControls,
    _defaults: ResynthControls,
    size: egui::Vec2,
    primary: bool,
) -> bool {
    let label = "DIR";
    let minimum = editor_theme::font::VALUE_SIZE + editor_theme::font::CAPTION_SIZE;
    let (id, rect) = ui.allocate_space(egui::vec2(size.x.max(minimum), size.y.max(minimum)));
    let direction_response = ui
        .interact(
            rect,
            id.with(("grain-direction", slot.index())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to cycle FWD, BACK, and PONG. Double-click resets to FWD.");
    let mut changed = false;
    if direction_response.double_clicked() {
        controls.grain_direction = GrainDirection::Forward as u8;
        changed = true;
    } else if direction_response.clicked() {
        controls.grain_direction = match controls.grain_direction() {
            GrainDirection::Forward | GrainDirection::Hold => GrainDirection::Backward as u8,
            GrainDirection::Backward => GrainDirection::PingPong as u8,
            GrainDirection::PingPong => GrainDirection::Forward as u8,
        };
        changed = true;
    }
    let value = grain_play_label(controls.grain_direction());
    if primary {
        paint_primary_metric_readout(
            ui,
            rect,
            label,
            value,
            direction_response.hovered(),
            direction_response.is_pointer_button_down_on(),
        );
    } else {
        paint_grain_metric_readout(
            ui,
            rect,
            label,
            value,
            direction_response.hovered(),
            direction_response.is_pointer_button_down_on(),
        );
    }
    changed
}

fn grain_scalar_drag(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    label: &str,
    value_text: &str,
    size: egui::Vec2,
) -> (egui::Rect, egui::Response, bool) {
    let minimum = editor_theme::font::VALUE_SIZE + editor_theme::font::CAPTION_SIZE;
    let (id, rect) = ui.allocate_space(egui::vec2(size.x.max(minimum), size.y.max(minimum)));
    let interaction = editor_controls::metric_text_bounds(ui, rect, label, value_text);
    let response = ui.interact(
        interaction,
        id.with("metric-value"),
        egui::Sense::click_and_drag(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Drag vertically to change. Hold Shift for fine control; double-click to reset.",
        );
    let changed =
        editor_controls::update_custom_value_drag(ui, &response, value, range, speed, default);
    (rect, response, changed)
}

fn paint_grain_metric_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    hovered: bool,
    active: bool,
) {
    paint_metric_readout(
        ui,
        rect,
        label,
        value,
        hovered,
        active,
        editor_theme::semantic().unison,
    );
}

fn paint_primary_metric_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    hovered: bool,
    active: bool,
) {
    paint_metric_readout(
        ui,
        rect,
        label,
        value,
        hovered,
        active,
        editor_theme::semantic().primary,
    );
}

fn paint_metric_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    hovered: bool,
    active: bool,
    accent: egui::Color32,
) {
    let painter = ui.painter_at(rect);
    let layout = editor_controls::layout_metric_text(ui, &painter, rect, label, value);
    let (label_color, value_color) = if !ui.is_enabled() {
        let disabled = editor_theme::semantic().disabled_text;
        (disabled, disabled)
    } else if active {
        (accent, accent)
    } else if hovered {
        (accent.gamma_multiply(0.88), accent)
    } else {
        (accent.gamma_multiply(0.64), accent.gamma_multiply(0.88))
    };
    painter.galley(layout.label_position, layout.label, label_color);
    painter.galley(layout.value_position, layout.value, value_color);
}

fn paint_grain_paired_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    random: &str,
    value_hovered: bool,
    random_hovered: bool,
    value_active: bool,
    random_active: bool,
) {
    let painter = ui.painter_at(rect);
    let combined = format!("{value} {random}");
    let layout = editor_controls::layout_metric_text(ui, &painter, rect, label, &combined);
    let accent = editor_theme::semantic().unison;
    let value_width = painter
        .layout_no_wrap(
            value.to_owned(),
            layout.value_font.clone(),
            egui::Color32::WHITE,
        )
        .size()
        .x;
    let separator = painter
        .layout_no_wrap(" ".into(), layout.value_font.clone(), egui::Color32::WHITE)
        .size()
        .x;
    let color = |hovered: bool, active: bool| {
        if !ui.is_enabled() {
            editor_theme::semantic().disabled_text
        } else if active {
            accent
        } else if hovered {
            accent.gamma_multiply(0.88)
        } else {
            accent.gamma_multiply(0.64)
        }
    };
    let label_color = color(
        value_hovered || random_hovered,
        value_active || random_active,
    );
    let value_color = if !ui.is_enabled() {
        editor_theme::semantic().disabled_text
    } else if value_active {
        accent
    } else if value_hovered {
        accent
    } else {
        accent.gamma_multiply(0.88)
    };
    let random_color = if !ui.is_enabled() {
        editor_theme::semantic().disabled_text
    } else if random_active {
        accent
    } else if random_hovered {
        accent
    } else {
        accent.gamma_multiply(0.88)
    };
    painter.galley(layout.label_position, layout.label, label_color);
    painter.text(
        egui::pos2(layout.value_position.x, layout.value_position.y),
        egui::Align2::LEFT_TOP,
        value,
        layout.value_font.clone(),
        value_color,
    );
    painter.text(
        egui::pos2(
            layout.value_position.x + value_width + separator,
            layout.value_position.y,
        ),
        egui::Align2::LEFT_TOP,
        random,
        layout.value_font,
        random_color,
    );
}

fn randomized_seed(previous: u64) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let mut value = previous ^ time ^ COUNTER.fetch_add(1, Ordering::Relaxed);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
fn format_pitch(hz: f32) -> String {
    if !hz.is_finite() || hz <= 0.0 {
        return "UNKNOWN".to_owned();
    }
    const NOTE_NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let nearest = midi.round() as i32;
    let note = NOTE_NAMES[nearest.rem_euclid(12) as usize];
    let octave = nearest.div_euclid(12) - 1;
    let cents = (midi - nearest as f32) * 100.0;
    format!("{note}{octave} · {hz:.2} Hz · {cents:+.1}¢")
}

fn algorithm_accent(algorithm: ResynthAlgorithm) -> egui::Color32 {
    let palette = editor_theme::semantic();
    match algorithm {
        ResynthAlgorithm::Sample => palette.primary,
        ResynthAlgorithm::Grain => palette.unison,
        ResynthAlgorithm::Rich => palette.unison,
    }
}

fn set_status(ui: &egui::Ui, module: ModuleId, message: String) {
    ui.data_mut(|data| data.insert_temp(egui::Id::new(("resynth-status", module.get())), message));
}
fn paint_status(ui: &egui::Ui, module: ModuleId, rect: egui::Rect) {
    if let Some(message) =
        ui.data(|data| data.get_temp::<String>(egui::Id::new(("resynth-status", module.get()))))
    {
        ui.painter().text(
            rect.right_bottom() - egui::vec2(editor_theme::space::SM, editor_theme::space::SM),
            egui::Align2::RIGHT_BOTTOM,
            message,
            editor_theme::font::caption(),
            editor_theme::semantic().text_muted,
        );
    }
}
fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let head = value
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>();
    format!("{head}…")
}
