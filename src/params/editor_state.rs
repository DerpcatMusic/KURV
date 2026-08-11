//! Persisted editor-only state.

use truce::prelude::*;

#[derive(Clone, PartialEq, State)]
pub struct KurvEditorState {
    pub width: u32,
    pub height: u32,
    pub ui_scale: u8,
    pub theme_schema: u8,
    pub theme_preset: u8,
    pub background_red: u8,
    pub background_green: u8,
    pub background_blue: u8,
    pub theme_tint: u8,
    pub theme_contrast: u8,
    pub primary_red: u8,
    pub primary_green: u8,
    pub primary_blue: u8,
    pub secondary_red: u8,
    pub secondary_green: u8,
    pub secondary_blue: u8,
    pub tertiary_red: u8,
    pub tertiary_green: u8,
    pub tertiary_blue: u8,
    pub collapsed_group_ids: Vec<u64>,
    pub collapsed_modulators: u64,
}

impl Default for KurvEditorState {
    fn default() -> Self {
        Self {
            width: 1120,
            height: 720,
            ui_scale: 1,
            theme_schema: 2,
            theme_preset: 0,
            background_red: 18,
            background_green: 20,
            background_blue: 23,
            theme_tint: 8,
            theme_contrast: 100,
            primary_red: 38,
            primary_green: 210,
            primary_blue: 204,
            secondary_red: 245,
            secondary_green: 173,
            secondary_blue: 71,
            tertiary_red: 176,
            tertiary_green: 126,
            tertiary_blue: 247,
            collapsed_group_ids: Vec::new(),
            collapsed_modulators: 0,
        }
    }
}
