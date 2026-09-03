use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use truce_core::editor::PluginContext;

use super::{DEFAULT_ROOT_MIDI, POLL, paint_status, set_status};
use crate::{
    KurvParams,
    editor::{DetachedJob, ImportSource, spawn_detached_job},
    editor_theme,
    generators::{ModuleId, OscillatorSlot},
    oscillators::{
        AUDIO_IMPORT_EXTENSIONS, MAX_RESYNTH_SOURCE_BYTES, ResynthAlgorithm, ResynthAnalysisModel,
        ResynthControls, analyze_wav_with_cancel, is_supported_audio_import_name,
    },
};

static IMPORT_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
type ImportResult = Result<ResynthAnalysisModel, String>;
type PendingImport = DetachedJob<ResynthAnalysisModel>;

/// Process-wide single-analysis lease. One maximum legal WAV can expand far
/// beyond its container size while decoding, so cards and plugin instances
/// share this conservative bound until the worker releases all PCM buffers.
pub(super) struct ImportWorkerPermit;

impl ImportWorkerPermit {
    pub(super) fn try_acquire() -> Result<Self, String> {
        IMPORT_WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "Another RESYNTH Source is already being analyzed".to_owned())
    }
}

impl Drop for ImportWorkerPermit {
    fn drop(&mut self) {
        IMPORT_WORKER_ACTIVE.store(false, Ordering::Release);
    }
}

struct ImportJob {
    source: ImportSource,
    name: String,
    controls: ResynthControls,
}

pub(super) fn handle_import(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    browse: &egui::Response,
    build_pending: bool,
    slot: OscillatorSlot,
    module_id: ModuleId,
    controls: ResynthControls,
) {
    let pending_id = egui::Id::new(("resynth-import", module_id.get(), slot.index()));
    if let Some(pending) = ui.data(|data| data.get_temp::<Arc<PendingImport>>(pending_id)) {
        if let Some(result) = pending.poll() {
            ui.data_mut(|data| data.remove::<Arc<PendingImport>>(pending_id));
            match result {
                Ok(model) => {
                    let module_still_matches = state
                        .generator_stack
                        .snapshot()
                        .groups()
                        .iter()
                        .flat_map(crate::generators::Group::modules)
                        .any(|module| {
                            module.id() == module_id && module.oscillator_slot() == Some(slot)
                        });
                    if module_still_matches {
                        let source_bytes = model.source.original_bytes.len();
                        if !state
                            .resynth_assets
                            .can_replace_source(slot.index(), source_bytes)
                        {
                            set_status(
                                ui,
                                module_id,
                                "Source budget exceeded (20 MiB across RESYNTH slots)".to_owned(),
                            );
                        } else {
                            let mut model = model;
                            if model.effective_root_hz().is_none() {
                                model.root_override_hz =
                                    Some(crate::dsp::midi_note_hz(DEFAULT_ROOT_MIDI as f32));
                            }
                            if let Some(revision) = state.resynth_assets.request_import(
                                slot.index(),
                                model,
                                ResynthAlgorithm::Grain,
                                controls,
                            ) {
                                crate::editor::notify_persisted_state_changed(state);
                                set_status(ui, module_id, format!("Building Grain r{revision}"));
                            } else {
                                set_status(ui, module_id, "Artifact worker unavailable".to_owned());
                            }
                        }
                    }
                }
                Err(error) => set_status(ui, module_id, error),
            }
        } else {
            ui.ctx().request_repaint_after(POLL);
        }
    }
    if browse.clicked() && !build_pending {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio", AUDIO_IMPORT_EXTENSIONS)
            .set_title("Load RESYNTH source")
            .pick_file()
        {
            let name = path.file_name().map_or_else(
                || "source.wav".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            let job = ImportJob {
                source: ImportSource::Path(path),
                name,
                controls,
            };
            match start_import(job) {
                Ok(pending) => {
                    ui.data_mut(|data| data.insert_temp(pending_id, pending));
                    set_status(ui, module_id, "Importing and analyzing…".to_owned());
                    ui.ctx().request_repaint_after(POLL);
                }
                Err(error) => set_status(ui, module_id, error),
            }
        }
    }
    let pointer_pos = ui
        .ctx()
        .pointer_hover_pos()
        .or_else(|| ui.ctx().pointer_latest_pos());
    let pointer_over = pointer_pos.is_some_and(|position| response.rect.contains(position))
        || response.contains_pointer()
        || response.hovered();
    let hover_id = egui::Id::new(("resynth-drop-armed", module_id.get()));
    let claimed_id = egui::Id::new("resynth-drop-claimed");
    let hovering_files = ui.input(|input| !input.raw.hovered_files.is_empty());
    if hovering_files && pointer_over {
        ui.data_mut(|data| data.insert_temp(hover_id, true));
        ui.painter().rect_stroke(
            response.rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(2.0_f32, editor_theme::semantic().primary),
            egui::StrokeKind::Inside,
        );
    }
    let files = ui.input(|input| input.raw.dropped_files.clone());
    let armed = ui
        .data(|data| data.get_temp::<bool>(hover_id))
        .unwrap_or(false);
    if files.is_empty() && !hovering_files {
        ui.data_mut(|data| {
            data.remove::<bool>(hover_id);
            data.remove::<bool>(claimed_id);
        });
    }
    let already_claimed = ui
        .data(|data| data.get_temp::<bool>(claimed_id))
        .unwrap_or(false);
    let plugin_drop = !files.is_empty()
        && !already_claimed
        && pointer_pos.is_some_and(|pos| ui.ctx().content_rect().contains(pos))
        && !armed
        && !pointer_over;
    let drop_hits = !already_claimed && (pointer_over || armed || plugin_drop);
    if build_pending && drop_hits && !files.is_empty() {
        set_status(
            ui,
            module_id,
            "Wait for the current build to finish".to_owned(),
        );
        paint_status(ui, module_id, response.rect);
        return;
    }
    if !drop_hits || files.is_empty() {
        paint_status(ui, module_id, response.rect);
        return;
    }
    if files.len() != 1 {
        set_status(ui, module_id, "Drop one audio file".to_owned());
        return;
    }
    match import_job(&files[0], controls).and_then(start_import) {
        Ok(pending) => {
            ui.data_mut(|data| {
                data.insert_temp(pending_id, pending);
                data.insert_temp(claimed_id, true);
                data.remove::<bool>(hover_id);
            });
            set_status(ui, module_id, "Importing and analyzing…".to_owned());
            ui.ctx().request_repaint_after(POLL);
        }
        Err(error) => set_status(ui, module_id, error),
    }
    paint_status(ui, module_id, response.rect);
}

fn import_job(file: &egui::DroppedFile, controls: ResynthControls) -> Result<ImportJob, String> {
    let name = file
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .map_or_else(
            || {
                if file.name.is_empty() {
                    "source.wav".to_owned()
                } else {
                    file.name.clone()
                }
            },
            |name| name.to_string_lossy().into_owned(),
        );
    if !is_supported_audio_import_name(&name) {
        return Err("Drop WAV, FLAC, AIFF, OGG, or MP3".to_owned());
    }
    let source = if let Some(bytes) = &file.bytes {
        if bytes.len() > MAX_RESYNTH_SOURCE_BYTES {
            return Err("WAV exceeds the 16 MiB source limit".to_owned());
        }
        ImportSource::Bytes(Arc::clone(bytes))
    } else if let Some(path) = &file.path {
        ImportSource::Path(path.clone())
    } else {
        return Err("Host supplied neither bytes nor a readable path".to_owned());
    };
    Ok(ImportJob {
        source,
        name,
        controls,
    })
}

fn start_import(job: ImportJob) -> Result<Arc<PendingImport>, String> {
    let permit = ImportWorkerPermit::try_acquire()?;
    spawn_detached_job("kurv-resynth-import", move |cancelled| {
        let _permit = permit;
        run_import(job, cancelled)
    })
}

fn run_import(job: ImportJob, cancelled: &AtomicBool) -> ImportResult {
    // Reserve decoded PCM/scratch capacity before Source bytes are copied or
    // read. State rebuilds share this exact process-wide lease.
    let _analysis_work = crate::resynth_state::acquire_resynth_analysis_work();
    if cancelled.load(Ordering::Acquire) {
        return Err("Import cancelled".to_owned());
    }
    let bytes = job
        .source
        .read_bounded(&job.name, MAX_RESYNTH_SOURCE_BYTES, cancelled)?;
    if cancelled.load(Ordering::Acquire) {
        return Err("Import cancelled".to_owned());
    }
    let result = analyze_wav_with_cancel(job.name, bytes, job.controls, || {
        cancelled.load(Ordering::Acquire)
    })
    .map_err(|error| format!("WAV analysis failed: {error:?}"));
    if cancelled.load(Ordering::Acquire) {
        Err("Import cancelled".to_owned())
    } else {
        result
    }
}

#[cfg(test)]
pub(super) struct ExportTempGuard {
    pub(super) path: std::path::PathBuf,
}

#[cfg(test)]
impl Drop for ExportTempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
pub(super) fn create_unique_export_temp(
    destination: &std::path::Path,
) -> Result<(ExportTempGuard, std::fs::File), ()> {
    let (path, file) = crate::editor_presets::create_atomic_temp(destination).map_err(|_| ())?;
    Ok((ExportTempGuard { path }, file))
}
