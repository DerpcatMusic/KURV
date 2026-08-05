use crate::layout::{TypeStep, UiMetrics};
use crate::widgets::tokens::{DARK_TOKENS, WidgetTokens};
use egui::{Color32, Context, FontFamily as EguiFontFamily, FontId, TextStyle, Ui};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTokens {
    pub background: Color32,
    pub surface: Color32,
    pub surface_high: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub primary: Color32,
    pub border: Color32,
}

impl Default for ThemeTokens {
    fn default() -> Self {
        Self {
            background: Color32::from_rgb(17, 18, 20),
            surface: Color32::from_rgb(28, 30, 34),
            surface_high: Color32::from_rgb(40, 43, 49),
            text: Color32::from_rgb(235, 238, 241),
            text_muted: Color32::from_rgb(150, 158, 166),
            primary: Color32::from_rgb(92, 202, 216),
            border: Color32::from_rgb(70, 76, 84),
        }
    }
}

impl ThemeTokens {
    /// Convert the legacy seven-color palette into the full semantic widget
    /// palette. Existing callers can keep using `ThemeTokens`, while new
    /// editors get all roles required by the shared controls.
    pub fn to_widget_tokens(self) -> WidgetTokens {
        let mut tokens = DARK_TOKENS;
        tokens.colors.background = self.background;
        tokens.colors.surface = self.surface;
        tokens.colors.surface_low = self.background;
        tokens.colors.surface_high = self.surface_high;
        tokens.colors.surface_dark = self.background;
        tokens.colors.border = self.border;
        tokens.colors.text = self.text;
        tokens.colors.text_on_dark = self.text;
        tokens.colors.muted = self.text_muted;
        tokens.colors.muted_on_dark = self.text_muted;
        tokens.colors.accent = self.primary;
        tokens.colors.accent_hover = mix(self.primary, Color32::WHITE, 0.12);
        tokens.colors.selected = mix(self.surface, self.primary, 0.16);
        tokens.colors.track = mix(self.background, self.border, 0.42);
        tokens.colors.knob_arc_value = self.primary;
        tokens.colors.knob_marker = self.primary;
        tokens
    }

    pub fn to_ui_theme(self) -> UiTheme {
        UiTheme::from_tokens(self.to_widget_tokens())
    }
}

/// A font family reference owned by the consumer's font registry.
///
/// Plugcat deliberately does not embed or name product assets. Consumers can
/// register Barlow, Martian Mono, Phosphor, or any other family and bind it to
/// one of the semantic slots in [`FontFamilies`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamilySpec {
    Proportional,
    Monospace,
    Named(&'static str),
}

impl FontFamilySpec {
    pub const fn named(name: &'static str) -> Self {
        Self::Named(name)
    }

    pub fn resolve(self) -> EguiFontFamily {
        match self {
            Self::Proportional => EguiFontFamily::Proportional,
            Self::Monospace => EguiFontFamily::Monospace,
            Self::Named(name) => EguiFontFamily::Name(name.into()),
        }
    }
}

/// Semantic family slots used by shared controls and plugin-specific panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamilySlot {
    Proportional,
    Semibold,
    Display,
    Mono,
    NumericWeight,
    Icon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontFamilies {
    pub proportional: FontFamilySpec,
    pub semibold: FontFamilySpec,
    pub display: FontFamilySpec,
    pub mono: FontFamilySpec,
    pub numeric_weight: FontFamilySpec,
    pub icon: FontFamilySpec,
}

pub const DEFAULT_FONT_FAMILIES: FontFamilies = FontFamilies {
    proportional: FontFamilySpec::Proportional,
    semibold: FontFamilySpec::Proportional,
    display: FontFamilySpec::Proportional,
    mono: FontFamilySpec::Monospace,
    numeric_weight: FontFamilySpec::Monospace,
    icon: FontFamilySpec::Proportional,
};

impl FontFamilies {
    pub const fn with_proportional(mut self, family: FontFamilySpec) -> Self {
        self.proportional = family;
        self
    }

    pub const fn with_semibold(mut self, family: FontFamilySpec) -> Self {
        self.semibold = family;
        self
    }

    pub const fn with_display(mut self, family: FontFamilySpec) -> Self {
        self.display = family;
        self
    }

    pub const fn with_mono(mut self, family: FontFamilySpec) -> Self {
        self.mono = family;
        self
    }

    pub const fn with_numeric_weight(mut self, family: FontFamilySpec) -> Self {
        self.numeric_weight = family;
        self
    }

    pub const fn with_icon(mut self, family: FontFamilySpec) -> Self {
        self.icon = family;
        self
    }

    pub const fn family(self, slot: FontFamilySlot) -> FontFamilySpec {
        match slot {
            FontFamilySlot::Proportional => self.proportional,
            FontFamilySlot::Semibold => self.semibold,
            FontFamilySlot::Display => self.display,
            FontFamilySlot::Mono => self.mono,
            FontFamilySlot::NumericWeight => self.numeric_weight,
            FontFamilySlot::Icon => self.icon,
        }
    }
}

impl Default for FontFamilies {
    fn default() -> Self {
        DEFAULT_FONT_FAMILIES
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub caption: f32,
    pub label: f32,
    pub body: f32,
    pub title: f32,
    pub numeric: f32,
    pub families: FontFamilies,
}

impl Typography {
    pub const fn from_em(em: f32) -> Self {
        Self::from_metrics(UiMetrics::from_em(em))
    }

    pub const fn from_metrics(metrics: UiMetrics) -> Self {
        Self::from_metrics_with_families(metrics, DEFAULT_FONT_FAMILIES)
    }

    pub const fn from_metrics_with_families(metrics: UiMetrics, families: FontFamilies) -> Self {
        let scale = metrics.type_scale;
        Self {
            caption: scale.caption,
            label: scale.label,
            body: scale.body,
            title: scale.title,
            numeric: scale.numeric,
            families,
        }
    }

    pub const fn with_families(mut self, families: FontFamilies) -> Self {
        self.families = families;
        self
    }

    pub const fn family(self, slot: FontFamilySlot) -> FontFamilySpec {
        self.families.family(slot)
    }

    pub fn font(&self, role: TextRole) -> FontId {
        self.font_for(role.family_slot(), role.size(*self))
    }

    pub fn font_for(&self, slot: FontFamilySlot, size: f32) -> FontId {
        FontId::new(size.max(0.0), self.family(slot).resolve())
    }

    pub fn apply(&self, ctx: &Context) {
        ctx.global_style_mut(|style| {
            style
                .text_styles
                .insert(TextStyle::Small, self.font(TextRole::Caption));
            style
                .text_styles
                .insert(TextStyle::Button, self.font(TextRole::Label));
            style
                .text_styles
                .insert(TextStyle::Body, self.font(TextRole::Body));
            style
                .text_styles
                .insert(TextStyle::Heading, self.font(TextRole::Title));
            style
                .text_styles
                .insert(TextStyle::Monospace, self.font(TextRole::Numeric));
        });
    }
}

impl Default for Typography {
    fn default() -> Self {
        Self::from_em(14.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Caption,
    Label,
    Body,
    Title,
    Numeric,
}

impl TextRole {
    pub const fn type_step(self) -> TypeStep {
        match self {
            Self::Caption => TypeStep::Caption,
            Self::Label => TypeStep::Label,
            Self::Body => TypeStep::Body,
            Self::Title => TypeStep::Title,
            Self::Numeric => TypeStep::Numeric,
        }
    }

    pub const fn family_slot(self) -> FontFamilySlot {
        match self {
            Self::Caption | Self::Body => FontFamilySlot::Proportional,
            Self::Label => FontFamilySlot::Semibold,
            Self::Title => FontFamilySlot::Display,
            Self::Numeric => FontFamilySlot::NumericWeight,
        }
    }

    const fn size(self, typography: Typography) -> f32 {
        match self {
            Self::Caption => typography.caption,
            Self::Label => typography.label,
            Self::Body => typography.body,
            Self::Title => typography.title,
            Self::Numeric => typography.numeric,
        }
    }
}

/// The runtime theme passed to shared widgets.
///
/// `WidgetTokens` remains the low-level compatibility seam used by existing
/// widget functions. New editors can carry one `UiTheme` per frame and pass
/// `theme.tokens()` to those functions without relying on global light/dark
/// state or plugin-specific parameter types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiTheme {
    pub tokens: WidgetTokens,
    pub metrics: UiMetrics,
    pub typography: Typography,
}

impl UiTheme {
    pub const fn new(mut tokens: WidgetTokens, metrics: UiMetrics, typography: Typography) -> Self {
        tokens.spacing = metrics.widget_spacing();
        Self {
            tokens,
            metrics,
            typography,
        }
    }

    pub const fn from_tokens(tokens: WidgetTokens) -> Self {
        let metrics = UiMetrics::from_em(14.0);
        Self::new(tokens, metrics, Typography::from_metrics(metrics))
    }

    pub fn from_ui(ui: &Ui, tokens: WidgetTokens) -> Self {
        let metrics = UiMetrics::from_ui(ui);
        Self::new(tokens, metrics, Typography::from_metrics(metrics))
    }

    pub fn tokens(&self) -> &WidgetTokens {
        &self.tokens
    }

    pub fn palette(&self) -> &WidgetTokens {
        &self.tokens
    }

    pub const fn metrics(&self) -> UiMetrics {
        self.metrics
    }

    pub const fn typography(&self) -> Typography {
        self.typography
    }

    pub fn color(&self, role: crate::widgets::tokens::ColorRole) -> Color32 {
        self.tokens.color(role)
    }

    pub fn with_metrics(mut self, metrics: UiMetrics) -> Self {
        self.metrics = metrics;
        self.tokens.spacing = metrics.widget_spacing();
        self.typography = Typography::from_metrics_with_families(metrics, self.typography.families);
        self
    }

    pub fn with_typography(mut self, typography: Typography) -> Self {
        self.typography = typography;
        self
    }

    pub fn apply_typography(&self, ctx: &Context) {
        self.typography.apply(ctx);
    }

    /// Apply typography and geometry together so a consumer cannot accidentally
    /// use one scale for text and another for controls.
    pub fn apply(&self, ui: &mut Ui) {
        self.typography.apply(ui.ctx());
        self.metrics.apply(ui);
    }
}

impl AsRef<WidgetTokens> for UiTheme {
    fn as_ref(&self) -> &WidgetTokens {
        &self.tokens
    }
}

impl std::ops::Deref for UiTheme {
    type Target = WidgetTokens;

    fn deref(&self) -> &Self::Target {
        &self.tokens
    }
}

impl Default for UiTheme {
    fn default() -> Self {
        Self::from_tokens(DARK_TOKENS)
    }
}

/// Short name for callers that do not need to distinguish the implementation
/// from the public theme concept.
pub type Theme = UiTheme;

pub type Palette = WidgetTokens;

pub fn alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    let t = amount.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 * inv + b.r() as f32 * t).round() as u8,
        (a.g() as f32 * inv + b.g() as f32 * t).round() as u8,
        (a.b() as f32 * inv + b.b() as f32 * t).round() as u8,
        (a.a() as f32 * inv + b.a() as f32 * t).round() as u8,
    )
}

pub fn shade(color: Color32, amount: f32) -> Color32 {
    if amount >= 0.0 {
        mix(color, Color32::WHITE, amount)
    } else {
        mix(color, Color32::BLACK, -amount)
    }
}
