//! Shared UI framework pieces for Derpcat audio plugins.

#[cfg(feature = "egui")]
extern crate egui034 as egui;
#[cfg(feature = "egui")]
extern crate egui_taffy034 as egui_taffy;

#[cfg(feature = "egui")]
pub mod layout;
pub mod motion;
#[cfg(feature = "egui")]
pub mod scope;
#[cfg(feature = "egui")]
pub mod theme;
#[cfg(feature = "egui")]
pub mod widgets;
pub mod window;

#[cfg(feature = "egui")]
pub use layout::{SpacingStep, TypeScale, TypeStep, UiMetrics};
#[cfg(feature = "egui")]
pub use theme::{
    DEFAULT_FONT_FAMILIES, FontFamilies, FontFamilySlot, FontFamilySpec, Palette, TextRole, Theme,
    ThemeTokens, Typography, UiTheme,
};
#[cfg(feature = "egui")]
pub use widgets::*;
#[cfg(feature = "egui")]
pub use widgets::{chrome, tokens};
