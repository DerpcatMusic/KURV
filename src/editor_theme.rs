//! Shared visual tokens for KURV's egui editor.

use std::cell::Cell;
use std::fs;
use std::io;
use std::path::PathBuf;

use egui::{Stroke, Ui, Vec2};
use plugcat::layout::UiMetrics;
use plugcat::theme::{ThemeTokens, UiTheme, mix};
use plugcat::widgets::{WidgetColors, WidgetRadius, WidgetSpacing, WidgetStroke, WidgetTokens};

use crate::editor_presets::{atomic_write, sanitize_name, user_data_directory};

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

#[derive(Clone, Debug)]
struct NamedTheme {
    name: String,
    settings: ThemeSettings,
}

#[derive(Clone, Debug)]
pub(crate) struct ThemeLibrary {
    path: PathBuf,
    active: String,
    themes: Vec<NamedTheme>,
}

impl ThemeLibrary {
    pub(crate) fn load(initial: ThemeSettings) -> io::Result<Self> {
        let path = user_data_directory()?.join("Themes").join("themes.json");
        match fs::read(&path) {
            Ok(bytes) => Self::decode(path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let library = Self {
                    path,
                    active: "Custom".to_owned(),
                    themes: vec![NamedTheme {
                        name: "Custom".to_owned(),
                        settings: initial,
                    }],
                };
                library.write()?;
                Ok(library)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn active_name(&self) -> &str {
        &self.active
    }

    pub(crate) fn names(&self) -> Vec<String> {
        BUILTIN_THEMES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .chain(self.themes.iter().map(|theme| theme.name.clone()))
            .collect()
    }

    pub(crate) fn active_settings(&self) -> Option<ThemeSettings> {
        self.find(&self.active)
    }

    pub(crate) fn select(&mut self, name: &str) -> io::Result<ThemeSettings> {
        let settings = self
            .find(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "theme was not found"))?;
        self.active = name.to_owned();
        self.write()?;
        Ok(settings)
    }

    pub(crate) fn update_active(&mut self, settings: ThemeSettings) {
        if BUILTIN_THEMES
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(&self.active))
        {
            self.active = "Custom".to_owned();
        }
        if let Some(theme) = self
            .themes
            .iter_mut()
            .find(|theme| theme.name.eq_ignore_ascii_case(&self.active))
        {
            theme.settings = settings;
        } else {
            self.themes.push(NamedTheme {
                name: self.active.clone(),
                settings,
            });
        }
    }

    pub(crate) fn save_as(
        &mut self,
        requested_name: &str,
        settings: ThemeSettings,
    ) -> io::Result<()> {
        let name = sanitize_name(requested_name)?;
        if BUILTIN_THEMES
            .iter()
            .any(|(builtin, _)| builtin.eq_ignore_ascii_case(&name))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "built-in theme names are reserved",
            ));
        }
        self.active = name;
        self.update_active(settings);
        self.write()
    }

    pub(crate) fn write(&self) -> io::Result<()> {
        let themes: Vec<_> = self
            .themes
            .iter()
            .map(|theme| {
                serde_json::json!({
                    "name": theme.name,
                    "background": theme.settings.background_rgb,
                    "tint": theme.settings.tint,
                    "contrast": theme.settings.contrast,
                    "primary": theme.settings.primary_rgb,
                    "secondary": theme.settings.secondary_rgb,
                    "tertiary": theme.settings.tertiary_rgb,
                })
            })
            .collect();
        let document = serde_json::json!({
            "version": 1,
            "active": self.active,
            "themes": themes,
        });
        let bytes = serde_json::to_vec_pretty(&document).map_err(io::Error::other)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("theme path has no parent"))?;
        fs::create_dir_all(parent)?;
        atomic_write(&self.path, &bytes)
    }

    fn find(&self, name: &str) -> Option<ThemeSettings> {
        BUILTIN_THEMES
            .iter()
            .find(|(builtin, _)| builtin.eq_ignore_ascii_case(name))
            .map(|(_, settings)| *settings)
            .or_else(|| {
                self.themes
                    .iter()
                    .find(|theme| theme.name.eq_ignore_ascii_case(name))
                    .map(|theme| theme.settings)
            })
    }

    fn decode(path: PathBuf, bytes: &[u8]) -> io::Result<Self> {
        let document: serde_json::Value =
            serde_json::from_slice(bytes).map_err(io::Error::other)?;
        if document.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported theme library version",
            ));
        }
        let active = document
            .get("active")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing active theme"))?
            .to_owned();
        let mut themes = Vec::new();
        for value in document
            .get("themes")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing themes"))?
        {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "theme has no name"))?;
            let name = sanitize_name(name)?;
            if BUILTIN_THEMES
                .iter()
                .any(|(builtin, _)| builtin.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            themes.push(NamedTheme {
                name,
                settings: ThemeSettings {
                    background_rgb: decode_rgb(value, "background")?,
                    tint: decode_u8(value, "tint")?.min(100),
                    contrast: decode_u8(value, "contrast")?.clamp(50, 175),
                    primary_rgb: decode_rgb(value, "primary")?,
                    secondary_rgb: decode_rgb(value, "secondary")?,
                    tertiary_rgb: decode_rgb(value, "tertiary")?,
                },
            });
        }
        let library = Self {
            path,
            active,
            themes,
        };
        if library.active_settings().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "active theme was not found",
            ));
        }
        Ok(library)
    }
}

fn decode_rgb(value: &serde_json::Value, key: &str) -> io::Result<[u8; 3]> {
    let channels = value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .filter(|channels| channels.len() == 3)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid theme color"))?;
    let mut rgb = [0_u8; 3];
    for (output, channel) in rgb.iter_mut().zip(channels) {
        *output = u8::try_from(channel.as_u64().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid theme color channel")
        })?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "theme color overflow"))?;
    }
    Ok(rgb)
}

fn decode_u8(value: &serde_json::Value, key: &str) -> io::Result<u8> {
    u8::try_from(
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid theme value"))?,
    )
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "theme value overflow"))
}

thread_local! {
    static ACTIVE_SETTINGS: Cell<ThemeSettings> = Cell::new(ThemeSettings::default());
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

    pub(crate) const XS: f32 = 6.0;
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

    pub(crate) const CAPTION_SIZE: f32 = 8.5;
    pub(crate) const LABEL_SIZE: f32 = 9.5;
    pub(crate) const TITLE_SIZE: f32 = 11.0;
    pub(crate) const VALUE_SIZE: f32 = 10.5;

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

pub(crate) fn metrics(ui: &Ui) -> UiMetrics {
    UiMetrics::from_ui(ui)
}

pub(crate) fn title_height(ui: &Ui) -> f32 {
    metrics(ui).points(1.75).clamp(22.0, 28.0)
}

pub(crate) fn control_height(ui: &Ui) -> f32 {
    metrics(ui).points(3.6).clamp(48.0, 56.0)
}

pub(crate) fn knob_size(ui: &Ui) -> f32 {
    metrics(ui).knob_size().clamp(56.0, 68.0)
}

pub(crate) fn graph_inset(ui: &Ui) -> f32 {
    metrics(ui).control_gap().clamp(space::XS, space::SM)
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
    let chrome = shade(background, hierarchy * 0.045 * contrast);
    let surface = shade(background, hierarchy * 0.085 * contrast);
    let control = shade(background, hierarchy * 0.15 * contrast);
    let control_hover = shade(background, hierarchy * 0.215 * contrast);
    let grid = shade(background, hierarchy * 0.31 * contrast);
    let text_target = if light {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    };
    let surfaces = [background, chrome, surface, control, well];
    let text = ensure_contrast_many(
        mix(background, text_target, 0.94),
        &surfaces,
        text_target,
        4.5,
    );
    let text_muted = ensure_contrast_many(
        mix(background, text_target, 0.68),
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
    semantic_palette(ACTIVE_SETTINGS.with(Cell::get))
}

pub(crate) fn readable_text(background: egui::Color32) -> egui::Color32 {
    if contrast_ratio(egui::Color32::BLACK, background)
        >= contrast_ratio(egui::Color32::WHITE, background)
    {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
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
    tokens.colors.surface_low = palette.well;
    tokens.colors.surface_dark = palette.well;
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
        panel: 4,
        control: 3,
        tile: 4,
    };
    tokens.spacing = WidgetSpacing {
        xs: space::XS,
        sm: space::SM,
        md: space::MD,
        lg: space::LG,
    };
    tokens.stroke = WidgetStroke { control: 1.0 };
    tokens
}

pub(crate) fn palette() -> WidgetColors {
    palette_for(ACTIVE_SETTINGS.with(Cell::get))
}

pub(crate) fn palette_for(settings: ThemeSettings) -> WidgetColors {
    widget_tokens(settings).colors
}

pub(crate) fn warning_fill() -> egui::Color32 {
    let colors = palette();
    mix(colors.surface_low, colors.warning, 0.18)
}

pub(crate) fn theme_for(settings: ThemeSettings) -> UiTheme {
    UiTheme::from_tokens(widget_tokens(settings))
}

pub(crate) fn apply_with(ui: &mut Ui, settings: ThemeSettings) {
    ACTIVE_SETTINGS.with(|active| active.set(settings));
    let theme = theme_for(settings);
    theme.apply(ui);
    let metrics = theme.metrics();
    let colors = theme.tokens.colors;
    let style = ui.style_mut();
    style.spacing.item_spacing = metrics.spacing;
    style.spacing.button_padding = Vec2::new(space::SM, space::XS);
    style.visuals.panel_fill = colors.background;
    style.visuals.window_fill = colors.background;
    style.visuals.override_text_color = Some(colors.text);
    style.visuals.weak_text_color = Some(colors.muted);
    style.visuals.widgets.noninteractive.bg_fill = colors.surface;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors.muted);
    style.visuals.widgets.inactive.bg_fill = colors.track;
    style.visuals.widgets.inactive.weak_bg_fill = colors.track;
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, colors.text);
    style.visuals.widgets.hovered.bg_fill = colors.surface_high;
    style.visuals.widgets.hovered.weak_bg_fill = colors.surface_high;
    style.visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, colors.text);
    style.visuals.widgets.active.bg_fill = colors.selected;
    style.visuals.widgets.active.weak_bg_fill = colors.selected;
    style.visuals.widgets.active.bg_stroke = Stroke::NONE;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, colors.accent);
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
