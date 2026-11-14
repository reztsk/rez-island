#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;
mod media;
mod net;
mod secure;
mod settings;
mod updater;

use serde_json::Value;
use settings::{Settings, MASK};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub struct AppState {
    settings: Mutex<Settings>,
    discord: Mutex<Option<discord::Ipc>>,
    update: Mutex<Option<updater::Remote>>,
}

pub fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------- fenêtre : île ancrée en haut au centre ----------
/// Centre horizontalement, collée au bord haut de l'écran principal. `width` en px physiques.
fn place(w: &tauri::WebviewWindow, width: u32) {
    if let Ok(Some(m)) = w.primary_monitor() {
        let (ms, mp) = (m.size(), m.position());
        let x = mp.x + (ms.width as i32 - width as i32) / 2;
        let _ = w.set_position(PhysicalPosition::new(x, mp.y));
    }
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if let Ok(sz) = w.outer_size() {
            place(&w, sz.width);
        }
        let _ = w.show();
        let _ = app.emit("rez://visible", true);
    }
}

fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            let _ = app.emit("rez://visible", false);
        }
    }
}

/// Ouvre l'île (affiche la fenêtre si besoin) — le front gère l'animation.
fn open(app: &AppHandle) {
    show(app);
    let _ = app.emit("rez://open", ());
}

fn toggle(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = app.emit("rez://toggle", ());
    } else {
        open(app);
    }
}

fn register_shortcut(app: &AppHandle, sc: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(sc).map_err(err)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

// ---------- commandes : app ----------
#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    settings::masked(&state.settings.lock().unwrap())
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, mut s: Settings) -> Result<Settings, String> {
    let old = state.settings.lock().unwrap().clone();
    if s.ai_key == MASK {
        s.ai_key = old.ai_key.clone();
    }
    if s.fortnite_key == MASK {
        s.fortnite_key = old.fortnite_key.clone();
    }
    s.shortcut = s.shortcut.trim().to_string();
    if s.shortcut.is_empty() {
        s.shortcut = "Ctrl+B".into();
    }
    if s.shortcut != old.shortcut {
        if let Err(e) = register_shortcut(&app, &s.shortcut) {
            let _ = register_shortcut(&app, &old.shortcut);
            return Err(format!("Raccourci invalide ou déjà pris : {e}"));
        }
    }
    if s.autostart != old.autostart {
        let al = app.autolaunch();
        if s.autostart { al.enable() } else { al.disable() }.map_err(err)?;
    }
    settings::save(&app, &s)?;
    *state.settings.lock().unwrap() = s.clone();
    Ok(settings::masked(&s))
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide(&app);
}

/// Redimensionne la fenêtre à la taille de l'île (px logiques) et la garde collée en haut.
#[tauri::command]
fn island_fit(app: AppHandle, width: f64, height: f64, focus: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let scale = w.scale_factor().unwrap_or(1.0);
        let _ = w.set_size(LogicalSize::new(width, height));
        place(&w, (width * scale).round() as u32);
        if focus {
            let _ = w.set_focus();
        }
    }
}

#[tauri::command]
fn startup_state(app: AppHandle) -> Value {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    serde_json::json!({ "visible": visible, "hidden": std::env::args().any(|a| a == "--hidden") })
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ---------- commandes : fichiers / liens ----------
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    let p = path.trim();
    if !(p.starts_with("shell:") || Path::new(p).exists()) {
        return Err("Chemin introuvable".into());
    }
    Command::new("explorer.exe").arg(p).spawn().map(|_| ()).map_err(err)
}

#[tauri::command]
fn open_uri(uri: String) -> Result<(), String> {
    if !(uri.starts_with("https://") || uri.starts_with("discord:")) {
        return Err("Lien refusé".into());
    }
    Command::new("explorer.exe").arg(&uri).spawn().map(|_| ()).map_err(err)
}

// ---------- commandes : multimédia ----------
#[tauri::command]
async fn media_info() -> Result<media::MediaInfo, String> {
    blocking(|| Ok(media::info())).await
}

#[tauri::command]
async fn media_action(action: String) -> Result<(), String> {
    blocking(move || media::action(&action)).await
}

#[tauri::command]
async fn media_volume(value: f32) -> Result<(), String> {
    blocking(move || media::set_volume(value)).await
}

#[tauri::command]
async fn media_mute(muted: bool) -> Result<(), String> {
    blocking(move || media::set_mute(muted)).await
}

// ---------- commandes : discord ----------
#[tauri::command]
async fn discord_set(app: AppHandle, details: String, state: String) -> Result<(), String> {
    blocking(move || {
        let st = app.state::<AppState>();
        let cid = {
            let mut s = st.settings.lock().unwrap();
            s.discord_details = details.clone();
            s.discord_state = state.clone();
            let _ = settings::save(&app, &s);
            s.discord_client_id.trim().to_string()
        };
        if cid.is_empty() {
            return Err("Discord Client ID manquant (Paramètres)".into());
        }
        let mut g = st.discord.lock().unwrap();
        let mut last = String::new();
        for _ in 0..2 {
            if g.is_none() {
                *g = Some(discord::Ipc::connect(&cid)?);
            }
            match g.as_mut().unwrap().set_activity(Some((&details, &state))) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = e;
                    *g = None;
                }
            }
        }
        Err(last)
    })
    .await
}

#[tauri::command]
async fn discord_clear(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let st = app.state::<AppState>();
        let mut g = st.discord.lock().unwrap();
        if let Some(ipc) = g.as_mut() {
            let r = ipc.set_activity(None);
            *g = None;
            r
        } else {
            Ok(())
        }
    })
    .await
}

#[tauri::command]
fn discord_status(state: State<AppState>) -> bool {
    state.discord.lock().unwrap().is_some()
}

// ---------- commandes : réseau ----------
#[tauri::command]
async fn weather(app: AppHandle) -> Result<Value, String> {
    let city = app.state::<AppState>().settings.lock().unwrap().city.clone();
    net::weather(&city).await
}

#[tauri::command]
async fn fortnite_stats(app: AppHandle, name: String, platform: String, window: String) -> Result<Value, String> {
    let key = app.state::<AppState>().settings.lock().unwrap().fortnite_key.clone();
    net::fortnite(&key, &name, &platform, &window).await
}

#[tauri::command]
async fn ai_chat(app: AppHandle, messages: Vec<Value>) -> Result<String, String> {
    let (key, model) = {
        let st = app.state::<AppState>();
        let s = st.settings.lock().unwrap();
        (s.ai_key.clone(), s.ai_model.clone())
    };
    net::claude(&key, &model, messages).await
}

// ---------- commandes : mise à jour ----------
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<updater::Check, String> {
    let current = app.package_info().version.clone();
    let (check, remote) = updater::check(&current).await?;
    *app.state::<AppState>().update.lock().unwrap() = if check.available { Some(remote) } else { None };
    Ok(check)
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let remote = app
        .state::<AppState>()
        .update
        .lock()
        .unwrap()
        .clone()
        .ok_or("Aucune mise à jour en attente")?;
    updater::install(remote).await?;
    app.exit(0);
    Ok(())
}

// ---------- main ----------
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            updater::cleanup_old();

            let s = settings::load(&handle);
            if register_shortcut(&handle, &s.shortcut).is_err() {
                let _ = register_shortcut(&handle, "Ctrl+B");
            }
            let pill_mode = s.pill_mode;
            app.manage(AppState {
                settings: Mutex::new(s),
                discord: Mutex::new(None),
                update: Mutex::new(None),
            });

            let m_open = MenuItem::with_id(app, "open", "Ouvrir REZ", true, None::<&str>)?;
            let m_hide = MenuItem::with_id(app, "hide", "Masquer", true, None::<&str>)?;
            let m_quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&m_open, &m_hide, &m_quit])?;
            TrayIconBuilder::with_id("rez")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip("REZ")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => open(app),
                    "hide" => hide(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        toggle(tray.app_handle());
                    }
                })
                .build(app)?;

            // pilule visible dès le démarrage ; ouverture auto sauf lancement masqué (autostart)
            if pill_mode || !std::env::args().any(|a| a == "--hidden") {
                show(&handle);
            }
            Ok(())
        })
        .on_window_event(|w, ev| match ev {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide(w.app_handle());
            }
            WindowEvent::Focused(false) => {
                let _ = w.app_handle().emit("rez://blur", ());
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            hide_window,
            island_fit,
            startup_state,
            quit_app,
            app_version,
            open_path,
            open_uri,
            media_info,
            media_action,
            media_volume,
            media_mute,
            discord_set,
            discord_clear,
            discord_status,
            weather,
            fortnite_stats,
            ai_chat,
            check_update,
            install_update
        ])
        .run(tauri::generate_context!())
        .expect("REZ: erreur au lancement");
}
