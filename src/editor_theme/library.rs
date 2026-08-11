use std::fs;
use std::io;
use std::path::PathBuf;

use crate::editor_presets::{atomic_write, sanitize_name, user_data_directory};

use super::{BUILTIN_THEMES, ThemeSettings};

#[derive(Clone, Debug)]
struct NamedTheme {
    name: String,
    settings: ThemeSettings,
}

#[derive(Clone, Debug)]
pub(crate) struct ThemeLibrary {
    path: PathBuf,
    active: String,
    themes: Vec<NamedTheme>,
}

impl ThemeLibrary {
    pub(crate) fn load(initial: ThemeSettings) -> io::Result<Self> {
        let path = user_data_directory()?.join("Themes").join("themes.json");
        match fs::read(&path) {
            Ok(bytes) => Self::decode(path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let library = Self {
                    path,
                    active: "Custom".to_owned(),
                    themes: vec![NamedTheme {
                        name: "Custom".to_owned(),
                        settings: initial,
                    }],
                };
                library.write()?;
                Ok(library)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn active_name(&self) -> &str {
        &self.active
    }

    pub(crate) fn names(&self) -> Vec<String> {
        BUILTIN_THEMES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .chain(self.themes.iter().map(|theme| theme.name.clone()))
            .collect()
    }

    pub(crate) fn active_settings(&self) -> Option<ThemeSettings> {
        self.find(&self.active)
    }

    pub(crate) fn select(&mut self, name: &str) -> io::Result<ThemeSettings> {
        let settings = self
            .find(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "theme was not found"))?;
        self.active = name.to_owned();
        self.write()?;
        Ok(settings)
    }

    pub(crate) fn update_active(&mut self, settings: ThemeSettings) {
        if BUILTIN_THEMES
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(&self.active))
        {
            self.active = "Custom".to_owned();
        }
        if let Some(theme) = self
            .themes
            .iter_mut()
            .find(|theme| theme.name.eq_ignore_ascii_case(&self.active))
        {
            theme.settings = settings;
        } else {
            self.themes.push(NamedTheme {
                name: self.active.clone(),
                settings,
            });
        }
    }

    pub(crate) fn save_as(
        &mut self,
        requested_name: &str,
        settings: ThemeSettings,
    ) -> io::Result<()> {
        let name = sanitize_name(requested_name)?;
        if BUILTIN_THEMES
            .iter()
            .any(|(builtin, _)| builtin.eq_ignore_ascii_case(&name))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "built-in theme names are reserved",
            ));
        }
        self.active = name;
        self.update_active(settings);
        self.write()
    }

    pub(crate) fn write(&self) -> io::Result<()> {
        let themes: Vec<_> = self
            .themes
            .iter()
            .map(|theme| {
                serde_json::json!({
                    "name": theme.name,
                    "background": theme.settings.background_rgb,
                    "tint": theme.settings.tint,
                    "contrast": theme.settings.contrast,
                    "primary": theme.settings.primary_rgb,
                    "secondary": theme.settings.secondary_rgb,
                    "tertiary": theme.settings.tertiary_rgb,
                })
            })
            .collect();
        let document = serde_json::json!({
            "version": 1,
            "active": self.active,
            "themes": themes,
        });
        let bytes = serde_json::to_vec_pretty(&document).map_err(io::Error::other)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("theme path has no parent"))?;
        fs::create_dir_all(parent)?;
        atomic_write(&self.path, &bytes)
    }

    fn find(&self, name: &str) -> Option<ThemeSettings> {
        BUILTIN_THEMES
            .iter()
            .find(|(builtin, _)| builtin.eq_ignore_ascii_case(name))
            .map(|(_, settings)| *settings)
            .or_else(|| {
                self.themes
                    .iter()
                    .find(|theme| theme.name.eq_ignore_ascii_case(name))
                    .map(|theme| theme.settings)
            })
    }

    fn decode(path: PathBuf, bytes: &[u8]) -> io::Result<Self> {
        let document: serde_json::Value =
            serde_json::from_slice(bytes).map_err(io::Error::other)?;
        if document.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported theme library version",
            ));
        }
        let active = document
            .get("active")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing active theme"))?
            .to_owned();
        let mut themes = Vec::new();
        for value in document
            .get("themes")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing themes"))?
        {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "theme has no name"))?;
            let name = sanitize_name(name)?;
            if BUILTIN_THEMES
                .iter()
                .any(|(builtin, _)| builtin.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            themes.push(NamedTheme {
                name,
                settings: ThemeSettings {
                    background_rgb: decode_rgb(value, "background")?,
                    tint: decode_u8(value, "tint")?.min(100),
                    contrast: decode_u8(value, "contrast")?.clamp(50, 175),
                    primary_rgb: decode_rgb(value, "primary")?,
                    secondary_rgb: decode_rgb(value, "secondary")?,
                    tertiary_rgb: decode_rgb(value, "tertiary")?,
                },
            });
        }
        let library = Self {
            path,
            active,
            themes,
        };
        if library.active_settings().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "active theme was not found",
            ));
        }
        Ok(library)
    }
}

fn decode_rgb(value: &serde_json::Value, key: &str) -> io::Result<[u8; 3]> {
    let channels = value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .filter(|channels| channels.len() == 3)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid theme color"))?;
    let mut rgb = [0_u8; 3];
    for (output, channel) in rgb.iter_mut().zip(channels) {
        *output = u8::try_from(channel.as_u64().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid theme color channel")
        })?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "theme color overflow"))?;
    }
    Ok(rgb)
}

fn decode_u8(value: &serde_json::Value, key: &str) -> io::Result<u8> {
    u8::try_from(
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid theme value"))?,
    )
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "theme value overflow"))
}
