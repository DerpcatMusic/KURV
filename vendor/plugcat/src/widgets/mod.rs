//! Shared egui controls for Dolmen Gate audio plugins.
//!
//! This crate is WASM-safe (egui only) so widgets can be iterated in the browser
//! via `widget-preview/` without building a full CLAP/VST3 target.
//!
//! # Modules
//! - [`button`] — tokenized command buttons
//! - [`chrome`] — shared drawing vocabulary for plugin controls
//! - [`knob`] — tactile rotary controls
//! - [`meter`] — mix and dB meter composites
//! - [`segmented`] — grouped segmented buttons
//! - [`slider`] — tokenized horizontal sliders
//! - [`surface`] — bolted panel surfaces
//! - [`toggle`] — tactile power toggle switches
//! - [`tokens`] — shared design tokens

pub mod button;
pub mod chrome;
pub mod knob;
pub mod meter;
pub mod segmented;
pub mod slider;
pub mod surface;
pub mod toggle;
pub mod tokens;

pub use button::{
    ButtonSize, ButtonTone, button, button_with_tokens, dropdown_button_rect_with_tokens,
    icon_button_rect_with_tokens,
};
pub use chrome::{
    draw_flat_field_shell, draw_group_shell, draw_segment_hover, draw_segment_pressed,
    draw_slider_knob, draw_slider_track, draw_vertical_separator, section_caption_y,
    segment_label_font, segment_rect, segment_rounding, segment_text_color,
};
pub use knob::{
    tactile_knob, tactile_knob_db, tactile_knob_db_with_tokens, tactile_knob_display,
    tactile_knob_display_wide_with_tokens, tactile_knob_display_with_tokens,
    tactile_knob_field_inset, tactile_knob_layout_size, tactile_knob_readout_clearance,
    tactile_knob_with_tokens,
};
pub use meter::{
    DbMeterRange, MixDbMeterValues, StereoDbMeterValues, mix_db_meter, mix_db_meter_mix_rect,
    mix_db_meter_track_rect, mix_db_meter_with_tokens, paint_mix_db_meter, paint_stereo_db_meter,
    stereo_db_meter_track_rect,
};
pub use segmented::{
    segmented, segmented_rect_custom_with_tokens, segmented_rect_with_tokens, segmented_with_tokens,
};
pub use slider::{slider, slider_with_tokens};
pub use surface::{
    paint_surface_chrome, surface, surface_with_margin_with_theme, surface_with_margin_with_tokens,
    surface_with_theme, surface_with_tokens,
};
pub use toggle::{
    PowerToggleOrientation, PowerToggleSize, power_toggle, power_toggle_rect_oriented_with_tokens,
    power_toggle_rect_with_tokens, power_toggle_with_tokens,
};
pub use tokens::{
    ColorRole, DARK_TOKENS, LIGHT_TOKENS, WidgetColors, WidgetRadius, WidgetSpacing, WidgetStroke,
    WidgetTokens,
};
