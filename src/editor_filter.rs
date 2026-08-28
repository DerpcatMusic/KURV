//! Compact editor card for ordered generator filters.

mod painting;

use crate::editor_theme;
use crate::filters::{FilterConfig, FilterMode, MAX_Q, MAX_SLOPE_DB, MIN_Q, MIN_SLOPE_DB};

use painting::{paint_header, paint_metric_knob, paint_response_preview, paint_type_dropdown};

const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 20_000.0;
const MIN_SLOPE: f32 = MIN_SLOPE_DB;
const MAX_SLOPE: f32 = MAX_SLOPE_DB;
const MIN_RESPONSE_SEGMENTS: usize = 64;
const MAX_RESPONSE_SEGMENTS: usize = 256;

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

pub(crate) fn filter_type_popup_open(ui: &egui::Ui, id_salt: u64) -> bool {
    let response_id = egui::Id::new(("ordered-filter", id_salt)).with("mode-picker");
    egui::Popup::is_id_open(ui.ctx(), response_id.with("popup"))
}

pub(crate) fn draw_ordered_filter_module(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: u64,
    display_number: usize,
    config: &mut FilterConfig,
    group_accent: egui::Color32,
    dsp_sample_rate: f32,
) -> FilterModuleUi {
    let id = egui::Id::new(("ordered-filter", id_salt));
    let palette = editor_theme::semantic();
    let inset = editor_theme::graph_inset(ui).min(rect.width().min(rect.height()) * 0.08);
    let inner = rect.shrink(inset);
    let identity_width = (inner.width() * 0.055).max(editor_theme::font::CAPTION_SIZE * 2.1);
    let identity = egui::Rect::from_min_size(
        inner.min,
        egui::vec2(identity_width.min(inner.width() * 0.16), inner.height()),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + editor_theme::space::XXS, inner.top()),
        inner.max,
    );
    // Phase Plant-style composition: the response graph owns the center and
    // the compact parameter strip sits on the right. The filter remains a
    // self-contained module; no analyzer/scope is introduced beside it.
    let controls_width = (body.width() * 0.43)
        .max(editor_theme::font::VALUE_SIZE * 21.0)
        .min(body.width() * 0.52);
    let controls = egui::Rect::from_min_max(
        egui::pos2((body.right() - controls_width).max(body.left()), body.top()),
        body.max,
    );
    let preview = egui::Rect::from_min_max(
        body.min,
        egui::pos2(
            (controls.left() - editor_theme::space::XXS).max(body.left()),
            body.bottom(),
        ),
    );
    ui.painter()
        .rect_filled(body, editor_theme::shape::CONTROL_RADIUS, palette.well);
    let cells = horizontal_cells::<5>(controls, [1.7, 1.25, 1.0, 1.25, 1.0]);
    let picker_rect = cells[0];

    let close_side = identity.width() * 0.42;
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - close_side * 0.42,
            identity.top() + close_side * 0.42,
        ),
        egui::Vec2::splat(close_side),
    );
    let grip_height = (identity.height() * 0.18)
        .max(editor_theme::space::MD)
        .min((identity.height() - close_side).max(0.0));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), identity.bottom() - grip_height),
        identity.right_bottom(),
    );
    let drag_response = ui
        .interact(drag_rect, id.with("drag"), egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag this grip to reorder or move the filter between groups.");
    let grip_color = if drag_response.dragged() {
        palette.text
    } else if drag_response.hovered() || drag_response.has_focus() {
        group_accent
    } else {
        palette.text_muted.gamma_multiply(0.56)
    };
    let grip_gap = editor_theme::space::XXS;
    let grip_origin = drag_rect.center() - egui::vec2(grip_gap * 0.5, grip_gap);
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                grip_origin + egui::vec2(column as f32 * grip_gap, row as f32 * grip_gap),
                editor_theme::shape::STROKE,
                grip_color,
            );
        }
    }
    let close_response = ui
        .interact(close_rect, id.with("remove"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove filter");

    let mut changed = sanitize_config(config);
    let picker_response = ui
        .interact(picker_rect, id.with("mode-picker"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Choose the filter type. Double-click to reset.");
    let mut selected_mode = None;
    if picker_response.double_clicked() {
        *config = FilterConfig::default();
        changed = true;
    } else {
        egui::Popup::menu(&picker_response).show(|ui| {
            ui.set_min_width(controls.width().max(editor_theme::font::VALUE_SIZE * 7.0));
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
            *config = FilterConfig::for_mode(mode);
            changed = true;
        }
    }
    paint_type_dropdown(ui, picker_rect, config.mode, &picker_response, group_accent);

    let preview_response = ui
        .interact(preview, id.with("response"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Crosshair)
        .on_hover_text(match config.mode {
            FilterMode::Svf => {
                "Drag cutoff horizontally and Q vertically. Hold Shift for fine control. Double-click to reset."
            }
            FilterMode::Phaser => {
                "Drag cutoff horizontally and Q vertically. Hold Shift for fine control. Double-click to reset."
            }
            FilterMode::Scream => {
                "Drag cutoff horizontally and resonance vertically. Hold Shift for fine control. Double-click to reset."
            }
        });
    let cutoff_response = metric_response(ui, cells[1], id.with("cutoff"), "Cutoff");
    let resonance_response = metric_response(
        ui,
        cells[2],
        id.with("resonance"),
        config.mode.resonance_help(),
    );
    let slope_response = metric_response(ui, cells[3], id.with("slope"), config.mode.slope_help());
    let morph_response = metric_response(ui, cells[4], id.with("morph"), "Morph");
    let defaults = FilterConfig::for_mode(config.mode);
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
    changed |= drag_log_value(
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
        identity,
        close_rect,
        display_number,
        &drag_response,
        &close_response,
        group_accent,
    );
    paint_response_preview(
        ui,
        preview,
        *config,
        group_accent,
        &preview_response,
        dsp_sample_rate,
    );
    paint_metric_knob(
        ui,
        cells[1],
        "CUTOFF",
        &format_frequency(config.cutoff_hz),
        normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
        &cutoff_response,
        group_accent,
    );
    paint_metric_knob(
        ui,
        cells[2],
        config.mode.resonance_label(),
        &format_q(*config),
        config.normalized_q(),
        &resonance_response,
        group_accent,
    );
    paint_metric_knob(
        ui,
        cells[3],
        config.mode.slope_label(),
        &format_slope(*config),
        config.normalized_slope(),
        &slope_response,
        group_accent,
    );
    paint_metric_knob(
        ui,
        cells[4],
        config.mode.morph_label(),
        &format_morph(*config),
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

fn horizontal_cells<const N: usize>(rect: egui::Rect, weights: [f32; N]) -> [egui::Rect; N] {
    let gap = editor_theme::space::XXS;
    let usable = (rect.width() - gap * N.saturating_sub(1) as f32).max(0.0);
    let total = weights.iter().copied().sum::<f32>().max(f32::EPSILON);
    let mut left = rect.left();
    std::array::from_fn(|index| {
        let width = if index + 1 == N {
            (rect.right() - left).max(0.0)
        } else {
            usable * weights[index].max(0.0) / total
        };
        let cell = egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2((left + width).min(rect.right()), rect.bottom()),
        );
        left = (cell.right() + gap).min(rect.right());
        cell
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
    *config = config.sanitized();
    config.cutoff_hz = config.cutoff_hz.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
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
    crate::editor_controls::update_custom_value_drag(
        ui,
        response,
        value,
        minimum..=maximum,
        (maximum - minimum) / 150.0,
        default,
    )
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

fn format_slope(config: FilterConfig) -> String {
    let db = config.slope_db_oct;
    if matches!(config.mode, FilterMode::Phaser | FilterMode::Scream) {
        return format!("{:.0}%", normalized_log(db, MIN_SLOPE, MAX_SLOPE) * 100.0);
    }
    let poles = (db / 6.0).round().clamp(1.0, 128.0) as i32;
    if poles >= 96 {
        format!("{poles}P")
    } else if db >= 48.0 {
        format!("{db:.0}")
    } else {
        format!("{db:.1}")
    }
}

fn format_morph(config: FilterConfig) -> String {
    match config.mode {
        FilterMode::Svf => format!("{:.0}%", config.morph * 100.0),
        FilterMode::Phaser => format!("{:.1}P", config.effective_poles()),
        FilterMode::Scream => format!("{:.0}%", config.morph * 100.0),
    }
}

fn format_q(config: FilterConfig) -> String {
    match config.mode {
        FilterMode::Svf => format!("{:.2}", config.q),
        FilterMode::Phaser | FilterMode::Scream => {
            format!("{:.0}%", config.normalized_q() * 100.0)
        }
    }
}
