use crate::secure;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

pub const MASK: &str = "••••••••";

#[derive(Serialize, Deserialize, Clone)]
pub struct Pin {
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub shortcut: String,
    pub city: String,
    pub pins: Vec<Pin>,
    pub discord_client_id: String,
    pub discord_details: String,
    pub discord_state: String,
    pub fortnite_key: String,
    pub ai_key: String,
    pub ai_model: String,
    pub autostart: bool,
    pub hide_on_blur: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "Ctrl+B".into(),
            city: "Paris".into(),
            pins: vec![],
            discord_client_id: String::new(),
            discord_details: "Sur REZ".into(),
            discord_state: "Dynamic Island".into(),
            fortnite_key: String::new(),
            ai_key: String::new(),
            ai_model: "claude-opus-5-5".into(),
            autostart: false,
            hide_on_blur: false,
        }
    }
}

fn file(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("REZ"));
    let _ = fs::create_dir_all(&dir);
    dir.join("settings.json")
}

fn default_pins(app: &AppHandle) -> Vec<Pin> {
    let p = app.path();
    let mut v = Vec::new();
    for (name, dir) in [
        ("Bureau", p.desktop_dir()),
        ("Téléchargements", p.download_dir()),
        ("Documents", p.document_dir()),
        ("Images", p.picture_dir()),
    ] {
        if let Ok(path) = dir {
            v.push(Pin { name: name.into(), path: path.to_string_lossy().into() });
        }
    }
    v.push(Pin { name: "Ce PC".into(), path: "shell:MyComputerFolder".into() });
    v.push(Pin { name: "Corbeille".into(), path: "shell:RecycleBinFolder".into() });
    v
}

pub fn load(app: &AppHandle) -> Settings {
    let mut s: Settings = fs::read_to_string(file(app))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Settings { pins: default_pins(app), ..Settings::default() });
    s.ai_key = secure::decrypt(&s.ai_key);
    s.fortnite_key = secure::decrypt(&s.fortnite_key);
    s
}

pub fn save(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let mut d = s.clone();
    d.ai_key = secure::encrypt(&d.ai_key);
    d.fortnite_key = secure::encrypt(&d.fortnite_key);
    let txt = serde_json::to_string_pretty(&d).map_err(crate::err)?;
    fs::write(file(app), txt).map_err(crate::err)
}

/// Les clés ne quittent jamais le backend : le front reçoit un masque.
pub fn masked(s: &Settings) -> Settings {
    let mut m = s.clone();
    if !m.ai_key.is_empty() {
        m.ai_key = MASK.into();
    }
    if !m.fortnite_key.is_empty() {
        m.fortnite_key = MASK.into();
    }
    m
}
