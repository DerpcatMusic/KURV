//! Persisted editor-theme translation.

use truce_core::editor::PluginContext;

use crate::{KurvParams, editor_theme};

pub(in crate::editor_shell) fn read_persisted_theme(
    state: &PluginContext<KurvParams>,
) -> editor_theme::ThemeSettings {
    state.params().editor_state.lock().map_or_else(
        |_| editor_theme::ThemeSettings::default(),
        |mut editor| {
            let theme = theme_from_editor(&editor);
            if editor.theme_schema < 2 {
                editor.theme_schema = 2;
                [
                    editor.background_red,
                    editor.background_green,
                    editor.background_blue,
                ] = theme.background_rgb;
                editor.theme_tint = theme.tint;
                editor.theme_contrast = theme.contrast;
                [
                    editor.secondary_red,
                    editor.secondary_green,
                    editor.secondary_blue,
                ] = theme.secondary_rgb;
                [
                    editor.tertiary_red,
                    editor.tertiary_green,
                    editor.tertiary_blue,
                ] = theme.tertiary_rgb;
            }
            theme
        },
    )
}

pub(super) fn read_editor_theme(state: &PluginContext<KurvParams>) -> editor_theme::ThemeSettings {
    state.params().editor_state.lock().map_or_else(
        |_| editor_theme::ThemeSettings::default(),
        |editor| theme_from_editor(&editor),
    )
}

pub(super) fn write_editor_theme(
    state: &PluginContext<KurvParams>,
    settings: editor_theme::ThemeSettings,
) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor.theme_schema = 2;
        [
            editor.background_red,
            editor.background_green,
            editor.background_blue,
        ] = settings.background_rgb;
        editor.theme_tint = settings.tint;
        editor.theme_contrast = settings.contrast;
        [
            editor.primary_red,
            editor.primary_green,
            editor.primary_blue,
        ] = settings.primary_rgb;
        [
            editor.secondary_red,
            editor.secondary_green,
            editor.secondary_blue,
        ] = settings.secondary_rgb;
        [
            editor.tertiary_red,
            editor.tertiary_green,
            editor.tertiary_blue,
        ] = settings.tertiary_rgb;
    }
}

fn theme_from_editor(editor: &crate::KurvEditorState) -> editor_theme::ThemeSettings {
    editor_theme::ThemeSettings::from_persisted(
        editor.theme_schema,
        editor.theme_preset,
        [
            editor.background_red,
            editor.background_green,
            editor.background_blue,
        ],
        editor.theme_tint,
        editor.theme_contrast,
        [
            editor.primary_red,
            editor.primary_green,
            editor.primary_blue,
        ],
        [
            editor.secondary_red,
            editor.secondary_green,
            editor.secondary_blue,
        ],
        [
            editor.tertiary_red,
            editor.tertiary_green,
            editor.tertiary_blue,
        ],
    )
}
