//! Persisted editor-only state.

use truce::prelude::*;

#[derive(Clone, Default, PartialEq, State)]
pub struct GroupAccent {
    pub group_id: u64,
    pub accent: u8,
}

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
    pub group_accents: Vec<GroupAccent>,
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
            group_accents: Vec::new(),
            collapsed_modulators: 0,
        }
    }
}

impl KurvEditorState {
    pub(crate) fn group_accent_index(&self, group_id: u64, fallback: usize) -> usize {
        self.group_accents
            .iter()
            .find(|accent| accent.group_id == group_id)
            .map_or(fallback, |accent| usize::from(accent.accent))
    }

    pub(crate) fn set_group_accent(&mut self, group_id: u64, accent: usize) {
        if let Some(stored) = self
            .group_accents
            .iter_mut()
            .find(|stored| stored.group_id == group_id)
        {
            stored.accent = accent as u8;
        } else {
            self.group_accents.push(GroupAccent {
                group_id,
                accent: accent as u8,
            });
        }
    }
}
