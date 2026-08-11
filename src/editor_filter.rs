//! Compact editor card for ordered generator filters.

use crate::editor_theme;
use crate::filters::{FilterConfig, FilterMode};

const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 20_000.0;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 32.0;
const MIN_RESPONSE_SEGMENTS: usize = 32;
const MAX_RESPONSE_SEGMENTS: usize = 128;

/// Result of drawing one filter module. The header response is reserved for
/// structural drag-and-drop so parameter gestures do not move the module.
pub(crate) struct FilterModuleUi {
    pub(crate) changed: bool,
    pub(crate) remove: bool,
    pub(crate) rect: egui::Rect,
    pub(crate) drag_response: egui::Response,
    pub(crate) preview_response: egui::Response,
    pub(crate) cutoff_response: egui::Response,
    pub(crate) resonance_response: egui::Response,
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
    let readout_height = (editor_theme::font::CAPTION_SIZE
        + editor_theme::font::VALUE_SIZE
        + editor_theme::space::XXS)
        .min(inner.height() * 0.36);
    let header = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(inner.right(), inner.top() + header_height),
    );
    let readouts = egui::Rect::from_min_max(
        egui::pos2(inner.left(), inner.bottom() - readout_height),
        inner.max,
    );
    let preview = egui::Rect::from_min_max(
        egui::pos2(inner.left(), header.bottom() + editor_theme::space::XXS),
        egui::pos2(inner.right(), readouts.top() - editor_theme::space::XXS),
    );

    ui.painter()
        .rect_filled(preview, editor_theme::shape::CONTROL_RADIUS, palette.well);
    let action_side = header.height();
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(header.right() - action_side, header.top()),
        egui::vec2(action_side, action_side),
    );
    let modes_rect = egui::Rect::from_min_max(
        egui::pos2(close_rect.left() - action_side * 3.0, header.top()),
        egui::pos2(close_rect.left(), header.bottom()),
    );
    let drag_rect = egui::Rect::from_min_max(
        header.min,
        egui::pos2(
            (modes_rect.left() - editor_theme::space::XXS).max(header.left()),
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

    let defaults = FilterConfig::default();
    let mut changed = false;
    changed |= sanitize_config(config);
    for (index, (mode, label)) in [
        (FilterMode::LowPass, "LP"),
        (FilterMode::BandPass, "BP"),
        (FilterMode::HighPass, "HP"),
    ]
    .into_iter()
    .enumerate()
    {
        let mode_width = modes_rect.width() / 3.0;
        let mode_rect = egui::Rect::from_min_max(
            egui::pos2(
                modes_rect.left() + mode_width * index as f32,
                modes_rect.top(),
            ),
            egui::pos2(
                modes_rect.left() + mode_width * (index + 1) as f32,
                modes_rect.bottom(),
            ),
        );
        let response = ui
            .interact(mode_rect, id.with(("mode", index)), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() && config.mode != mode {
            config.mode = mode;
            changed = true;
        }
        let selected = config.mode == mode;
        ui.painter().text(
            mode_rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            editor_theme::font::caption(),
            if selected {
                group_accent
            } else if response.hovered() {
                palette.text
            } else {
                palette.text_muted
            },
        );
        if selected {
            ui.painter().line_segment(
                [mode_rect.left_bottom(), mode_rect.right_bottom()],
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, group_accent),
            );
        }
    }

    let cutoff_rect = egui::Rect::from_min_max(
        readouts.min,
        egui::pos2(readouts.center().x, readouts.bottom()),
    );
    let resonance_rect = egui::Rect::from_min_max(
        egui::pos2(readouts.center().x, readouts.top()),
        readouts.max,
    );
    let preview_response = ui
        .interact(preview, id.with("response"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Crosshair)
        .on_hover_text(
            "Filter response: drag horizontally for cutoff and vertically for resonance. Hold Shift for fine control; double-click to reset.",
        );
    let cutoff_response = ui
        .interact(
            cutoff_rect,
            id.with("cutoff"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Cutoff: drag vertically. Hold Shift for fine control; double-click to reset.",
        );
    let resonance_response = ui
        .interact(
            resonance_rect,
            id.with("resonance"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Resonance: drag vertically. Hold Shift for fine control; double-click to reset.",
        );
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
        cutoff_rect,
        "CUTOFF",
        &format_frequency(config.cutoff_hz),
        normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
        &cutoff_response,
        group_accent,
    );
    paint_readout(
        ui,
        resonance_rect,
        "RESONANCE",
        &format!("Q {:.2}", config.q),
        normalized_log(config.q, MIN_Q, MAX_Q),
        &resonance_response,
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
    }
}

fn sanitize_config(config: &mut FilterConfig) -> bool {
    let before = *config;
    config.cutoff_hz = finite_or(config.cutoff_hz, FilterConfig::default().cutoff_hz)
        .clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
    config.q = finite_or(config.q, FilterConfig::default().q).clamp(MIN_Q, MAX_Q);
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
        let motion = response.drag_motion().y;
        let fine = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        let normalized = crate::editor_controls::accumulate_drag(
            normalized_log(*value, minimum, maximum),
            motion * fine,
        );
        *value = denormalized_log(normalized.clamp(0.0, 1.0), minimum, maximum);
    } else if response.double_clicked() {
        *value = default.clamp(minimum, maximum);
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
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let fine = ui.input(|input| input.modifiers.shift);
        if fine {
            let motion = ui.input(|input| input.pointer.delta());
            let cutoff = normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
                + motion.x / rect.width().max(editor_theme::font::CAPTION_SIZE) * 0.1;
            let resonance = normalized_log(config.q, MIN_Q, MAX_Q)
                - motion.y / rect.height().max(editor_theme::font::CAPTION_SIZE) * 0.1;
            config.cutoff_hz =
                denormalized_log(cutoff.clamp(0.0, 1.0), MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
            config.q = denormalized_log(resonance.clamp(0.0, 1.0), MIN_Q, MAX_Q);
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

fn paint_header(
    ui: &egui::Ui,
    label_rect: egui::Rect,
    close_rect: egui::Rect,
    drag_response: &egui::Response,
    close_response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    ui.painter().text(
        label_rect.left_center() + egui::vec2(editor_theme::space::XXS, 0.0),
        egui::Align2::LEFT_CENTER,
        "FILTER",
        editor_theme::font::caption(),
        if drag_response.dragged() || drag_response.hovered() {
            accent
        } else {
            palette.text_muted
        },
    );

    let cross = close_rect.shrink(close_rect.width() * 0.34);
    let close_color = if close_response.hovered() {
        palette.danger
    } else {
        palette.text_muted
    };
    let stroke = egui::Stroke::new(editor_theme::shape::STROKE, close_color);
    ui.painter()
        .line_segment([cross.left_top(), cross.right_bottom()], stroke);
    ui.painter()
        .line_segment([cross.right_top(), cross.left_bottom()], stroke);
}

fn paint_response_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    config: FilterConfig,
    accent: egui::Color32,
    response: &egui::Response,
) {
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter_at(rect);
    let cutoff_x = egui::lerp(
        rect.left()..=rect.right(),
        normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
    );
    painter.line_segment(
        [
            egui::pos2(cutoff_x, rect.top()),
            egui::pos2(cutoff_x, rect.bottom()),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.18)),
    );

    let point_count = ((rect.width() / editor_theme::space::XS.max(1.0)).ceil() as usize)
        .clamp(MIN_RESPONSE_SEGMENTS, MAX_RESPONSE_SEGMENTS);
    let mut points = Vec::with_capacity(point_count + 1);
    for index in 0..=point_count {
        let normalized = index as f32 / point_count as f32;
        let frequency = denormalized_log(normalized, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
        let magnitude = response_magnitude(config.mode, frequency / config.cutoff_hz, config.q);
        let decibels = (20.0 * magnitude.max(f32::MIN_POSITIVE).log10()).clamp(-36.0, 12.0);
        points.push(egui::pos2(
            egui::lerp(rect.left()..=rect.right(), normalized),
            egui::lerp(rect.bottom()..=rect.top(), (decibels + 36.0) / 48.0),
        ));
    }
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.86 }),
        ),
    ));
    let handle_radius =
        editor_theme::font::CAPTION_SIZE * if response.dragged() { 0.34 } else { 0.27 };
    let handle_bounds = rect.shrink(handle_radius.min(rect.width().min(rect.height()) * 0.5));
    let handle = egui::pos2(
        cutoff_x.clamp(handle_bounds.left(), handle_bounds.right()),
        egui::lerp(
            rect.bottom()..=rect.top(),
            normalized_log(config.q, MIN_Q, MAX_Q),
        )
        .clamp(handle_bounds.top(), handle_bounds.bottom()),
    );
    painter.circle_filled(
        handle,
        handle_radius,
        if response.hovered() {
            editor_theme::semantic().text
        } else {
            accent
        },
    );
}

fn paint_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    normalized: f32,
    response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    let active = response.is_pointer_button_down_on() || response.dragged();
    let label_y = rect.top() + editor_theme::font::CAPTION_SIZE * 0.5;
    let value_y = rect.bottom() - editor_theme::font::VALUE_SIZE * 0.5;
    ui.painter().text(
        egui::pos2(rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        label,
        editor_theme::font::caption(),
        if active || response.hovered() {
            accent
        } else {
            palette.text_muted
        },
    );
    ui.painter().text(
        egui::pos2(rect.center().x, value_y),
        egui::Align2::CENTER_CENTER,
        value,
        editor_theme::font::value(),
        if active { palette.text } else { accent },
    );

    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - editor_theme::shape::STROKE),
        rect.right_bottom(),
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            track.min,
            egui::pos2(
                egui::lerp(track.left()..=track.right(), normalized),
                track.bottom(),
            ),
        ),
        0.0,
        accent.gamma_multiply(if active { 1.0 } else { 0.72 }),
    );
}

fn response_magnitude(mode: FilterMode, ratio: f32, q: f32) -> f32 {
    let denominator = ((1.0 - ratio * ratio).powi(2) + (ratio / q).powi(2)).sqrt();
    match mode {
        FilterMode::LowPass => denominator.recip(),
        FilterMode::BandPass => ratio / denominator,
        FilterMode::HighPass => ratio * ratio / denominator,
    }
}

fn normalized_log(value: f32, minimum: f32, maximum: f32) -> f32 {
    (value.clamp(minimum, maximum) / minimum).ln() / (maximum / minimum).ln()
}

fn denormalized_log(normalized: f32, minimum: f32, maximum: f32) -> f32 {
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
