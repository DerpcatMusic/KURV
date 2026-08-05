use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetTokens {
    pub name: &'static str,
    pub light_visuals: bool,
    pub colors: WidgetColors,
    pub radius: WidgetRadius,
    pub spacing: WidgetSpacing,
    pub stroke: WidgetStroke,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetColors {
    pub background: Color32,
    pub surface: Color32,
    pub surface_low: Color32,
    pub surface_high: Color32,
    pub surface_dark: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_on_dark: Color32,
    pub muted: Color32,
    pub muted_on_dark: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub selected: Color32,
    pub track: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub disabled: Color32,
    pub disabled_text: Color32,
    pub shadow: Color32,
    pub transparent: Color32,
    pub white: Color32,
    pub knob_cap: Color32,
    pub knob_cap_highlight: Color32,
    pub knob_arc_track: Color32,
    pub knob_arc_value: Color32,
    pub knob_marker: Color32,
}

impl WidgetColors {
    pub fn color(self, role: ColorRole) -> Color32 {
        match role {
            ColorRole::Background => self.background,
            ColorRole::Surface => self.surface,
            ColorRole::SurfaceLow => self.surface_low,
            ColorRole::SurfaceHigh => self.surface_high,
            ColorRole::SurfaceDark => self.surface_dark,
            ColorRole::Border => self.border,
            ColorRole::Text => self.text,
            ColorRole::TextOnDark => self.text_on_dark,
            ColorRole::Muted => self.muted,
            ColorRole::MutedOnDark => self.muted_on_dark,
            ColorRole::Accent => self.accent,
            ColorRole::AccentHover => self.accent_hover,
            ColorRole::Selected => self.selected,
            ColorRole::Track => self.track,
            ColorRole::Success => self.success,
            ColorRole::Warning => self.warning,
            ColorRole::Error => self.error,
            ColorRole::Disabled => self.disabled,
            ColorRole::DisabledText => self.disabled_text,
            ColorRole::Shadow => self.shadow,
            ColorRole::Transparent => self.transparent,
            ColorRole::White => self.white,
            ColorRole::KnobCap => self.knob_cap,
            ColorRole::KnobCapHighlight => self.knob_cap_highlight,
            ColorRole::KnobArcTrack => self.knob_arc_track,
            ColorRole::KnobArcValue => self.knob_arc_value,
            ColorRole::KnobMarker => self.knob_marker,
        }
    }

    pub const fn all(self) -> [(&'static str, Color32); 27] {
        [
            ("background", self.background),
            ("surface", self.surface),
            ("surface_low", self.surface_low),
            ("surface_high", self.surface_high),
            ("surface_dark", self.surface_dark),
            ("border", self.border),
            ("text", self.text),
            ("text_on_dark", self.text_on_dark),
            ("muted", self.muted),
            ("muted_on_dark", self.muted_on_dark),
            ("accent", self.accent),
            ("accent_hover", self.accent_hover),
            ("selected", self.selected),
            ("track", self.track),
            ("success", self.success),
            ("warning", self.warning),
            ("error", self.error),
            ("disabled", self.disabled),
            ("disabled_text", self.disabled_text),
            ("shadow", self.shadow),
            ("transparent", self.transparent),
            ("white", self.white),
            ("knob_cap", self.knob_cap),
            ("knob_cap_highlight", self.knob_cap_highlight),
            ("knob_arc_track", self.knob_arc_track),
            ("knob_arc_value", self.knob_arc_value),
            ("knob_marker", self.knob_marker),
        ]
    }

    pub fn selected_fill(self) -> Color32 {
        Color32::from_rgb(30, 36, 42)
    }

    pub fn control_accent(self) -> Color32 {
        self.text
    }

    pub fn control_track(self) -> Color32 {
        lerp_color(self.background, self.border, 0.42)
    }

    pub fn control_shell_fill(self) -> Color32 {
        Color32::from_rgb(214, 215, 218)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorRole {
    Background,
    Surface,
    SurfaceLow,
    SurfaceHigh,
    SurfaceDark,
    Border,
    Text,
    TextOnDark,
    Muted,
    MutedOnDark,
    Accent,
    AccentHover,
    Selected,
    Track,
    Success,
    Warning,
    Error,
    Disabled,
    DisabledText,
    Shadow,
    Transparent,
    White,
    KnobCap,
    KnobCapHighlight,
    KnobArcTrack,
    KnobArcValue,
    KnobMarker,
}

impl WidgetTokens {
    pub fn color(self, role: ColorRole) -> Color32 {
        self.colors.color(role)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetRadius {
    pub panel: u8,
    pub control: u8,
    pub tile: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetSpacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
}

impl WidgetSpacing {
    /// Scale the shared 4/8/12/16 point rhythm from the UI's body size.
    pub const fn from_em(em: f32) -> Self {
        let em = if em.is_finite() && em > 0.01 { em } else { 1.0 };
        let unit = em * 0.25;
        Self {
            xs: unit,
            sm: unit * 2.0,
            md: unit * 3.0,
            lg: unit * 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetStroke {
    pub control: f32,
}

pub const LIGHT_TOKENS: WidgetTokens = WidgetTokens {
    name: "Polar Air",
    light_visuals: true,
    colors: WidgetColors {
        background: Color32::from_rgb(244, 244, 245),
        surface: Color32::from_rgb(226, 227, 229),
        surface_low: Color32::from_rgb(202, 203, 207),
        surface_high: Color32::from_rgb(236, 237, 239),
        surface_dark: Color32::from_rgb(26, 30, 34),
        border: Color32::from_rgb(214, 216, 220),
        text: Color32::from_rgb(37, 43, 48),
        text_on_dark: Color32::from_rgb(244, 246, 248),
        muted: Color32::from_rgb(100, 108, 118),
        muted_on_dark: Color32::from_rgb(166, 172, 178),
        accent: Color32::from_rgb(56, 132, 220),
        accent_hover: Color32::from_rgb(72, 148, 236),
        selected: Color32::from_rgb(189, 196, 203),
        track: Color32::from_rgb(202, 203, 206),
        success: Color32::from_rgb(52, 168, 120),
        warning: Color32::from_rgb(210, 120, 48),
        error: Color32::from_rgb(196, 62, 72),
        disabled: Color32::from_rgb(228, 229, 231),
        disabled_text: Color32::from_rgb(126, 132, 139),
        shadow: Color32::from_black_alpha(28),
        transparent: Color32::TRANSPARENT,
        white: Color32::WHITE,
        knob_cap: Color32::from_rgb(37, 43, 48),
        knob_cap_highlight: Color32::from_rgb(48, 54, 59),
        knob_arc_track: Color32::from_rgb(130, 138, 150),
        knob_arc_value: Color32::from_rgb(37, 43, 48),
        knob_marker: Color32::from_rgb(37, 43, 48),
    },
    radius: WidgetRadius {
        panel: 4,
        control: 3,
        tile: 4,
    },
    spacing: WidgetSpacing {
        xs: 4.0,
        sm: 8.0,
        md: 8.0,
        lg: 12.0,
    },
    stroke: WidgetStroke { control: 1.35 },
};

pub const DARK_TOKENS: WidgetTokens = WidgetTokens {
    name: "Polar Night",
    light_visuals: false,
    colors: WidgetColors {
        background: Color32::from_rgb(7, 10, 12),
        surface: Color32::from_rgb(11, 15, 17),
        surface_low: Color32::from_rgb(7, 10, 12),
        surface_high: Color32::from_rgb(20, 26, 29),
        surface_dark: Color32::from_rgb(7, 10, 12),
        border: Color32::from_rgb(38, 48, 54),
        text: Color32::from_rgb(229, 235, 234),
        text_on_dark: Color32::from_rgb(229, 235, 234),
        muted: Color32::from_rgb(151, 164, 164),
        muted_on_dark: Color32::from_rgb(151, 164, 164),
        accent: Color32::from_rgb(72, 174, 240),
        accent_hover: Color32::from_rgb(92, 194, 255),
        selected: Color32::from_rgb(34, 41, 45),
        track: Color32::from_rgb(19, 24, 27),
        success: Color32::from_rgb(82, 202, 151),
        warning: Color32::from_rgb(238, 126, 74),
        error: Color32::from_rgb(228, 78, 90),
        disabled: Color32::from_rgb(20, 26, 29),
        disabled_text: Color32::from_rgb(92, 105, 108),
        shadow: Color32::from_black_alpha(64),
        transparent: Color32::TRANSPARENT,
        white: Color32::WHITE,
        knob_cap: Color32::from_rgb(37, 43, 48),
        knob_cap_highlight: Color32::from_rgb(48, 54, 59),
        knob_arc_track: Color32::from_rgb(78, 91, 96),
        knob_arc_value: Color32::from_rgb(176, 186, 188),
        knob_marker: Color32::from_rgb(176, 186, 188),
    },
    radius: LIGHT_TOKENS.radius,
    spacing: LIGHT_TOKENS.spacing,
    stroke: LIGHT_TOKENS.stroke,
};

pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_control_tokens_match_polaar_top_row_contract() {
        assert_eq!(
            LIGHT_TOKENS.colors.control_shell_fill(),
            Color32::from_rgb(214, 215, 218)
        );
        assert_eq!(
            LIGHT_TOKENS.colors.selected_fill(),
            Color32::from_rgb(30, 36, 42)
        );
        assert_eq!(
            DARK_TOKENS.colors.control_shell_fill(),
            Color32::from_rgb(214, 215, 218)
        );
        assert_eq!(
            DARK_TOKENS.colors.selected_fill(),
            Color32::from_rgb(30, 36, 42)
        );
    }
}
