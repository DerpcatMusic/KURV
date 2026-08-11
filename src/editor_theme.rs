//! Shared visual tokens for KURV's egui editor.

use std::cell::Cell;

use egui::{Stroke, Ui, Vec2};
use plugcat::layout::UiMetrics;
use plugcat::theme::{ThemeTokens, UiTheme, mix};
use plugcat::widgets::{WidgetColors, WidgetRadius, WidgetSpacing, WidgetStroke, WidgetTokens};

mod library;

pub(crate) use library::ThemeLibrary;

/// Default product accent. Runtime editor preferences may override it through
/// [`ThemeSettings`] without creating a host-automatable parameter.
pub(crate) const PRIMARY_ACCENT: egui::Color32 = egui::Color32::from_rgb(38, 210, 204);
pub(crate) const SECONDARY_ACCENT: egui::Color32 = egui::Color32::from_rgb(245, 173, 71);
pub(crate) const TERTIARY_ACCENT: egui::Color32 = egui::Color32::from_rgb(176, 126, 247);

/// Keep continuous editor visuals on the host's display cadence. Audio
/// processing never uses this timing path.
pub(crate) fn request_display_repaint(ui: &egui::Ui) {
    let predicted_dt = ui.input(|input| input.predicted_dt);
    let interval = if predicted_dt.is_finite() && predicted_dt > 0.0 {
        // egui subtracts one predicted frame from delayed repaint requests.
        // Request two frame intervals so the integration schedules the next
        // paint one display interval from now instead of waking immediately.
        predicted_dt * 2.0
    } else {
        // Without an integration prediction there is nothing to compensate
        // for, so the conservative fallback is one 60 Hz display interval.
        1.0 / 60.0
    };
    ui.ctx().request_repaint_after_secs(interval);
}

pub(crate) mod color {
    use egui::Color32;

    pub(crate) const DANGER: Color32 = Color32::from_rgb(241, 103, 91);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ThemePreset {
    #[default]
    Charcoal,
    Slate,
    Obsidian,
}

impl ThemePreset {
    pub(crate) const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Slate,
            2 => Self::Obsidian,
            _ => Self::Charcoal,
        }
    }

    const fn neutral_base(self) -> (u8, u8, u8) {
        match self {
            Self::Charcoal => (18, 20, 23),
            Self::Slate => (17, 20, 24),
            Self::Obsidian => (11, 12, 14),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThemeSettings {
    pub(crate) background_rgb: [u8; 3],
    pub(crate) tint: u8,
    pub(crate) contrast: u8,
    pub(crate) primary_rgb: [u8; 3],
    pub(crate) secondary_rgb: [u8; 3],
    pub(crate) tertiary_rgb: [u8; 3],
}

impl ThemeSettings {
    pub(crate) const fn from_persisted(
        schema: u8,
        preset: u8,
        background_rgb: [u8; 3],
        tint: u8,
        contrast: u8,
        primary_rgb: [u8; 3],
        secondary_rgb: [u8; 3],
        tertiary_rgb: [u8; 3],
    ) -> Self {
        if schema == 0 {
            let base = ThemePreset::from_index(preset).neutral_base();
            return Self {
                background_rgb: [base.0, base.1, base.2],
                tint: 8,
                contrast: 100,
                primary_rgb,
                secondary_rgb: [
                    SECONDARY_ACCENT.r(),
                    SECONDARY_ACCENT.g(),
                    SECONDARY_ACCENT.b(),
                ],
                tertiary_rgb: [
                    TERTIARY_ACCENT.r(),
                    TERTIARY_ACCENT.g(),
                    TERTIARY_ACCENT.b(),
                ],
            };
        }
        Self {
            background_rgb,
            tint: if schema < 2 { 8 } else { tint },
            contrast,
            primary_rgb,
            secondary_rgb,
            tertiary_rgb,
        }
    }
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            background_rgb: [18, 20, 23],
            tint: 8,
            contrast: 100,
            primary_rgb: [PRIMARY_ACCENT.r(), PRIMARY_ACCENT.g(), PRIMARY_ACCENT.b()],
            secondary_rgb: [
                SECONDARY_ACCENT.r(),
                SECONDARY_ACCENT.g(),
                SECONDARY_ACCENT.b(),
            ],
            tertiary_rgb: [
                TERTIARY_ACCENT.r(),
                TERTIARY_ACCENT.g(),
                TERTIARY_ACCENT.b(),
            ],
        }
    }
}

pub(crate) const BUILTIN_THEMES: [(&str, ThemeSettings); 2] = [
    (
        "KURV",
        ThemeSettings {
            background_rgb: [18, 20, 23],
            tint: 8,
            contrast: 100,
            primary_rgb: [38, 210, 204],
            secondary_rgb: [245, 173, 71],
            tertiary_rgb: [176, 126, 247],
        },
    ),
    (
        "Serum",
        ThemeSettings {
            background_rgb: [18, 20, 23],
            tint: 8,
            contrast: 100,
            primary_rgb: [88, 255, 0],
            secondary_rgb: [245, 173, 71],
            tertiary_rgb: [176, 126, 247],
        },
    ),
];

thread_local! {
    static ACTIVE_SETTINGS: Cell<ThemeSettings> = Cell::new(ThemeSettings::default());
    static ACTIVE_PALETTE: Cell<KurvPalette> = Cell::new(semantic_palette(ThemeSettings::default()));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KurvPalette {
    pub(crate) background: egui::Color32,
    pub(crate) chrome: egui::Color32,
    pub(crate) surface: egui::Color32,
    pub(crate) control: egui::Color32,
    pub(crate) well: egui::Color32,
    pub(crate) control_hover: egui::Color32,
    pub(crate) grid: egui::Color32,
    pub(crate) text: egui::Color32,
    pub(crate) text_muted: egui::Color32,
    pub(crate) disabled: egui::Color32,
    pub(crate) disabled_text: egui::Color32,
    pub(crate) primary: egui::Color32,
    pub(crate) pan_shape: egui::Color32,
    pub(crate) unison: egui::Color32,
    pub(crate) envelope: egui::Color32,
    pub(crate) danger: egui::Color32,
}

pub(crate) mod space {
    use egui::{Vec2, vec2};

    pub(crate) const XXS: f32 = 2.0;
    pub(crate) const XS: f32 = 4.0;
    pub(crate) const SM: f32 = 8.0;
    pub(crate) const MD: f32 = 12.0;
    pub(crate) const LG: f32 = 16.0;
    #[allow(
        dead_code,
        reason = "the optional editor shell consumes this shared spacing vector"
    )]
    pub(crate) const ITEM_GAP: Vec2 = vec2(SM, SM);
}

pub(crate) mod font {
    use egui::{FontFamily, FontId};

    pub(crate) const CAPTION_SIZE: f32 = 9.5;
    pub(crate) const LABEL_SIZE: f32 = 10.0;
    pub(crate) const TITLE_SIZE: f32 = 12.0;
    pub(crate) const VALUE_SIZE: f32 = 10.75;

    pub(crate) fn caption() -> FontId {
        FontId::new(CAPTION_SIZE, FontFamily::Proportional)
    }

    pub(crate) fn label() -> FontId {
        FontId::new(LABEL_SIZE, FontFamily::Proportional)
    }

    pub(crate) fn title() -> FontId {
        FontId::new(TITLE_SIZE, FontFamily::Proportional)
    }

    pub(crate) fn value() -> FontId {
        FontId::new(VALUE_SIZE, FontFamily::Monospace)
    }
}

pub(crate) mod shape {
    pub(crate) const CONTROL_RADIUS: f32 = 2.0;
    pub(crate) const STROKE: f32 = 1.0;
    pub(crate) const FOCUS_STROKE: f32 = 1.5;
    pub(crate) const GROUP_STROKE: f32 = FOCUS_STROKE + STROKE * 0.5;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ControlVisuals {
    pub(crate) fill: egui::Color32,
    pub(crate) stroke: Stroke,
    pub(crate) label: egui::Color32,
    pub(crate) value: egui::Color32,
    pub(crate) indicator: egui::Color32,
}

pub(crate) fn control_visuals(
    enabled: bool,
    hovered: bool,
    active: bool,
    focused: bool,
    accent: egui::Color32,
) -> ControlVisuals {
    let palette = semantic();
    if !enabled {
        return ControlVisuals {
            fill: palette.disabled,
            stroke: Stroke::new(shape::STROKE, palette.grid.gamma_multiply(0.45)),
            label: palette.disabled_text,
            value: palette.disabled_text,
            indicator: palette.disabled_text,
        };
    }

    let fill = if active {
        mix(palette.control_hover, accent, 0.12)
    } else if focused {
        mix(palette.control_hover, accent, 0.07)
    } else if hovered {
        palette.control_hover
    } else {
        palette.control
    };
    let stroke = if focused {
        Stroke::new(shape::FOCUS_STROKE, accent.gamma_multiply(0.92))
    } else if active {
        Stroke::new(shape::STROKE, accent.gamma_multiply(0.78))
    } else if hovered {
        Stroke::new(shape::STROKE, palette.grid.gamma_multiply(0.68))
    } else {
        Stroke::new(shape::STROKE, palette.grid.gamma_multiply(0.48))
    };

    ControlVisuals {
        fill,
        stroke,
        label: if active {
            accent
        } else if hovered || focused {
            palette.text
        } else {
            palette.text_muted
        },
        value: palette.text,
        indicator: accent.gamma_multiply(if active || focused { 1.0 } else { 0.84 }),
    }
}

pub(crate) fn metrics(ui: &Ui) -> UiMetrics {
    UiMetrics::from_ui(ui)
}

pub(crate) fn title_height(ui: &Ui) -> f32 {
    metrics(ui).points(1.7)
}

pub(crate) fn control_height(ui: &Ui) -> f32 {
    metrics(ui).points(3.45)
}

pub(crate) fn graph_inset(ui: &Ui) -> f32 {
    let metrics = metrics(ui);
    metrics.control_gap().min(metrics.spacing_scale.sm)
}

pub(crate) fn compact_gap(ui: &Ui) -> f32 {
    metrics(ui).points(0.16)
}

pub(crate) fn semantic_palette(settings: ThemeSettings) -> KurvPalette {
    let selected = rgb(settings.background_rgb);
    let primary_source = rgb(settings.primary_rgb);
    let tint = f32::from(settings.tint.min(100)) / 100.0;
    let contrast = f32::from(settings.contrast.clamp(50, 175)) / 100.0;
    let background = mix(selected, primary_source, tint);
    let light = relative_luminance(background) > 0.42;
    let hierarchy = if light { -1.0 } else { 1.0 };
    let well = shade(background, if light { -0.025 } else { -0.09 } * contrast);
    let chrome = shade(background, hierarchy * 0.04 * contrast);
    let surface = shade(background, hierarchy * 0.075 * contrast);
    let control = shade(background, hierarchy * 0.125 * contrast);
    let control_hover = shade(background, hierarchy * 0.18 * contrast);
    let grid = shade(background, hierarchy * 0.25 * contrast);
    let text_target = if light {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    };
    // Grid is a stroke role, not a fill. Including it here flattened the type
    // hierarchy by forcing muted labels almost as bright as held values.
    let surfaces = [background, chrome, surface, control, well, control_hover];
    let text = ensure_contrast_many(
        mix(background, text_target, 0.9),
        &surfaces,
        text_target,
        4.5,
    );
    let text_muted = ensure_contrast_many(
        mix(background, text_target, 0.64),
        &surfaces,
        text_target,
        4.5,
    );
    let accent_surfaces = [well, control];
    let primary = ensure_contrast_many(primary_source, &accent_surfaces, text_target, 3.0);
    let secondary = ensure_contrast_many(
        rgb(settings.secondary_rgb),
        &accent_surfaces,
        text_target,
        3.0,
    );
    let tertiary = ensure_contrast_many(
        rgb(settings.tertiary_rgb),
        &accent_surfaces,
        text_target,
        3.0,
    );
    let disabled = mix(surface, control, 0.48);
    let disabled_text = ensure_contrast_many(
        mix(disabled, text_target, 0.58),
        &[disabled],
        text_target,
        3.0,
    );
    KurvPalette {
        background,
        chrome,
        surface,
        control,
        well,
        control_hover,
        grid,
        text,
        text_muted,
        disabled,
        disabled_text,
        primary,
        pan_shape: tertiary,
        unison: secondary,
        envelope: tertiary,
        danger: ensure_contrast_many(color::DANGER, &accent_surfaces, text_target, 3.0),
    }
}

const fn rgb(value: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(value[0], value[1], value[2])
}

fn shade(color: egui::Color32, amount: f32) -> egui::Color32 {
    if amount >= 0.0 {
        mix(color, egui::Color32::WHITE, amount.clamp(0.0, 1.0))
    } else {
        mix(color, egui::Color32::BLACK, (-amount).clamp(0.0, 1.0))
    }
}

pub(crate) fn semantic() -> KurvPalette {
    ACTIVE_PALETTE.with(Cell::get)
}

pub(crate) fn modulation_source_accent(index: usize) -> egui::Color32 {
    let palette = semantic();
    let base = match index % 4 {
        0 => palette.primary,
        1 => palette.envelope,
        2 => palette.unison,
        _ => palette.danger,
    };
    if index % 8 < 4 {
        base
    } else {
        mix(base, palette.text, 0.26)
    }
}

fn widget_tokens(settings: ThemeSettings) -> WidgetTokens {
    let palette = semantic_palette(settings);
    let mut tokens = ThemeTokens {
        background: palette.background,
        surface: palette.surface,
        surface_high: palette.control_hover,
        text: palette.text,
        text_muted: palette.text_muted,
        primary: palette.primary,
        border: palette.grid,
    }
    .to_widget_tokens();
    tokens.name = "KURV";
    tokens.light_visuals = relative_luminance(palette.background) > 0.42;
    tokens.colors.surface_low = palette.well;
    tokens.colors.surface_dark = palette.well;
    tokens.colors.accent_hover = mix(palette.primary, palette.text, 0.14);
    tokens.colors.track = palette.control;
    tokens.colors.success = tokens.colors.accent;
    tokens.colors.warning = palette.unison;
    tokens.colors.error = palette.danger;
    tokens.colors.disabled = palette.disabled;
    tokens.colors.disabled_text = palette.disabled_text;
    tokens.colors.knob_cap = palette.control;
    tokens.colors.knob_cap_highlight = palette.control_hover;
    tokens.colors.knob_arc_track = palette.grid;
    tokens.colors.knob_arc_value = tokens.colors.accent;
    tokens.colors.knob_marker = palette.text;
    tokens.radius = WidgetRadius {
        panel: 3,
        control: shape::CONTROL_RADIUS as u8,
        tile: 3,
    };
    tokens.spacing = WidgetSpacing {
        xs: space::XS,
        sm: space::SM,
        md: space::MD,
        lg: space::LG,
    };
    tokens.stroke = WidgetStroke {
        control: shape::STROKE,
    };
    tokens
}

pub(crate) fn palette() -> WidgetColors {
    palette_for(ACTIVE_SETTINGS.with(Cell::get))
}

pub(crate) fn palette_for(settings: ThemeSettings) -> WidgetColors {
    widget_tokens(settings).colors
}

pub(crate) fn theme_for(settings: ThemeSettings) -> UiTheme {
    UiTheme::from_tokens(widget_tokens(settings))
}

pub(crate) fn apply_with(ui: &mut Ui, settings: ThemeSettings) {
    ACTIVE_SETTINGS.with(|active| active.set(settings));
    ACTIVE_PALETTE.with(|active| active.set(semantic_palette(settings)));
    let theme = theme_for(settings);
    theme.apply(ui);
    let metrics = theme.metrics();
    let colors = theme.tokens.colors;
    let style = ui.style_mut();
    style.spacing.item_spacing = Vec2::new(metrics.spacing.x, space::XS);
    style.spacing.button_padding = Vec2::new(space::SM, space::XXS);
    style.visuals.panel_fill = colors.background;
    style.visuals.window_fill = colors.background;
    style.visuals.override_text_color = Some(colors.text);
    style.visuals.weak_text_color = Some(colors.muted);
    style.visuals.widgets.noninteractive.bg_fill = colors.surface;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors.muted);
    style.visuals.widgets.inactive.bg_fill = colors.track;
    style.visuals.widgets.inactive.weak_bg_fill = colors.track;
    style.visuals.widgets.inactive.bg_stroke =
        Stroke::new(shape::STROKE, colors.border.gamma_multiply(0.32));
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, colors.text);
    style.visuals.widgets.hovered.bg_fill = colors.surface_high;
    style.visuals.widgets.hovered.weak_bg_fill = colors.surface_high;
    style.visuals.widgets.hovered.bg_stroke =
        Stroke::new(shape::STROKE, colors.border.gamma_multiply(0.68));
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, colors.text);
    let active_fill = mix(colors.surface_high, colors.accent, 0.10);
    style.visuals.widgets.active.bg_fill = active_fill;
    style.visuals.widgets.active.weak_bg_fill = active_fill;
    style.visuals.widgets.active.bg_stroke =
        Stroke::new(shape::STROKE, colors.accent.gamma_multiply(0.72));
    style.visuals.widgets.active.fg_stroke = Stroke::new(shape::STROKE, colors.text);
    style.visuals.widgets.open = style.visuals.widgets.active;
    style.visuals.selection.bg_fill = colors.selected;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, colors.accent);
    style.visuals.extreme_bg_color = colors.background;
    style.visuals.faint_bg_color = colors.surface;
    style.visuals.hyperlink_color = colors.accent;
    style.visuals.warn_fg_color = colors.warning;
    style.visuals.error_fg_color = colors.error;
}

fn ensure_contrast_many(
    mut foreground: egui::Color32,
    backgrounds: &[egui::Color32],
    target: egui::Color32,
    minimum: f32,
) -> egui::Color32 {
    for _ in 0..16 {
        if backgrounds
            .iter()
            .all(|background| contrast_ratio(foreground, *background) >= minimum)
        {
            return foreground;
        }
        foreground = mix(foreground, target, 0.12);
    }
    foreground
}

fn contrast_ratio(left: egui::Color32, right: egui::Color32) -> f32 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

fn relative_luminance(color: egui::Color32) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    channel(color.r()).mul_add(
        0.2126,
        channel(color.g()).mul_add(0.7152, channel(color.b()) * 0.0722),
    )
}
