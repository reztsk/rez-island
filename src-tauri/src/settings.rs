use crate::secure;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    pub hide_shortcut: String,
    pub city: String,
    pub pins: Vec<Pin>,
    pub discord_client_id: String,
    pub discord_details: String,
    pub discord_state: String,
    pub discord_auto: bool,
    pub fortnite_key: String,
    pub henrik_key: String,
    pub apex_key: String,
    pub ai_key: String,
    pub ai_model: String,
    pub autostart: bool,
    pub hide_on_blur: bool,
    pub size: String,
    pub pill_mode: bool,
    pub opacity: f64,
    pub locked: bool,
    /// Point d'ancrage (centre haut, px physiques) quand l'île est déverrouillée.
    pub anchor: Option<(i32, i32)>,
    /// ID joueur lié par jeu : fortnite, valorant, apex, r6, rl (+ "<jeu>_opt" plateforme/région).
    pub players: BTreeMap<String, String>,
    /// Mode Focus : fin (s, epoch) + valeurs registre d'origine à restaurer.
    pub focus_until: u64,
    pub focus_prev: BTreeMap<String, i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "Ctrl+B".into(),
            hide_shortcut: "Ctrl+N".into(),
            city: "Paris".into(),
            pins: vec![],
            discord_client_id: String::new(),
            discord_details: "Sur REZ".into(),
            discord_state: "Dynamic Island".into(),
            discord_auto: true,
            fortnite_key: String::new(),
            henrik_key: String::new(),
            apex_key: String::new(),
            ai_key: String::new(),
            ai_model: "claude-opus-5-5".into(),
            autostart: false,
            hide_on_blur: false,
            size: "compact".into(),
            pill_mode: true,
            opacity: 0.72,
            locked: true,
            anchor: None,
            players: BTreeMap::new(),
            focus_until: 0,
            focus_prev: BTreeMap::new(),
        }
    }
}

impl Settings {
    fn keys_mut(&mut self) -> [&mut String; 4] {
        [&mut self.ai_key, &mut self.fortnite_key, &mut self.henrik_key, &mut self.apex_key]
    }
    /// Remet les vraies clés quand le front renvoie le masque.
    pub fn unmask_from(&mut self, old: &Settings) {
        let mut old = old.clone();
        for (k, o) in self.keys_mut().into_iter().zip(old.keys_mut()) {
            if k == MASK {
                *k = o.clone();
            }
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
    for k in s.keys_mut() {
        *k = secure::decrypt(k);
    }
    s.opacity = s.opacity.clamp(0.25, 1.0);
    s
}

pub fn save(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let mut d = s.clone();
    for k in d.keys_mut() {
        *k = secure::encrypt(k);
    }
    let txt = serde_json::to_string_pretty(&d).map_err(crate::err)?;
    fs::write(file(app), txt).map_err(crate::err)
}

/// Les clés ne quittent jamais le backend : le front reçoit un masque.
pub fn masked(s: &Settings) -> Settings {
    let mut m = s.clone();
    for k in m.keys_mut() {
        if !k.is_empty() {
            *k = MASK.into();
        }
    }
    m
}
