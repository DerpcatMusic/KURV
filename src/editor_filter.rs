//! Compact editor card for ordered generator filters.

mod painting;

use crate::editor_theme;
use crate::filters::{
    FilterConfig, FilterDomain, FilterMode, MAX_Q, MAX_RATIO, MAX_SLOPE_DB, MIN_Q, MIN_RATIO,
    MIN_SLOPE_DB, OBJECT_MAX_DECAY, OBJECT_MIN_DECAY, denormalized_ratio, normalized_ratio,
    ratio_brickwall_bypassed,
};

pub(crate) use painting::paint_metric_knob;
use painting::{paint_header, paint_pass_toggle, paint_response_preview, paint_type_dropdown};

const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 20_000.0;
const MAX_SLOPE: f32 = MAX_SLOPE_DB;
const MIN_RESPONSE_SEGMENTS: usize = 64;
const MAX_RESPONSE_SEGMENTS: usize = 256;

pub(crate) struct FilterModuleUi {
    pub(crate) changed: bool,
    pub(crate) remove: bool,
    pub(crate) drag_response: egui::Response,
    pub(crate) preview_response: egui::Response,
    pub(crate) cutoff_response: egui::Response,
    pub(crate) resonance_response: Option<egui::Response>,
    pub(crate) slope_response: Option<egui::Response>,
    pub(crate) morph_response: Option<egui::Response>,
    pub(crate) shape_response: Option<egui::Response>,
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
    let inner = rect.shrink(editor_theme::shape::STROKE);
    let identity_width = (inner.width() * 0.055).max(editor_theme::font::CAPTION_SIZE * 2.1);
    let identity = egui::Rect::from_min_size(
        inner.min,
        egui::vec2(identity_width.min(inner.width() * 0.16), inner.height()),
    );
    ui.painter().rect_filled(identity, 0.0, palette.control);
    ui.painter().line_segment(
        [identity.right_top(), identity.right_bottom()],
        egui::Stroke::new(
            editor_theme::shape::GROUP_STROKE,
            group_accent.gamma_multiply(0.72),
        ),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + editor_theme::space::XXS, inner.top()),
        inner.max,
    );
    // Phase Plant-style composition: the response graph owns the center and
    // the compact parameter strip sits on the right. The filter remains a
    // self-contained module; no analyzer/scope is introduced beside it.
    let controls_share = match config.mode {
        FilterMode::Phaser | FilterMode::Object => 0.49,
        FilterMode::RatioBrickwall => 0.28,
        _ => 0.43,
    };
    let controls_width = (body.width() * controls_share)
        .max(editor_theme::font::VALUE_SIZE * 21.0)
        .min(body.width() * 0.58);
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
    let (picker_rect, cutoff_rect, pass_rect, resonance_rect, slope_rect, morph_rect, shape_rect) =
        if config.mode == FilterMode::RatioBrickwall {
            let cells = horizontal_cells::<3>(controls, [1.55, 0.82, 1.0]);
            (cells[0], cells[2], Some(cells[1]), None, None, None, None)
        } else if matches!(config.mode, FilterMode::Phaser | FilterMode::Object) {
            let cells = horizontal_cells::<6>(controls, [1.7, 1.2, 0.95, 1.15, 0.95, 1.0]);
            (
                cells[0],
                cells[1],
                None,
                Some(cells[2]),
                Some(cells[3]),
                Some(cells[4]),
                Some(cells[5]),
            )
        } else {
            let cells = horizontal_cells::<5>(controls, [1.7, 1.25, 1.0, 1.25, 1.0]);
            (
                cells[0],
                cells[1],
                None,
                Some(cells[2]),
                Some(cells[3]),
                Some(cells[4]),
                None,
            )
        };

    let close_side = identity.width() * 0.42;
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - close_side * 0.42,
            identity.top() + close_side * 0.42,
        ),
        egui::Vec2::splat(close_side),
    );
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), close_rect.bottom()),
        identity.right_bottom(),
    );
    let drag_response = ui
        .interact(drag_rect, id.with("drag"), egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag the filter name to move; hold Ctrl to duplicate");
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
            ui.set_min_width(
                picker_rect
                    .width()
                    .max(editor_theme::font::VALUE_SIZE * 7.0),
            );
            for (index, domain) in FilterDomain::ALL.into_iter().enumerate() {
                if index != 0 {
                    ui.add_space(editor_theme::space::XS);
                }
                ui.label(
                    egui::RichText::new(domain.label())
                        .font(editor_theme::font::caption())
                        .color(editor_theme::semantic().text_muted),
                );
                for mode in FilterMode::ALL
                    .into_iter()
                    .filter(|mode| mode.domain() == domain)
                {
                    if ui
                        .selectable_label(config.mode == mode, mode.label())
                        .clicked()
                    {
                        selected_mode = Some(mode);
                    }
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
                "Drag cutoff horizontally and Q vertically. Hold Shift for fine control or Ctrl for semantic snap. Double-click to reset."
            }
            FilterMode::Phaser => {
                "Drag cutoff horizontally and Q vertically. Hold Shift for fine control or Ctrl for semantic snap. Double-click to reset."
            }
            FilterMode::Scream => {
                "Drag cutoff horizontally and resonance vertically. Hold Shift for fine control or Ctrl for semantic snap. Double-click to reset."
            }
            FilterMode::Object => {
                "Drag frequency horizontally and decay vertically. Hold Shift for fine control or Ctrl for semantic snap. Double-click to reset."
            }
            FilterMode::RatioBrickwall => {
                "Drag the harmonic cutoff horizontally. HIGH removes harmonics at and below the ratio; LOW removes harmonics above it. 0x bypasses; LOW 1024x also bypasses."
            }
        });
    let pass_response = pass_rect.map(|rect| {
        ui.interact(rect, id.with("pass"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Switch between harmonic low-pass and high-pass")
    });
    if pass_response.as_ref().is_some_and(|response| {
        response.clicked()
            || response.has_focus()
                && ui.input(|input| {
                    input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
                })
    }) {
        config.shape = if config.shape >= 0.5 { 0.0 } else { 1.0 };
        changed = true;
    }
    let cutoff_response = metric_response(
        ui,
        cutoff_rect,
        id.with("cutoff"),
        if config.mode == FilterMode::RatioBrickwall {
            "Harmonic ratio"
        } else if config.mode == FilterMode::Object {
            "Modal base frequency"
        } else {
            "Cutoff"
        },
    );
    let resonance_response = resonance_rect
        .map(|rect| metric_response(ui, rect, id.with("resonance"), config.mode.resonance_help()));
    let slope_response = slope_rect
        .map(|rect| metric_response(ui, rect, id.with("slope"), config.mode.slope_help()));
    let morph_response =
        morph_rect.map(|rect| metric_response(ui, rect, id.with("morph"), "Morph"));
    let shape_response = shape_rect.map(|rect| {
        metric_response(
            ui,
            rect,
            id.with("shape"),
            if config.mode == FilterMode::Object {
                "Strike position/formant emphasis across the physical modes"
            } else {
                "Notch width around each stage. Broad keeps wide passbands; Brick widens each notch until the remaining bands are thin."
            },
        )
    });
    let defaults = FilterConfig::for_mode(config.mode);
    changed |= if config.mode == FilterMode::RatioBrickwall {
        drag_ratio_value(
            ui,
            &cutoff_response,
            &mut config.cutoff_hz,
            defaults.cutoff_hz,
        )
    } else {
        drag_log_value(
            ui,
            &cutoff_response,
            &mut config.cutoff_hz,
            MIN_CUTOFF_HZ,
            MAX_CUTOFF_HZ,
            defaults.cutoff_hz,
            crate::editor_controls::ValueSemantic::Cutoff,
        )
    };
    if let Some(response) = &resonance_response {
        let (minimum, maximum, semantic) = if config.mode == FilterMode::Object {
            (
                OBJECT_MIN_DECAY,
                OBJECT_MAX_DECAY,
                crate::editor_controls::ValueSemantic::Continuous,
            )
        } else {
            (
                MIN_Q,
                MAX_Q,
                if config.mode == FilterMode::Svf {
                    crate::editor_controls::ValueSemantic::Q
                } else {
                    crate::editor_controls::ValueSemantic::Percent
                },
            )
        };
        changed |= drag_log_value(
            ui,
            response,
            &mut config.q,
            minimum,
            maximum,
            defaults.q,
            semantic,
        );
    }
    let minimum_slope = config.minimum_slope();
    if let Some(response) = &slope_response {
        changed |= if config.mode == FilterMode::Object {
            drag_linear_value(
                ui,
                response,
                &mut config.slope_db_oct,
                0.0,
                1.0,
                defaults.slope_db_oct,
                crate::editor_controls::ValueSemantic::Percent,
            )
        } else {
            drag_log_value(
                ui,
                response,
                &mut config.slope_db_oct,
                minimum_slope,
                MAX_SLOPE,
                defaults.slope_db_oct,
                if config.mode == FilterMode::Svf {
                    crate::editor_controls::ValueSemantic::Slope
                } else {
                    crate::editor_controls::ValueSemantic::Percent
                },
            )
        };
    }
    if let Some(response) = &morph_response {
        changed |= drag_linear_value(
            ui,
            response,
            &mut config.morph,
            0.0,
            1.0,
            defaults.morph,
            crate::editor_controls::ValueSemantic::Percent,
        );
    }
    if let Some(response) = &shape_response {
        changed |= drag_linear_value(
            ui,
            response,
            &mut config.shape,
            0.0,
            1.0,
            defaults.shape,
            crate::editor_controls::ValueSemantic::Percent,
        );
    }
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
    if let (Some(rect), Some(response)) = (pass_rect, &pass_response) {
        paint_pass_toggle(ui, rect, config.shape >= 0.5, response, group_accent);
    }
    paint_metric_knob(
        ui,
        cutoff_rect,
        if config.mode == FilterMode::RatioBrickwall {
            "RATIO"
        } else if config.mode == FilterMode::Object {
            "FREQ"
        } else {
            "CUTOFF"
        },
        &if config.mode == FilterMode::RatioBrickwall {
            format_ratio(*config)
        } else {
            format_frequency(config.cutoff_hz)
        },
        if config.mode == FilterMode::RatioBrickwall {
            normalized_ratio(config.cutoff_hz)
        } else {
            normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
        },
        &cutoff_response,
        group_accent,
    );
    if let (Some(rect), Some(response)) = (resonance_rect, &resonance_response) {
        paint_metric_knob(
            ui,
            rect,
            config.mode.resonance_label(),
            &format_q(*config),
            config.normalized_q(),
            response,
            group_accent,
        );
    }
    if let (Some(rect), Some(response)) = (slope_rect, &slope_response) {
        paint_metric_knob(
            ui,
            rect,
            config.mode.slope_label(),
            &format_slope(*config),
            config.normalized_slope(),
            response,
            group_accent,
        );
    }
    if let (Some(rect), Some(response)) = (morph_rect, &morph_response) {
        paint_metric_knob(
            ui,
            rect,
            config.mode.morph_label(),
            &format_morph(*config),
            config.morph,
            response,
            group_accent,
        );
    }
    if let (Some(rect), Some(response)) = (shape_rect, &shape_response) {
        paint_metric_knob(
            ui,
            rect,
            if config.mode == FilterMode::Object {
                "FORMANT"
            } else {
                "SHAPE"
            },
            &if config.mode == FilterMode::Object {
                format_object_formant(config.shape)
            } else {
                format_shape(config.shape)
            },
            config.shape,
            response,
            group_accent,
        );
    }

    FilterModuleUi {
        changed,
        remove: close_response.clicked(),
        drag_response,
        preview_response,
        cutoff_response,
        resonance_response,
        slope_response,
        morph_response,
        shape_response,
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
            "{help}: drag vertically. Hold Shift for fine control or Ctrl for semantic snap; double-click to reset."
        ))
}

fn sanitize_config(config: &mut FilterConfig) -> bool {
    let before = *config;
    *config = config.sanitized();
    config.cutoff_hz = if config.mode == FilterMode::RatioBrickwall {
        config.cutoff_hz.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        config.cutoff_hz.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
    };
    *config != before
}

fn drag_ratio_value(
    ui: &egui::Ui,
    response: &egui::Response,
    value: &mut f32,
    default: f32,
) -> bool {
    let before = *value;
    if response.double_clicked() {
        *value = default;
        return value.to_bits() != before.to_bits();
    }
    let mut normalized = normalized_ratio(*value);
    if crate::editor_controls::update_custom_value_drag(
        ui,
        response,
        &mut normalized,
        0.0..=1.0,
        1.0 / 150.0,
        normalized_ratio(default),
        crate::editor_controls::ValueSemantic::Continuous,
    ) {
        *value = crate::editor_controls::semantic_snap(
            denormalized_ratio(normalized),
            crate::editor_controls::ValueSemantic::Ratio,
            ui.input(|input| input.modifiers.ctrl),
        )
        .clamp(MIN_RATIO, MAX_RATIO);
    }
    value.to_bits() != before.to_bits()
}

fn drag_log_value(
    ui: &egui::Ui,
    response: &egui::Response,
    value: &mut f32,
    minimum: f32,
    maximum: f32,
    default: f32,
    semantic: crate::editor_controls::ValueSemantic,
) -> bool {
    let before = *value;
    if response.double_clicked() {
        *value = default;
        return value.to_bits() != before.to_bits();
    }
    let mut normalized = normalized_log(*value, minimum, maximum);
    if crate::editor_controls::update_custom_value_drag(
        ui,
        response,
        &mut normalized,
        0.0..=1.0,
        1.0 / 150.0,
        normalized_log(default, minimum, maximum),
        crate::editor_controls::ValueSemantic::Continuous,
    ) {
        let coarse = ui.input(|input| input.modifiers.ctrl);
        if semantic == crate::editor_controls::ValueSemantic::Percent {
            normalized = crate::editor_controls::semantic_snap(normalized, semantic, coarse);
        }
        let raw = denormalized_log(normalized.clamp(0.0, 1.0), minimum, maximum);
        *value = crate::editor_controls::semantic_snap(
            raw,
            if semantic == crate::editor_controls::ValueSemantic::Percent {
                crate::editor_controls::ValueSemantic::Continuous
            } else {
                semantic
            },
            coarse,
        )
        .clamp(minimum, maximum);
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
    semantic: crate::editor_controls::ValueSemantic,
) -> bool {
    crate::editor_controls::update_custom_value_drag(
        ui,
        response,
        value,
        minimum..=maximum,
        (maximum - minimum) / 150.0,
        default,
        semantic,
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
        if config.mode == FilterMode::RatioBrickwall {
            let normalized = if ui.input(|input| input.modifiers.shift) {
                normalized_ratio(config.cutoff_hz)
                    + ui.input(|input| input.pointer.delta().x) / rect.width().max(1.0) * 0.1
            } else {
                (pointer.x - rect.left()) / rect.width()
            };
            config.cutoff_hz = crate::editor_controls::semantic_snap(
                denormalized_ratio(normalized),
                crate::editor_controls::ValueSemantic::Ratio,
                ui.input(|input| input.modifiers.ctrl),
            )
            .clamp(MIN_RATIO, MAX_RATIO);
            return config.cutoff_hz.to_bits() != before.cutoff_hz.to_bits();
        }
        if ui.input(|input| input.modifiers.shift) {
            let motion = ui.input(|input| input.pointer.delta());
            let cutoff = normalized_log(config.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
                + motion.x / rect.width().max(1.0) * 0.1;
            let (q_min, q_max) = if config.mode == FilterMode::Object {
                (OBJECT_MIN_DECAY, OBJECT_MAX_DECAY)
            } else {
                (MIN_Q, MAX_Q)
            };
            let q =
                normalized_log(config.q, q_min, q_max) - motion.y / rect.height().max(1.0) * 0.1;
            config.cutoff_hz = crate::editor_controls::semantic_snap(
                denormalized_log(cutoff.clamp(0.0, 1.0), MIN_CUTOFF_HZ, MAX_CUTOFF_HZ),
                crate::editor_controls::ValueSemantic::Cutoff,
                ui.input(|input| input.modifiers.ctrl),
            )
            .clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
            config.q = snap_filter_q(
                config.mode,
                denormalized_log(q.clamp(0.0, 1.0), q_min, q_max),
                ui.input(|input| input.modifiers.ctrl),
            );
        } else {
            let coarse = ui.input(|input| input.modifiers.ctrl);
            let (q_min, q_max) = if config.mode == FilterMode::Object {
                (OBJECT_MIN_DECAY, OBJECT_MAX_DECAY)
            } else {
                (MIN_Q, MAX_Q)
            };
            config.cutoff_hz = crate::editor_controls::semantic_snap(
                denormalized_log(
                    ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                    MIN_CUTOFF_HZ,
                    MAX_CUTOFF_HZ,
                ),
                crate::editor_controls::ValueSemantic::Cutoff,
                coarse,
            )
            .clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
            config.q = snap_filter_q(
                config.mode,
                denormalized_log(
                    (1.0 - (pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
                    q_min,
                    q_max,
                ),
                coarse,
            );
        }
    } else if response.double_clicked() {
        config.cutoff_hz = defaults.cutoff_hz;
        config.q = defaults.q;
    }
    *config != before
}

fn snap_filter_q(mode: FilterMode, value: f32, coarse: bool) -> f32 {
    if mode == FilterMode::Object {
        return value.clamp(OBJECT_MIN_DECAY, OBJECT_MAX_DECAY);
    }
    if mode == FilterMode::Svf {
        return crate::editor_controls::semantic_snap(
            value,
            crate::editor_controls::ValueSemantic::Q,
            coarse,
        )
        .clamp(MIN_Q, MAX_Q);
    }
    let normalized = normalized_log(value, MIN_Q, MAX_Q);
    denormalized_log(
        crate::editor_controls::semantic_snap(
            normalized,
            crate::editor_controls::ValueSemantic::Percent,
            coarse,
        ),
        MIN_Q,
        MAX_Q,
    )
}

fn format_ratio(config: FilterConfig) -> String {
    if ratio_brickwall_bypassed(config.cutoff_hz, config.shape >= 0.5) {
        "BYPASS".into()
    } else {
        format!("{:.0}x", config.cutoff_hz.ceil())
    }
}

pub(crate) fn normalized_log(value: f32, minimum: f32, maximum: f32) -> f32 {
    (value.clamp(minimum, maximum) / minimum).ln() / (maximum / minimum).ln()
}

pub(crate) fn denormalized_log(normalized: f32, minimum: f32, maximum: f32) -> f32 {
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
    if config.mode == FilterMode::Object {
        return format!("{:.0}%", db * 100.0);
    }
    if matches!(config.mode, FilterMode::Phaser | FilterMode::Scream) {
        return format!(
            "{:.0}%",
            normalized_log(db, MIN_SLOPE_DB, MAX_SLOPE) * 100.0
        );
    }
    if db > 96.0 {
        let amount = ((db - 96.0) / (MAX_SLOPE - 96.0)).clamp(0.0, 1.0);
        return if amount >= 0.995 {
            "BRICK".into()
        } else {
            format!("BRICK {:.0}%", amount * 100.0)
        };
    }
    if db >= 48.0 {
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
        FilterMode::Object => format!("{:.0}%", config.morph * 100.0),
        FilterMode::RatioBrickwall => "—".into(),
    }
}

fn format_q(config: FilterConfig) -> String {
    match config.mode {
        FilterMode::Svf => format!("{:.2}", config.q),
        FilterMode::Phaser | FilterMode::Scream => {
            format!("{:.0}%", config.normalized_q() * 100.0)
        }
        FilterMode::Object => format!("{:.2}s", config.q),
        FilterMode::RatioBrickwall => "—".into(),
    }
}

fn format_shape(shape: f32) -> String {
    if shape >= 0.995 {
        "BRICK".into()
    } else if shape <= 0.005 {
        "BROAD".into()
    } else {
        format!("{:.0}%", shape * 100.0)
    }
}

fn format_object_formant(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}
