//! Compact editor card for ordered generator filters.

mod painting;

use crate::editor_theme;
use crate::filters::{FilterConfig, FilterMode};

use painting::{paint_header, paint_readout, paint_response_preview};

const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 20_000.0;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 32.0;
const MIN_SLOPE: f32 = 12.0;
const MAX_SLOPE: f32 = 24.0;
const MIN_RESPONSE_SEGMENTS: usize = 32;
const MAX_RESPONSE_SEGMENTS: usize = 128;

pub(crate) struct FilterModuleUi {
    pub(crate) changed: bool,
    pub(crate) remove: bool,
    pub(crate) rect: egui::Rect,
    pub(crate) drag_response: egui::Response,
    pub(crate) preview_response: egui::Response,
    pub(crate) cutoff_response: egui::Response,
    pub(crate) resonance_response: egui::Response,
    pub(crate) slope_response: egui::Response,
    pub(crate) morph_response: egui::Response,
}

pub(crate) fn draw_ordered_filter_module(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: u64,
    config: &mut FilterConfig,
    group_accent: egui::Color32,
) -> FilterModuleUi {
    let id = egui::Id::new(("ordered-filter", id_salt));
    let palette = editor_theme::semantic();
    let inset = editor_theme::graph_inset(ui).min(rect.width().min(rect.height()) * 0.08);
    let inner = rect.shrink(inset);
    let header_height =
        (editor_theme::font::CAPTION_SIZE + editor_theme::space::XS).min(inner.height() * 0.24);
    let header = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(inner.right(), inner.top() + header_height),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(inner.left(), header.bottom() + editor_theme::space::XXS),
        inner.max,
    );
    let controls_width = (body.width() * 0.30).max(editor_theme::font::VALUE_SIZE * 4.2);
    let controls = egui::Rect::from_min_max(
        egui::pos2((body.right() - controls_width).max(body.left()), body.top()),
        body.max,
    );
    let preview = egui::Rect::from_min_max(
        body.min,
        egui::pos2(controls.left() - editor_theme::space::XXS, body.bottom()),
    );
    ui.painter()
        .rect_filled(preview, editor_theme::shape::CONTROL_RADIUS, palette.well);

    let action_side = header.height();
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(header.right() - action_side, header.top()),
        egui::vec2(action_side, action_side),
    );
    let picker_rect = egui::Rect::from_min_max(
        egui::pos2(close_rect.left() - action_side * 4.4, header.top()),
        egui::pos2(close_rect.left(), header.bottom()),
    );
    let drag_rect = egui::Rect::from_min_max(
        header.min,
        egui::pos2(
            (picker_rect.left() - editor_theme::space::XXS).max(header.left()),
            header.bottom(),
        ),
    );
    let drag_response = ui
        .interact(drag_rect, id.with("drag"), egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to reorder this filter or move it to another group.");
    let close_response = ui
        .interact(close_rect, id.with("remove"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove filter");

    let mut changed = sanitize_config(config);
    let picker_response = ui
        .interact(picker_rect, id.with("mode-picker"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Choose SVF morph, phaser, or Fibonacci phase notches");
    ui.painter().rect_stroke(
        picker_rect.shrink(editor_theme::shape::STROKE),
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            if picker_response.hovered() {
                group_accent
            } else {
                palette.grid
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        picker_rect.center(),
        egui::Align2::CENTER_CENTER,
        config.mode.short_label(),
        editor_theme::font::caption(),
        group_accent,
    );
    let mut selected_mode = None;
    egui::Popup::menu(&picker_response).show(|ui| {
        ui.set_min_width(action_side * 6.0);
        for mode in FilterMode::ALL {
            if ui
                .selectable_label(config.mode == mode, mode.label())
                .clicked()
            {
                selected_mode = Some(mode);
            }
        }
    });
    if let Some(mode) = selected_mode
        && config.mode != mode
    {
        *config = defaults_for_mode(mode);
        changed = true;
    }

    let cells = vertical_cells::<4>(controls, 4);
    let preview_response = ui
        .interact(preview, id.with("response"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Crosshair)
        .on_hover_text("Drag cutoff horizontally and Q vertically. Hold Shift for fine control.");
    let cutoff_response = metric_response(ui, cells[0], id.with("cutoff"), "Cutoff");
    let resonance_response = metric_response(ui, cells[1], id.with("resonance"), "Q");
    let slope_response = metric_response(ui, cells[2], id.with("slope"), "dB per octave");
    let morph_response = metric_response(ui, cells[3], id.with("morph"), "Morph");
    let defaults = defaults_for_mode(config.mode);
    changed |= drag_log_value(
        ui,
        &cutoff_response,
        &mut config.cutoff_hz,
        MIN_CUTOFF_HZ,
        MAX_CUTOFF_HZ,
        defaults.cutoff_hz,
    );
    changed |= drag_log_value(
        ui,
        &resonance_response,
        &mut config.q,
        MIN_Q,
        MAX_Q,
        defaults.q,
    );
    changed |= drag_linear_value(
        ui,
        &slope_response,
        &mut config.slope_db_oct,
        MIN_SLOPE,
        MAX_SLOPE,
        defaults.slope_db_oct,
    );
    changed |= drag_linear_value(
        ui,
        &morph_response,
        &mut config.morph,
        0.0,
        1.0,
        defaults.morph,
    );
    changed |= drag_filter_response(ui, &preview_response, config, defaults, preview);

    paint_header(
        ui,
        drag_rect,
        close_rect,
        &drag_response,
        &close_response,
        group_accent,
    );
    paint_response_preview(ui, preview, *config, group_accent, &preview_response);
    paint_readout(
        ui,
        cells[0],
        "CUTOFF",
        &format_frequency(config.cutoff_hz),
        normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
        &cutoff_response,
        group_accent,
    );
    paint_readout(
        ui,
        cells[1],
        "Q",
        &format!("{:.2}", config.q),
        normalized_log(config.q, MIN_Q, MAX_Q),
        &resonance_response,
        group_accent,
    );
    paint_readout(
        ui,
        cells[2],
        "DB/OCT",
        &format!("{:.1}", config.slope_db_oct),
        (config.slope_db_oct - MIN_SLOPE) / (MAX_SLOPE - MIN_SLOPE),
        &slope_response,
        group_accent,
    );
    paint_readout(
        ui,
        cells[3],
        "MORPH",
        &format!("{:.0}%", config.morph * 100.0),
        config.morph,
        &morph_response,
        group_accent,
    );

    FilterModuleUi {
        changed,
        remove: close_response.clicked(),
        rect,
        drag_response,
        preview_response,
        cutoff_response,
        resonance_response,
        slope_response,
        morph_response,
    }
}

fn defaults_for_mode(mode: FilterMode) -> FilterConfig {
    match mode {
        FilterMode::Svf => FilterConfig::default(),
        FilterMode::Phaser => FilterConfig {
            mode,
            cutoff_hz: 800.0,
            q: 1.0,
            slope_db_oct: 24.0,
            morph: 1.0,
        },
        FilterMode::Fibonacci => FilterConfig {
            mode,
            cutoff_hz: 500.0,
            q: 1.2,
            slope_db_oct: 24.0,
            morph: 1.0,
        },
    }
}

fn vertical_cells<const N: usize>(rect: egui::Rect, count: usize) -> [egui::Rect; N] {
    std::array::from_fn(|index| {
        let top = egui::lerp(rect.top()..=rect.bottom(), index as f32 / count as f32);
        let bottom = egui::lerp(
            rect.top()..=rect.bottom(),
            (index + 1) as f32 / count as f32,
        );
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), top),
            egui::pos2(rect.right(), bottom),
        )
    })
}

fn metric_response(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    help: &str,
) -> egui::Response {
    ui.interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(format!(
            "{help}: drag vertically. Hold Shift for fine control; double-click to reset."
        ))
}

fn sanitize_config(config: &mut FilterConfig) -> bool {
    let before = *config;
    config.cutoff_hz = finite_or(config.cutoff_hz, 20_000.0).clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
    config.q = finite_or(config.q, std::f32::consts::FRAC_1_SQRT_2).clamp(MIN_Q, MAX_Q);
    config.slope_db_oct = finite_or(config.slope_db_oct, MIN_SLOPE).clamp(MIN_SLOPE, MAX_SLOPE);
    config.morph = finite_or(config.morph, 0.0).clamp(0.0, 1.0);
    *config != before
}

fn drag_log_value(
    ui: &egui::Ui,
    response: &egui::Response,
    value: &mut f32,
    minimum: f32,
    maximum: f32,
    default: f32,
) -> bool {
    let before = *value;
    if response.dragged() {
        let fine = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        let normalized = crate::editor_controls::accumulate_drag(
            normalized_log(*value, minimum, maximum),
            response.drag_motion().y * fine,
        );
        *value = denormalized_log(normalized.clamp(0.0, 1.0), minimum, maximum);
    } else if response.double_clicked() {
        *value = default;
    }
    value.to_bits() != before.to_bits()
}

fn drag_linear_value(
    ui: &egui::Ui,
    response: &egui::Response,
    value: &mut f32,
    minimum: f32,
    maximum: f32,
    default: f32,
) -> bool {
    let before = *value;
    if response.dragged() {
        let fine = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        let normalized = crate::editor_controls::accumulate_drag(
            (*value - minimum) / (maximum - minimum),
            response.drag_motion().y * fine,
        );
        *value = normalized
            .clamp(0.0, 1.0)
            .mul_add(maximum - minimum, minimum);
    } else if response.double_clicked() {
        *value = default;
    }
    value.to_bits() != before.to_bits()
}

fn drag_filter_response(
    ui: &egui::Ui,
    response: &egui::Response,
    config: &mut FilterConfig,
    defaults: FilterConfig,
    rect: egui::Rect,
) -> bool {
    let before = *config;
    if response.dragged()
        && rect.is_positive()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        if ui.input(|input| input.modifiers.shift) {
            let motion = ui.input(|input| input.pointer.delta());
            let cutoff = normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
                + motion.x / rect.width().max(1.0) * 0.1;
            let q =
                normalized_log(config.q, MIN_Q, MAX_Q) - motion.y / rect.height().max(1.0) * 0.1;
            config.cutoff_hz =
                denormalized_log(cutoff.clamp(0.0, 1.0), MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
            config.q = denormalized_log(q.clamp(0.0, 1.0), MIN_Q, MAX_Q);
        } else {
            config.cutoff_hz = denormalized_log(
                ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                MIN_CUTOFF_HZ,
                MAX_CUTOFF_HZ,
            );
            config.q = denormalized_log(
                (1.0 - (pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
                MIN_Q,
                MAX_Q,
            );
        }
    } else if response.double_clicked() {
        config.cutoff_hz = defaults.cutoff_hz;
        config.q = defaults.q;
    }
    *config != before
}

pub(super) fn normalized_log(value: f32, minimum: f32, maximum: f32) -> f32 {
    (value.clamp(minimum, maximum) / minimum).ln() / (maximum / minimum).ln()
}

pub(super) fn denormalized_log(normalized: f32, minimum: f32, maximum: f32) -> f32 {
    minimum * (maximum / minimum).powf(normalized)
}

fn format_frequency(value: f32) -> String {
    if value >= 1_000.0 {
        format!("{:.2} kHz", value / 1_000.0)
    } else {
        format!("{value:.0} Hz")
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
