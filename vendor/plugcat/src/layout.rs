use crate::theme::TextRole;
use crate::widgets::WidgetSpacing;
use egui::{Rect, Ui, Vec2, pos2, vec2};

/// The shared four-step spacing vocabulary, scaled from a 4 point design unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpacingStep {
    Xs,
    Sm,
    Md,
    Lg,
}

/// Named text sizes shared by chrome, controls, and domain panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeStep {
    Micro,
    Caption,
    Label,
    Body,
    Title,
    Display,
    Numeric,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    pub micro: f32,
    pub caption: f32,
    pub label: f32,
    pub body: f32,
    pub title: f32,
    pub display: f32,
    pub numeric: f32,
}

impl TypeScale {
    /// The reference scale is 16 logical points for body text.
    pub const fn from_em(em: f32) -> Self {
        let em = valid_em(em);
        Self {
            micro: em * 0.625,
            caption: em * 0.6875,
            label: em * 0.75,
            body: em,
            title: em * 1.125,
            display: em * 1.375,
            numeric: em * 0.875,
        }
    }

    pub const fn size(self, step: TypeStep) -> f32 {
        match step {
            TypeStep::Micro => self.micro,
            TypeStep::Caption => self.caption,
            TypeStep::Label => self.label,
            TypeStep::Body => self.body,
            TypeStep::Title => self.title,
            TypeStep::Display => self.display,
            TypeStep::Numeric => self.numeric,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiMetrics {
    pub em: f32,
    pub unit: f32,
    pub spacing: Vec2,
    pub spacing_scale: WidgetSpacing,
    pub type_scale: TypeScale,
    pub available: Vec2,
    pub control_radius: f32,
    pub button_radius: f32,
    pub toolbar_pad: f32,
    pub toolbar_height: f32,
    pub toolbar_icon_width: f32,
    pub toolbar_gap: f32,
    pub gap_x: f32,
    pub gap_y: f32,
    pub output_trim_min_track: f32,
    pub settings_slider_width: f32,
    pub settings_slider_height: f32,
    pub fx_card_gap: f32,
    pub chip_height: f32,
    pub chip_min_width: f32,
    pub chip_pad_x: f32,
    pub selection_edge_hit: f32,
    pub zoom_thumb_radius: f32,
}

impl UiMetrics {
    pub fn from_ui(ui: &Ui) -> Self {
        let metrics = Self::from_em(em(ui));
        Self {
            spacing: metrics.spacing,
            available: ui.available_size(),
            ..metrics
        }
    }

    pub const fn from_em(em: f32) -> Self {
        let em = valid_em(em);
        let unit = em * 0.25;
        let spacing_scale = WidgetSpacing::from_em(em);
        let type_scale = TypeScale::from_em(em);
        let toolbar_pad = unit * 1.5;
        let toolbar_icon_width = (em * 1.35 + toolbar_pad * 2.0).max(em * 2.3);
        Self {
            em,
            unit,
            spacing: Vec2::splat(spacing_scale.md),
            spacing_scale,
            type_scale,
            available: Vec2::ZERO,
            control_radius: unit * 1.25,
            button_radius: unit * 2.0,
            toolbar_pad,
            toolbar_height: toolbar_icon_width,
            toolbar_icon_width,
            toolbar_gap: unit * 1.5,
            gap_x: unit,
            gap_y: unit,
            output_trim_min_track: em * 4.5,
            settings_slider_width: em * 7.375,
            settings_slider_height: em * 1.125,
            fx_card_gap: unit * 2.5,
            chip_height: em * 1.625,
            chip_min_width: em * 2.0,
            chip_pad_x: unit * 2.0,
            selection_edge_hit: (em * 0.65).max(12.0),
            zoom_thumb_radius: unit * 0.8,
        }
    }

    pub fn points(self, em_units: f32) -> f32 {
        self.em * em_units.max(0.0)
    }

    pub const fn space(self, step: SpacingStep) -> f32 {
        match step {
            SpacingStep::Xs => self.spacing_scale.xs,
            SpacingStep::Sm => self.spacing_scale.sm,
            SpacingStep::Md => self.spacing_scale.md,
            SpacingStep::Lg => self.spacing_scale.lg,
        }
    }

    pub const fn type_size(self, role: TextRole) -> f32 {
        self.type_scale.size(role.type_step())
    }

    pub const fn widget_spacing(self) -> WidgetSpacing {
        self.spacing_scale
    }

    /// Apply the shared geometry defaults to one egui layout scope.
    pub fn apply(self, ui: &mut Ui) {
        let spacing = ui.spacing_mut();
        spacing.item_spacing = vec2(self.space(SpacingStep::Md), self.space(SpacingStep::Md));
        spacing.button_padding = vec2(self.space(SpacingStep::Sm), self.space(SpacingStep::Xs));
        spacing.interact_size = vec2(self.control_height(), self.control_height());
    }

    pub fn control_height(self) -> f32 {
        self.points(2.25)
    }

    pub fn compact_control_height(self) -> f32 {
        self.points(1.8)
    }

    pub fn panel_padding(self) -> f32 {
        self.points(1.25)
    }

    pub fn section_gap(self) -> f32 {
        self.points(1.5)
    }

    pub fn control_gap(self) -> f32 {
        self.points(0.75)
    }

    pub fn knob_size(self) -> f32 {
        self.points(4.5)
    }

    pub fn icon_size(self) -> f32 {
        self.points(1.15)
    }

    pub fn scaled(self, factor: f32) -> Self {
        let mut metrics = Self::from_em(self.em * valid_em(factor));
        metrics.available = self.available;
        metrics
    }
}

impl Default for UiMetrics {
    fn default() -> Self {
        Self::from_em(14.0)
    }
}

pub(crate) const fn valid_em(em: f32) -> f32 {
    if em.is_finite() && em > 0.01 { em } else { 1.0 }
}

pub fn em(ui: &Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellBands {
    pub nav: Rect,
    pub toolbar: Rect,
    pub workspace: Rect,
    pub status: Rect,
}

pub fn shell_bands(rect: Rect, heights: [f32; 3], spacing: f32) -> Option<ShellBands> {
    let [nav_h, toolbar_h, status_h] = heights;
    let required = nav_h + toolbar_h + status_h + spacing * 3.0;
    if rect.height() < required || rect.width() <= 0.0 {
        return None;
    }

    let nav = Rect::from_min_max(rect.left_top(), pos2(rect.right(), rect.top() + nav_h));
    let toolbar_top = nav.bottom() + spacing;
    let toolbar = Rect::from_min_max(
        pos2(rect.left(), toolbar_top),
        pos2(rect.right(), toolbar_top + toolbar_h),
    );
    let status = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - status_h),
        rect.right_bottom(),
    );
    let workspace = Rect::from_min_max(
        pos2(rect.left(), toolbar.bottom() + spacing),
        pos2(rect.right(), status.top() - spacing),
    );

    Some(ShellBands {
        nav,
        toolbar,
        workspace,
        status,
    })
}

pub fn padded_child(rect: Rect, padding: f32) -> Option<Rect> {
    let child = rect.shrink(padding.max(0.0));
    (child.width() > 0.0 && child.height() > 0.0).then_some(child)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeWorkspaceConfig {
    pub meter_width: f32,
    pub dock_height: f32,
    pub gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeWorkspaceRects {
    pub scope: Rect,
    pub meter: Rect,
    pub dock: Rect,
}

pub fn scope_workspace(rect: Rect, config: ScopeWorkspaceConfig) -> Option<ScopeWorkspaceRects> {
    let gap = config.gap.max(0.0);
    let meter_width = config.meter_width.max(0.0);
    let dock_height = config.dock_height.max(0.0);
    if rect.width() <= meter_width + gap || rect.height() <= dock_height + gap {
        return None;
    }

    let dock = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - dock_height),
        rect.right_bottom(),
    );
    let top = Rect::from_min_max(rect.left_top(), pos2(rect.right(), dock.top() - gap));
    let meter = Rect::from_min_max(
        pos2(top.right() - meter_width, top.top()),
        top.right_bottom(),
    );
    let scope = Rect::from_min_max(top.left_top(), pos2(meter.left() - gap, top.bottom()));

    Some(ScopeWorkspaceRects { scope, meter, dock })
}

#[cfg(feature = "egui")]
pub fn taffy_panel<R>(
    ui: &mut Ui,
    _id_source: impl std::hash::Hash,
    _style: egui_taffy::taffy::Style,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    add_contents(ui)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_layout_creates_fixed_bands_and_fill_workspace() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 200.0));
        let bands = shell_bands(rect, [20.0, 30.0, 10.0], 5.0).unwrap();

        assert_eq!(bands.nav.height(), 20.0);
        assert_eq!(bands.toolbar.height(), 30.0);
        assert_eq!(bands.status.height(), 10.0);
        assert_eq!(bands.workspace.top(), 60.0);
        assert_eq!(bands.workspace.bottom(), 185.0);
    }

    #[test]
    fn scope_workspace_reserves_meter_and_dock() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 200.0));
        let layout = scope_workspace(
            rect,
            ScopeWorkspaceConfig {
                meter_width: 40.0,
                dock_height: 30.0,
                gap: 10.0,
            },
        )
        .unwrap();

        assert_eq!(layout.dock.height(), 30.0);
        assert_eq!(layout.meter.width(), 40.0);
        assert_eq!(layout.scope.right(), 250.0);
    }
}
