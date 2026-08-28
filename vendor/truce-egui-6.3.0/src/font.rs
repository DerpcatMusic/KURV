//! Font loading for egui contexts.

use std::sync::Arc;

/// Load a TrueType font as the default proportional and monospace font.
///
/// Call on an `egui::Context` before rendering. Used by both the live
/// editor and the snapshot renderer.
///
/// # Panics
///
/// Cannot panic in practice - egui's `FontDefinitions::default()`
/// always populates the `Proportional` and `Monospace` families, so
/// the `unwrap()`s succeed. A future egui release that drops either
/// family would surface as a panic here rather than a silent miss.
pub fn apply_font(ctx: &egui::Context, font_data: &'static [u8]) {
    apply_fonts(ctx, font_data, &[]);
}

/// Load a primary font plus named fallbacks before the first egui pass.
pub fn apply_fonts(
    ctx: &egui::Context,
    font_data: &'static [u8],
    fallbacks: &[(&'static str, &'static [u8])],
) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "truce_default".to_owned(),
        Arc::new(egui::FontData::from_static(font_data)),
    );
    for (name, data) in fallbacks {
        fonts.font_data.insert(
            (*name).to_owned(),
            Arc::new(egui::FontData::from_static(data)),
        );
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let family_fonts = fonts.families.get_mut(&family).unwrap();
        family_fonts.insert(0, "truce_default".to_owned());
        for (name, _) in fallbacks {
            family_fonts.push((*name).to_owned());
        }
    }
    ctx.set_fonts(fonts);
}
