//! Teardown-safe drag/drop loading and transient import feedback.

#[cfg(test)]
use std::sync::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use truce_core::custom_state::State;

use crate::editor::{ImportSource, detached_work_is_safe, spawn_detached_job_after_pin};
use crate::editor_presets::{atomic_write, user_data_directory};
use crate::editor_theme;
use crate::oscillators::{
    ImportedVaTable, MAX_WAVETABLE_FILE_BYTES, VaTableData, encode_surge_wt, parse_surge_wt,
};

const IMPORT_POLL_INTERVAL: Duration = Duration::from_millis(40);

type ImportResult = Result<ImportedVaTable, String>;

#[derive(Clone)]
struct ImportStatus {
    message: String,
    error: bool,
    expires_at: f64,
}

struct ImportJob {
    source: ImportSource,
    name: String,
}

type PendingImport = crate::editor::DetachedJob<ImportedVaTable>;

pub(super) fn wavetable_directory() -> Result<std::path::PathBuf, String> {
    user_data_directory()
        .map(|root| root.join("Wavetables"))
        .map_err(|error| error.to_string())
}

pub(super) fn list_surge_tables() -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let directory = wavetable_directory()?;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut tables = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let is_wt = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wt"));
        if !is_wt || !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        tables.push((name, path));
    }
    tables.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(tables)
}

pub(super) fn sanitize_table_name(name: &str) -> String {
    let mut sanitized = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ' ') {
            sanitized.push(character);
        }
    }
    let sanitized = sanitized.trim();
    if sanitized.is_empty() {
        "table".to_owned()
    } else {
        sanitized.to_owned()
    }
}

pub(super) fn save_surge_table(
    name: &str,
    table: &VaTableData,
) -> Result<std::path::PathBuf, String> {
    if table.frames.is_empty() {
        return Err("nothing to save: the VA table is empty".to_owned());
    }
    let directory = wavetable_directory()?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("{}.wt", sanitize_table_name(name)));
    let bytes = encode_surge_wt(table).map_err(|error| error.to_string())?;
    atomic_write(&path, &bytes).map_err(|error| error.to_string())?;
    Ok(path)
}

const NATIVE_VA_MAGIC_V1: &[u8; 8] = b"KURVVA01";
const NATIVE_VA_MAGIC_V2: &[u8; 8] = b"KURVVA02";
const NATIVE_VA_EXTENSION: &str = "kurv-va";

pub(super) fn list_native_tables() -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let directory = wavetable_directory()?;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut tables = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_native = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(NATIVE_VA_EXTENSION));
        if is_native && entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            tables.push((name, path));
        }
    }
    tables.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(tables)
}

pub(super) fn save_native_table(
    name: &str,
    table: &VaTableData,
) -> Result<std::path::PathBuf, String> {
    if table.frames.is_empty() {
        return Err("nothing to save: the VA table is empty".to_owned());
    }
    let directory = wavetable_directory()?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!(
        "{}.{}",
        sanitize_table_name(name),
        NATIVE_VA_EXTENSION
    ));
    if path.exists() {
        return Err(format!(
            "{} already exists; choose another VA table name",
            path.display()
        ));
    }
    let table = table.clone().sanitized();
    let payload = table.serialize();
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| "VA table is too large to save".to_owned())?;
    let magic = if table.is_positioned() {
        NATIVE_VA_MAGIC_V2
    } else {
        NATIVE_VA_MAGIC_V1
    };
    let mut bytes = Vec::with_capacity(magic.len() + 4 + payload.len());
    bytes.extend_from_slice(magic);
    bytes.extend_from_slice(&payload_len.to_le_bytes());
    bytes.extend_from_slice(&payload);
    atomic_write(&path, &bytes).map_err(|error| error.to_string())?;
    Ok(path)
}

fn parse_native_table(bytes: &[u8]) -> Result<ImportedVaTable, String> {
    if bytes.len() < NATIVE_VA_MAGIC_V1.len() + 4 {
        return Err("invalid KURV VA table header".to_owned());
    }
    let magic = &bytes[..NATIVE_VA_MAGIC_V1.len()];
    let positioned_format = if magic == NATIVE_VA_MAGIC_V2 {
        true
    } else if magic == NATIVE_VA_MAGIC_V1 {
        false
    } else {
        return Err("invalid KURV VA table header".to_owned());
    };
    let length_start = NATIVE_VA_MAGIC_V1.len();
    let length = u32::from_le_bytes(
        bytes[length_start..length_start + 4]
            .try_into()
            .map_err(|_| "invalid KURV VA table length".to_owned())?,
    ) as usize;
    let payload = &bytes[length_start + 4..];
    if payload.len() != length {
        return Err("truncated or trailing KURV VA table data".to_owned());
    }
    let table =
        VaTableData::deserialize(payload).ok_or_else(|| "invalid KURV VA table data".to_owned())?;
    if table.frames.is_empty() {
        return Err("KURV VA table contains no frames".to_owned());
    }
    if table.frames.len() > crate::oscillators::MAX_VA_TABLE_FRAMES {
        return Err(format!(
            "VA table contains more than {} frames",
            crate::oscillators::MAX_VA_TABLE_FRAMES
        ));
    }
    if positioned_format != table.is_positioned() {
        return Err("KURV VA table layout does not match its format version".to_owned());
    }
    if table.clone().sanitized() != table {
        return Err("KURV VA table contains invalid spline data or positions".to_owned());
    }
    let source_frame_count = table.frames.len();
    Ok(ImportedVaTable {
        table,
        source_frame_count,
    })
}

pub(super) fn queue_library_import(
    ui: &egui::Ui,
    response_id: egui::Id,
    oscillator: usize,
    path: std::path::PathBuf,
) -> Option<String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match start_import(ImportJob {
        source: ImportSource::Path(path),
        name: name.clone(),
    }) {
        Ok(pending) => {
            ui.data_mut(|store| {
                store.insert_temp(pending_import_id(response_id, oscillator), pending);
            });
            set_import_status(
                ui,
                response_id,
                oscillator,
                format!("Importing {name}…"),
                false,
            );
            ui.ctx().request_repaint_after(IMPORT_POLL_INTERVAL);
            None
        }
        Err(error) => Some(error),
    }
}

pub(super) fn handle_wavetable_drop(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    plot: egui::Rect,
    oscillator: usize,
) -> Option<ImportResult> {
    let pointer_over = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pointer| response.rect.contains(pointer));
    let hovering_files = ui.input(|input| !input.raw.hovered_files.is_empty());
    if pointer_over && hovering_files {
        let palette = editor_theme::semantic();
        painter.rect_filled(
            plot,
            editor_theme::shape::CONTROL_RADIUS,
            palette.surface.gamma_multiply(0.88),
        );
        painter.rect_stroke(
            plot,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(2.0_f32, palette.primary),
            egui::StrokeKind::Inside,
        );
        painter.text(
            plot.center(),
            egui::Align2::CENTER_CENTER,
            "DROP VA TABLE / SURGE .WT",
            editor_theme::font::label(),
            palette.primary,
        );
    }

    let pending_id = pending_import_id(response.id, oscillator);
    if let Some(pending) = ui.data(|store| store.get_temp::<Arc<PendingImport>>(pending_id)) {
        if let Some(result) = pending.poll() {
            ui.data_mut(|store| store.remove::<Arc<PendingImport>>(pending_id));
            if let Err(error) = &result {
                set_import_status(ui, response.id, oscillator, error.clone(), true);
            }
            return Some(result);
        }
        // Polling is the only completion mechanism. A worker never invokes an
        // egui or host callback, so callback revocation on close remains final.
        ui.ctx().request_repaint_after(IMPORT_POLL_INTERVAL);
        return None;
    }

    let dropped = ui.input(|input| input.raw.dropped_files.clone());
    if dropped.is_empty() || !pointer_over {
        return None;
    }
    if dropped.len() != 1 {
        let error = "Drop exactly one VA table or Surge .wt file at a time".to_owned();
        set_import_status(ui, response.id, oscillator, error.clone(), true);
        return Some(Err(error));
    }

    let job = match ImportJob::from_dropped(&dropped[0]) {
        Ok(job) => job,
        Err(error) => {
            set_import_status(ui, response.id, oscillator, error.clone(), true);
            return Some(Err(error));
        }
    };
    let name = job.name.clone();
    match start_import(job) {
        Ok(pending) => {
            ui.data_mut(|store| store.insert_temp(pending_id, pending));
            set_import_status(
                ui,
                response.id,
                oscillator,
                format!("Importing {name}…"),
                false,
            );
            ui.ctx().request_repaint_after(IMPORT_POLL_INTERVAL);
            None
        }
        Err(error) => {
            set_import_status(ui, response.id, oscillator, error.clone(), true);
            Some(Err(error))
        }
    }
}

impl ImportJob {
    fn from_dropped(file: &egui::DroppedFile) -> Result<Self, String> {
        let name = dropped_file_name(file);
        let source = if let Some(bytes) = &file.bytes {
            if bytes.len() > MAX_WAVETABLE_FILE_BYTES {
                return Err(format!(
                    "{name}: file is too large ({} bytes; limit is {MAX_WAVETABLE_FILE_BYTES})",
                    bytes.len()
                ));
            }
            ImportSource::Bytes(Arc::clone(bytes))
        } else if let Some(path) = &file.path {
            ImportSource::Path(path.clone())
        } else {
            return Err(format!(
                "{name}: the host supplied neither a readable path nor file bytes"
            ));
        };
        Ok(Self { source, name })
    }
}

fn start_import(job: ImportJob) -> Result<Arc<PendingImport>, String> {
    start_import_after_pin(job, detached_work_is_safe())
}

fn start_import_after_pin(
    job: ImportJob,
    safe_to_detach: bool,
) -> Result<Arc<PendingImport>, String> {
    // A worker may remain blocked in remote/removable-media I/O after editor
    // destruction. Never spawn unless its containing image is pinned first.
    spawn_detached_job_after_pin("kurv-wt-import", safe_to_detach, move |cancelled| {
        run_import(job, cancelled)
    })
}

fn run_import(job: ImportJob, cancelled: &AtomicBool) -> ImportResult {
    let bytes = job
        .source
        .read_bounded(&job.name, MAX_WAVETABLE_FILE_BYTES, cancelled)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(format!("{}: import cancelled", job.name));
    }
    if job.name.to_ascii_lowercase().ends_with(".kurv-va") {
        parse_native_table(&bytes).map_err(|error| format!("{}: {error}", job.name))
    } else {
        parse_surge_wt(&bytes).map_err(|error| format!("{}: {error}", job.name))
    }
}

fn dropped_file_name(file: &egui::DroppedFile) -> String {
    if !file.name.is_empty() {
        return file.name.clone();
    }
    file.path.as_ref().map_or_else(
        || "dropped file".to_owned(),
        |path| {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        },
    )
}

fn pending_import_id(response: egui::Id, oscillator: usize) -> egui::Id {
    response.with(("wavetable-pending-import", oscillator))
}

fn import_status_id(response: egui::Id, oscillator: usize) -> egui::Id {
    response.with(("wavetable-import-status", oscillator))
}

pub(super) fn set_import_status(
    ui: &egui::Ui,
    response: egui::Id,
    oscillator: usize,
    message: String,
    error: bool,
) {
    let expires_at = ui.input(|input| input.time) + 7.0;
    ui.data_mut(|store| {
        store.insert_temp(
            import_status_id(response, oscillator),
            ImportStatus {
                message,
                error,
                expires_at,
            },
        );
    });
}

pub(super) fn paint_import_status(
    ui: &egui::Ui,
    painter: &egui::Painter,
    plot: egui::Rect,
    response: egui::Id,
    oscillator: usize,
) {
    let id = import_status_id(response, oscillator);
    let now = ui.input(|input| input.time);
    let Some(status) = ui.data(|store| store.get_temp::<ImportStatus>(id)) else {
        return;
    };
    if now >= status.expires_at {
        ui.data_mut(|store| store.remove::<ImportStatus>(id));
        return;
    }
    let palette = editor_theme::semantic();
    let color = if status.error {
        palette.danger
    } else {
        palette.primary
    };
    let status_rect = egui::Rect::from_min_max(
        egui::pos2(
            plot.left(),
            (plot.bottom() - editor_theme::space::LG).max(plot.top()),
        ),
        plot.right_bottom(),
    );
    painter.rect_filled(
        status_rect,
        editor_theme::shape::CONTROL_RADIUS,
        palette.surface.gamma_multiply(0.94),
    );
    painter.text(
        status_rect.center(),
        egui::Align2::CENTER_CENTER,
        status.message,
        editor_theme::font::caption(),
        color,
    );
    ui.ctx().request_repaint_after(Duration::from_millis(100));
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    fn float_fixture() -> Arc<[u8]> {
        let samples = [0.0_f32, 1.0, 0.0, -1.0];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"vawt");
        bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes.into()
    }

    #[test]
    fn dropped_bytes_are_only_parsed_by_background_worker() {
        let pending = start_import_after_pin(
            ImportJob {
                source: ImportSource::Bytes(float_fixture()),
                name: "fixture.wt".to_owned(),
            },
            true,
        )
        .expect("worker starts");
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = pending.poll() {
                break result;
            }
            assert!(Instant::now() < deadline, "worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
        .expect("fixture imports");

        assert_eq!(result.source_frame_count, 1);
    }

    #[test]
    fn worker_is_not_started_when_image_cannot_be_pinned() {
        let result = start_import_after_pin(
            ImportJob {
                source: ImportSource::Bytes(float_fixture()),
                name: "fixture.wt".to_owned(),
            },
            false,
        );

        assert!(matches!(result, Err(error) if error.contains("Cannot safely start")));
    }

    #[test]
    fn pending_poll_never_waits_for_worker_completion() {
        let (sender, receiver) = mpsc::channel::<ImportResult>();
        let pending = PendingImport {
            result: Mutex::new(receiver),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let before = Instant::now();

        assert!(pending.poll().is_none());
        assert!(before.elapsed() < Duration::from_millis(20));
        drop(sender);
    }

    #[test]
    fn dropping_pending_import_revokes_result_without_joining() {
        let (sender, receiver) = mpsc::channel::<ImportResult>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let observe_cancelled = Arc::clone(&cancelled);
        let pending = PendingImport {
            result: Mutex::new(receiver),
            cancelled,
        };

        drop(pending);

        assert!(observe_cancelled.load(Ordering::Acquire));
        assert!(sender.send(Err("late".to_owned())).is_err());
    }

    #[derive(Default, truce::State)]
    struct HistoricalNativeVaTableV1 {
        frames: Vec<crate::wave_curve::WaveCurveData>,
    }

    #[test]
    fn historical_native_v1_payload_without_positions_stays_legacy() {
        let frame = crate::wave_curve::WaveCurveData::default();
        let payload = HistoricalNativeVaTableV1 {
            frames: vec![frame.clone()],
        }
        .serialize();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(NATIVE_VA_MAGIC_V1);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        let restored = parse_native_table(&bytes).expect("historical V1 loads");

        assert_eq!(restored.table.frames, vec![frame]);
        assert!(restored.table.positions.is_empty());
    }

    #[test]
    fn native_va_table_round_trip_preserves_spline_points_exactly() {
        let mut table = VaTableData {
            frames: vec![crate::wave_curve::WaveCurveData::default()],
            positions: Vec::new(),
        };
        table.frames[0].knots[1].curve = 0.37;
        table.frames[0].knots[1].curve_x = -0.22;
        let payload = table.serialize();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(NATIVE_VA_MAGIC_V1);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        let restored = parse_native_table(&bytes).expect("native VA table loads");

        assert_eq!(restored.table, table);
    }

    #[test]
    fn native_va_table_rejects_nonfinite_spline_data() {
        let mut table = VaTableData {
            frames: vec![crate::wave_curve::WaveCurveData::default()],
            positions: Vec::new(),
        };
        table.frames[0].knots[0].value = f32::NAN;
        let payload = table.serialize();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(NATIVE_VA_MAGIC_V1);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        assert!(matches!(
            parse_native_table(&bytes),
            Err(error) if error.contains("invalid spline data")
        ));
    }

    #[test]
    fn native_va_table_rejects_trailing_bytes() {
        let table = VaTableData {
            frames: vec![crate::wave_curve::WaveCurveData::default()],
            positions: Vec::new(),
        };
        let payload = table.serialize();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(NATIVE_VA_MAGIC_V1);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes.push(0);
        assert!(parse_native_table(&bytes).is_err());
    }

    #[test]
    fn positioned_native_v2_round_trip_preserves_positions_exactly() {
        let mut first = crate::wave_curve::WaveCurveData::default();
        first.knots[0].value = 0.21;
        let mut second = crate::wave_curve::WaveCurveData::default();
        second.knots[0].value = 0.79;
        let table = VaTableData {
            frames: vec![first, second],
            positions: vec![0.45, 0.81],
        };
        let payload = table.serialize();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(NATIVE_VA_MAGIC_V2);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        let restored = parse_native_table(&bytes).expect("positioned native VA table loads");

        assert_eq!(restored.table, table);
    }

    #[test]
    fn positioned_native_v2_rejects_factory_collision_and_unsorted_positions() {
        for positions in [vec![1.0 / 3.0], vec![0.7, 0.4]] {
            let table = VaTableData {
                frames: vec![crate::wave_curve::WaveCurveData::default(); positions.len()],
                positions,
            };
            let payload = table.serialize();
            let mut bytes = Vec::new();
            bytes.extend_from_slice(NATIVE_VA_MAGIC_V2);
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&payload);
            assert!(parse_native_table(&bytes).is_err());
        }
    }

    #[test]
    fn oversized_embedded_drop_is_rejected_before_starting_worker() {
        let file = egui::DroppedFile {
            name: "huge.wt".to_owned(),
            bytes: Some(vec![0; MAX_WAVETABLE_FILE_BYTES + 1].into()),
            ..Default::default()
        };

        let result = ImportJob::from_dropped(&file);

        assert!(matches!(result, Err(error) if error.contains("file is too large")));
    }
}
