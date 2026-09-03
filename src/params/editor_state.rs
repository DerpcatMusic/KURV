//! Persisted editor-only state.

use truce::prelude::*;

#[derive(Clone, Default, PartialEq, State)]
pub struct GroupAccent {
    pub group_id: u64,
    // Historical palette index retained for positional state compatibility.
    pub accent: u8,
    pub custom: bool,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Default, PartialEq, State)]
pub struct GroupName {
    pub group_id: u64,
    pub name: String,
}

#[derive(Clone, PartialEq, State)]
pub struct KurvEditorState {
    /// Legacy fields retained so older persist blobs still deserialize.
    /// Live window size is host-owned and lives in a skipped atomic.
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
    pub group_names: Vec<GroupName>,
    pub collapsed_modulators: u64,
    /// Keep all visible modulation routes drawn between their source and destination.
    pub persistent_modulation_cables: bool,
    /// RESYNTH pitch-map / reconstruction detail. 0 Eco … 3 Ultra.
    pub resynth_quality: u8,
    /// How strongly the generator/modulator split changes card density. 0 … 100.
    pub responsive_density: u8,
    /// Trade exact audio-rate route-depth evaluation for the faster block renderer.
    pub fast_audio_rate_modulation: bool,
}

impl Default for KurvEditorState {
    fn default() -> Self {
        Self {
            width: 1120,
            height: 720,
            ui_scale: 1,
            theme_schema: 2,
            theme_preset: 0,
            background_red: 38,
            background_green: 38,
            background_blue: 38,
            theme_tint: 0,
            theme_contrast: 100,
            primary_red: 255,
            primary_green: 182,
            primary_blue: 0,
            secondary_red: 44,
            secondary_green: 194,
            secondary_blue: 246,
            tertiary_red: 255,
            tertiary_green: 173,
            tertiary_blue: 124,
            collapsed_group_ids: Vec::new(),
            group_accents: Vec::new(),
            group_names: Vec::new(),
            collapsed_modulators: 0,
            persistent_modulation_cables: false,
            resynth_quality: crate::oscillators::ResynthQuality::Standard as u8,
            responsive_density: 100,
            fast_audio_rate_modulation: false,
        }
    }
}

impl KurvEditorState {
    pub(crate) fn group_accent_color(
        &self,
        group_id: u64,
        fallback: egui::Color32,
        palette: &[egui::Color32],
    ) -> egui::Color32 {
        self.group_accents
            .iter()
            .find(|accent| accent.group_id == group_id)
            .map_or(fallback, |accent| {
                if accent.custom {
                    egui::Color32::from_rgb(accent.red, accent.green, accent.blue)
                } else {
                    palette
                        .get(usize::from(accent.accent) % palette.len().max(1))
                        .copied()
                        .unwrap_or(fallback)
                }
            })
    }

    pub(crate) fn set_group_accent_color(&mut self, group_id: u64, color: egui::Color32) {
        if let Some(stored) = self
            .group_accents
            .iter_mut()
            .find(|stored| stored.group_id == group_id)
        {
            stored.custom = true;
            stored.red = color.r();
            stored.green = color.g();
            stored.blue = color.b();
        } else {
            self.group_accents.push(GroupAccent {
                group_id,
                accent: 0,
                custom: true,
                red: color.r(),
                green: color.g(),
                blue: color.b(),
            });
        }
    }

    pub(crate) fn group_name(&self, group_id: u64) -> Option<&str> {
        self.group_names
            .iter()
            .find(|stored| stored.group_id == group_id)
            .map(|stored| stored.name.as_str())
    }

    pub(crate) fn set_group_name(&mut self, group_id: u64, name: &str) {
        let name = name.trim().chars().take(32).collect::<String>();
        if name.is_empty() {
            self.group_names
                .retain(|stored| stored.group_id != group_id);
        } else if let Some(stored) = self
            .group_names
            .iter_mut()
            .find(|stored| stored.group_id == group_id)
        {
            stored.name = name;
        } else {
            self.group_names.push(GroupName { group_id, name });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KurvEditorState;
    use truce_core::custom_state::State;

    #[test]
    fn persistent_modulation_cables_default_off_and_round_trip() {
        let default = KurvEditorState::default();
        assert!(!default.persistent_modulation_cables);

        let mut enabled = default;
        enabled.persistent_modulation_cables = true;
        let restored = KurvEditorState::deserialize(&enabled.serialize()).expect("valid state");

        assert!(restored.persistent_modulation_cables);
    }

    #[test]
    fn resynth_quality_defaults_to_standard_and_round_trips() {
        let default = KurvEditorState::default();
        assert_eq!(
            default.resynth_quality,
            crate::oscillators::ResynthQuality::Standard as u8
        );
        let mut ultra = default;
        ultra.resynth_quality = crate::oscillators::ResynthQuality::Ultra as u8;
        let restored = KurvEditorState::deserialize(&ultra.serialize()).expect("valid state");
        assert_eq!(
            restored.resynth_quality,
            crate::oscillators::ResynthQuality::Ultra as u8
        );
    }

    #[test]
    fn group_names_are_bounded_persisted_and_empty_resets_to_default() {
        let mut state = KurvEditorState::default();
        state.set_group_name(42, "  Bass movement  ");
        assert_eq!(state.group_name(42), Some("Bass movement"));

        let restored = KurvEditorState::deserialize(&state.serialize()).expect("valid state");
        assert_eq!(restored.group_name(42), Some("Bass movement"));

        state.set_group_name(42, "   ");
        assert_eq!(state.group_name(42), None);
    }
}
