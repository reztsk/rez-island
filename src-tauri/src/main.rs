#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;
mod games;
mod media;
mod net;
mod secure;
mod settings;
mod sys;
mod updater;

use serde_json::{json, Value};
use settings::Settings;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, State, WebviewWindow, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const NO_WINDOW: u32 = 0x0800_0000;

/// État "live" : jeu détecté, Discord, présence envoyée.
#[derive(Default)]
struct Live {
    game: Option<(&'static str, &'static str, u64)>,
    discord_running: bool,
    connected: bool,
    user: Value,
    error: String,
    manual: bool,
    manual_since: u64,
    last_sent: Option<Value>,
    retry_at: u64,
}

pub struct AppState {
    settings: Mutex<Settings>,
    discord: Mutex<Option<discord::Ipc>>,
    live: Mutex<Live>,
    update: Mutex<Option<updater::Remote>>,
    /// Jusqu'à quand (ms) les déplacements de fenêtre viennent de REZ et non de l'utilisateur.
    self_move: AtomicU64,
    anchor_gen: AtomicU64,
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

fn now_s() -> u64 {
    now_ms() / 1000
}

// ---------- fenêtre : île ancrée en haut, ou libre si déverrouillée ----------
/// `width`/`height` en px physiques.
fn place(w: &WebviewWindow, width: i32, height: i32) {
    let st = w.app_handle().state::<AppState>();
    let (locked, anchor) = {
        let s = st.settings.lock().unwrap();
        (s.locked, s.anchor)
    };
    st.self_move.store(now_ms() + 400, Relaxed);
    if let (false, Some((ax, ay))) = (locked, anchor) {
        let mons = w.available_monitors().unwrap_or_default();
        let m = mons.iter().find(|m| {
            let (p, s) = (m.position(), m.size());
            ax >= p.x && ax < p.x + s.width as i32 && ay >= p.y && ay < p.y + s.height as i32
        });
        if let Some(m) = m {
            let (p, s) = (m.position(), m.size());
            let x = (ax - width / 2).min(p.x + s.width as i32 - width).max(p.x);
            let y = ay.min(p.y + s.height as i32 - height).max(p.y);
            let _ = w.set_position(PhysicalPosition::new(x, y));
            return;
        }
    }
    if let Ok(Some(m)) = w.primary_monitor() {
        let (ms, mp) = (m.size(), m.position());
        let x = mp.x + (ms.width as i32 - width) / 2;
        let _ = w.set_position(PhysicalPosition::new(x, mp.y));
    }
}

fn replace_window(w: &WebviewWindow) {
    if let Ok(sz) = w.outer_size() {
        place(w, sz.width as i32, sz.height as i32);
    }
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        replace_window(&w);
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

fn visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Ouvre l'île (affiche la fenêtre si besoin) — le front gère l'animation.
fn open(app: &AppHandle) {
    show(app);
    let _ = app.emit("rez://open", ());
}

/// Raccourci principal : ouvrir / réduire.
fn toggle(app: &AppHandle) {
    if visible(app) {
        let _ = app.emit("rez://toggle", ());
    } else {
        open(app);
    }
}

/// Raccourci "cacher" : retire l'île de l'écran (REZ reste dans le tray) / la remet.
fn hide_toggle(app: &AppHandle) {
    if visible(app) {
        let _ = app.emit("rez://collapse-now", ());
        hide(app);
    } else {
        show(app);
    }
}

fn register_shortcuts(app: &AppHandle, main: &str, hide: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(main).map_err(|e| format!("{main} : {e}"))?;
    if !hide.is_empty() && !hide.eq_ignore_ascii_case(main) {
        gs.register(hide).map_err(|e| format!("{hide} : {e}"))?;
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

fn persist(app: &AppHandle) {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let _ = settings::save(app, &s);
}

// ---------- Discord : connexion auto + présence ----------
fn desired_activity(l: &Live, s: &Settings) -> Value {
    if s.focus_until > now_s() {
        return discord::activity("Mode Focus", "Notifications en pause", None, Some(s.focus_until));
    }
    if s.discord_auto {
        if let Some((_, name, since)) = l.game {
            return discord::activity(&format!("Joue à {name}"), "Suivi par REZ", Some(since), None);
        }
    }
    if l.manual {
        return discord::activity(&s.discord_details, &s.discord_state, Some(l.manual_since), None);
    }
    Value::Null
}

/// Se connecte à Discord s'il tourne et pousse la présence voulue (uniquement si elle change).
fn sync_discord(app: &AppHandle, force: bool) {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();
    let cid = s.discord_client_id.trim().to_string();
    let mut g = st.discord.lock().unwrap();
    let mut l = st.live.lock().unwrap();
    if !l.discord_running || cid.is_empty() {
        *g = None;
        l.connected = false;
        l.user = Value::Null;
        l.last_sent = None;
        l.error = if cid.is_empty() { "Client ID manquant (Paramètres)".into() } else { String::new() };
        return;
    }
    if g.is_none() {
        if !force && now_ms() < l.retry_at {
            return;
        }
        match discord::Ipc::connect(&cid) {
            Ok(ipc) => {
                l.user = ipc.user.clone();
                l.connected = true;
                l.error.clear();
                l.last_sent = None;
                *g = Some(ipc);
            }
            Err(e) => {
                l.connected = false;
                l.error = if e.contains("pas lancé") { "Connexion à Discord…".into() } else { e };
                l.retry_at = now_ms() + 15_000;
                return;
            }
        }
    }
    let want = desired_activity(&l, &s);
    if l.last_sent.as_ref() != Some(&want) {
        match g.as_mut().map(|ipc| ipc.set_activity(&want)) {
            Some(Ok(())) => l.last_sent = Some(want),
            Some(Err(e)) => {
                *g = None;
                l.connected = false;
                l.last_sent = None;
                l.error = e;
            }
            None => {}
        }
    }
}

fn snapshot(app: &AppHandle) -> Value {
    let st = app.state::<AppState>();
    let focus_until = st.settings.lock().unwrap().focus_until;
    let l = st.live.lock().unwrap();
    let act = l.last_sent.clone().unwrap_or(Value::Null);
    json!({
        "game": l.game.map(|(id, name, since)| json!({ "id": id, "name": name, "since": since })),
        "discord": {
            "running": l.discord_running,
            "connected": l.connected,
            "user": l.user,
            "error": l.error,
            "manual": l.manual,
            "activity": if act.is_null() { Value::Null } else { json!({ "details": act["details"], "state": act["state"] }) },
        },
        "focus_until": if focus_until > now_s() { focus_until } else { 0 },
    })
}

fn focus_end(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut s = st.settings.lock().unwrap();
    if !s.focus_prev.is_empty() {
        sys::focus_restore(&s.focus_prev);
    }
    s.focus_prev.clear();
    s.focus_until = 0;
    let c = s.clone();
    drop(s);
    let _ = settings::save(app, &c);
}

/// Boucle légère (4 s) : jeu en cours (noms de processus), Discord, fin du Mode Focus.
fn monitor(app: AppHandle) {
    let mut last = Value::Null;
    loop {
        let scan = games::scan();
        {
            let st = app.state::<AppState>();
            let mut l = st.live.lock().unwrap();
            l.discord_running = scan.discord;
            let same = matches!((&l.game, &scan.game), (Some(a), Some(b)) if a.0 == b.0);
            if !same {
                l.game = scan.game.map(|(id, name)| (id, name, now_s()));
            }
        }
        let expired = {
            let s = app.state::<AppState>().settings.lock().unwrap().focus_until;
            s > 0 && now_s() >= s
        };
        if expired {
            focus_end(&app);
        }
        sync_discord(&app, false);
        let snap = snapshot(&app);
        if snap != last {
            let _ = app.emit("rez://live", &snap);
            last = snap;
        }
        std::thread::sleep(Duration::from_secs(4));
    }
}

fn kick_discord(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        sync_discord(&app, true);
        let _ = app.emit("rez://live", snapshot(&app));
    });
}

// ---------- commandes : app ----------
#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    settings::masked(&state.settings.lock().unwrap())
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, mut s: Settings) -> Result<Settings, String> {
    let old = state.settings.lock().unwrap().clone();
    s.unmask_from(&old);
    // champs gérés uniquement par le backend
    s.anchor = old.anchor;
    s.focus_until = old.focus_until;
    s.focus_prev = old.focus_prev.clone();
    s.opacity = s.opacity.clamp(0.25, 1.0);
    s.shortcut = s.shortcut.trim().to_string();
    s.hide_shortcut = s.hide_shortcut.trim().to_string();
    if s.shortcut.is_empty() {
        s.shortcut = "Ctrl+B".into();
    }
    if s.shortcut.eq_ignore_ascii_case(&s.hide_shortcut) {
        return Err("Les deux raccourcis doivent être différents".into());
    }
    if s.shortcut != old.shortcut || s.hide_shortcut != old.hide_shortcut {
        if let Err(e) = register_shortcuts(&app, &s.shortcut, &s.hide_shortcut) {
            let _ = register_shortcuts(&app, &old.shortcut, &old.hide_shortcut);
            return Err(format!("Raccourci invalide ou déjà pris : {e}"));
        }
    }
    if s.autostart != old.autostart {
        let al = app.autolaunch();
        if s.autostart { al.enable() } else { al.disable() }.map_err(err)?;
    }
    let w = app.get_webview_window("main");
    if !s.locked && s.anchor.is_none() {
        // déverrouillage : l'ancre part de la position actuelle
        if let Some(w) = &w {
            if let (Ok(p), Ok(sz)) = (w.outer_position(), w.outer_size()) {
                s.anchor = Some((p.x + sz.width as i32 / 2, p.y));
            }
        }
    }
    settings::save(&app, &s)?;
    *state.settings.lock().unwrap() = s.clone();
    if s.locked != old.locked {
        if let Some(w) = &w {
            replace_window(w);
        }
    }
    if s.discord_client_id != old.discord_client_id || s.discord_auto != old.discord_auto {
        if s.discord_client_id != old.discord_client_id {
            *state.discord.lock().unwrap() = None;
        }
        kick_discord(&app);
    }
    Ok(settings::masked(&s))
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide(&app);
}

/// Redimensionne la fenêtre à la taille de l'île (px logiques) et la replace.
#[tauri::command]
fn island_fit(app: AppHandle, width: f64, height: f64, focus: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let scale = w.scale_factor().unwrap_or(1.0);
        let _ = w.set_size(LogicalSize::new(width, height));
        place(&w, (width * scale).round() as i32, (height * scale).round() as i32);
        if focus {
            let _ = w.set_focus();
        }
    }
}

/// Glisser-déposer natif (uniquement quand l'île est déverrouillée).
#[tauri::command]
fn start_drag(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    if state.settings.lock().unwrap().locked {
        return Err("Emplacement verrouillé".into());
    }
    let w = app.get_webview_window("main").ok_or("fenêtre")?;
    w.start_dragging().map_err(err)
}

#[tauri::command]
fn startup_state(app: AppHandle) -> Value {
    json!({ "visible": visible(&app), "hidden": std::env::args().any(|a| a == "--hidden") })
}

#[tauri::command]
fn live_state(app: AppHandle) -> Value {
    snapshot(&app)
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn sys_info() -> Value {
    let (cpu, ram, used, total) = sys::info();
    json!({ "cpu": cpu, "ram": ram, "used": used, "total": total })
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

/// Mini-apps Windows (liste fermée).
#[tauri::command]
fn launch(target: String) -> Result<(), String> {
    let t = match target.as_str() {
        "screenshot" => "ms-screenclip:",
        "settings" => "ms-settings:",
        "taskmgr" => "taskmgr",
        "calc" => "calc",
        "notepad" => "notepad",
        "explorer" => "explorer",
        _ => return Err("Application inconnue".into()),
    };
    Command::new("cmd")
        .args(["/C", "start", "", t])
        .creation_flags(NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(err)
}

// ---------- commandes : multimédia ----------
#[tauri::command]
async fn media_info() -> Result<media::MediaInfo, String> {
    blocking(|| Ok(media::info())).await
}

#[tauri::command]
fn media_thumb() -> String {
    media::thumb()
}

#[tauri::command]
async fn media_cover(title: String, artist: String) -> Result<String, String> {
    net::cover_hd(&title, &artist).await
}

#[tauri::command]
async fn media_action(action: String) -> Result<(), String> {
    blocking(move || media::action(&action)).await
}

#[tauri::command]
async fn media_seek(delta: f64, to: Option<f64>) -> Result<(), String> {
    blocking(move || media::seek(delta, to)).await
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
async fn discord_set(app: AppHandle, details: String, state: String) -> Result<Value, String> {
    blocking(move || {
        let st = app.state::<AppState>();
        {
            let mut s = st.settings.lock().unwrap();
            if s.discord_client_id.trim().is_empty() {
                return Err("Discord Client ID manquant (Paramètres)".into());
            }
            s.discord_details = details;
            s.discord_state = state;
        }
        persist(&app);
        {
            let mut l = st.live.lock().unwrap();
            if !l.discord_running {
                return Err("Discord n'est pas lancé".into());
            }
            l.manual = true;
            l.manual_since = now_s();
        }
        sync_discord(&app, true);
        let snap = snapshot(&app);
        let _ = app.emit("rez://live", &snap);
        let l = st.live.lock().unwrap();
        if l.connected { Ok(snap.clone()) } else { Err(l.error.clone()) }
    })
    .await
}

#[tauri::command]
async fn discord_clear(app: AppHandle) -> Result<Value, String> {
    blocking(move || {
        app.state::<AppState>().live.lock().unwrap().manual = false;
        sync_discord(&app, true);
        let snap = snapshot(&app);
        let _ = app.emit("rez://live", &snap);
        Ok(snap)
    })
    .await
}

// ---------- commandes : Mode Focus ----------
#[tauri::command]
async fn focus_start(app: AppHandle, minutes: u32) -> Result<u64, String> {
    let minutes = minutes.clamp(1, 600) as u64;
    blocking(move || {
        let st = app.state::<AppState>();
        let until = now_s() + minutes * 60;
        {
            let mut s = st.settings.lock().unwrap();
            if s.focus_prev.is_empty() {
                s.focus_prev = sys::focus_mute();
            }
            s.focus_until = until;
        }
        persist(&app);
        sync_discord(&app, true);
        let _ = app.emit("rez://live", snapshot(&app));
        Ok(until)
    })
    .await
}

#[tauri::command]
async fn focus_stop(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        focus_end(&app);
        sync_discord(&app, true);
        let _ = app.emit("rez://live", snapshot(&app));
        Ok(())
    })
    .await
}

// ---------- commandes : réseau ----------
/// Météo de `city` (saisie) ou de la ville enregistrée ; une recherche réussie devient la ville par défaut.
#[tauri::command]
async fn weather(app: AppHandle, city: Option<String>) -> Result<Value, String> {
    let typed = city.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    let q = typed.clone().unwrap_or_else(|| app.state::<AppState>().settings.lock().unwrap().city.clone());
    let w = net::weather(&q).await?;
    if typed.is_some() {
        let name = w["city"].as_str().unwrap_or(&q).to_string();
        app.state::<AppState>().settings.lock().unwrap().city = name;
        persist(&app);
    }
    Ok(w)
}

fn key(app: &AppHandle, f: impl Fn(&Settings) -> String) -> String {
    f(&app.state::<AppState>().settings.lock().unwrap())
}

#[tauri::command]
async fn fortnite_stats(app: AppHandle, name: String, platform: String, window: String) -> Result<Value, String> {
    let k = key(&app, |s| s.fortnite_key.clone());
    net::fortnite(&k, &name, &platform, &window).await
}

#[tauri::command]
async fn valorant_stats(app: AppHandle, id: String, region: String) -> Result<Value, String> {
    let k = key(&app, |s| s.henrik_key.clone());
    net::valorant(&k, &id, &region).await
}

#[tauri::command]
async fn apex_stats(app: AppHandle, name: String, platform: String) -> Result<Value, String> {
    let k = key(&app, |s| s.apex_key.clone());
    net::apex(&k, &name, &platform).await
}

#[tauri::command]
async fn ai_chat(app: AppHandle, messages: Vec<Value>) -> Result<String, String> {
    let (k, model) = {
        let st = app.state::<AppState>();
        let s = st.settings.lock().unwrap();
        (s.ai_key.clone(), s.ai_model.clone())
    };
    net::claude(&k, &model, messages).await
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
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() != ShortcutState::Pressed {
                        return;
                    }
                    let hide_sc = app
                        .try_state::<AppState>()
                        .map(|st| st.settings.lock().unwrap().hide_shortcut.clone())
                        .unwrap_or_default();
                    if hide_sc.parse::<Shortcut>().map(|h| &h == sc).unwrap_or(false) {
                        hide_toggle(app);
                    } else {
                        toggle(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            updater::cleanup_old();

            let mut s = settings::load(&handle);
            // Mode Focus interrompu (crash / arrêt) : restaure les notifications si le temps est écoulé
            if !s.focus_prev.is_empty() && s.focus_until <= now_s() {
                sys::focus_restore(&s.focus_prev);
                s.focus_prev.clear();
                s.focus_until = 0;
                let _ = settings::save(&handle, &s);
            }
            if register_shortcuts(&handle, &s.shortcut, &s.hide_shortcut).is_err() {
                let _ = register_shortcuts(&handle, &s.shortcut, "");
            }
            let pill_mode = s.pill_mode;
            app.manage(AppState {
                settings: Mutex::new(s),
                discord: Mutex::new(None),
                live: Mutex::new(Live::default()),
                update: Mutex::new(None),
                self_move: AtomicU64::new(0),
                anchor_gen: AtomicU64::new(0),
            });

            let m_open = MenuItem::with_id(app, "open", "Ouvrir REZ", true, None::<&str>)?;
            let m_hide = MenuItem::with_id(app, "hide", "Afficher / masquer l'île", true, None::<&str>)?;
            let m_quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&m_open, &m_hide, &m_quit])?;
            TrayIconBuilder::with_id("rez")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip("REZ")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => open(app),
                    "hide" => hide_toggle(app),
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

            if pill_mode || !std::env::args().any(|a| a == "--hidden") {
                show(&handle);
            }
            let h = handle.clone();
            std::thread::spawn(move || monitor(h));
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
            WindowEvent::Moved(p) => {
                // glisser-déposer par l'utilisateur (île déverrouillée) -> nouvelle ancre, sauvegardée après 700 ms
                let app = w.app_handle().clone();
                let st = app.state::<AppState>();
                if now_ms() < st.self_move.load(Relaxed) {
                    return;
                }
                {
                    let mut s = st.settings.lock().unwrap();
                    if s.locked {
                        return;
                    }
                    let width = w.outer_size().map(|z| z.width as i32).unwrap_or(0);
                    s.anchor = Some((p.x + width / 2, p.y));
                }
                let gen = st.anchor_gen.fetch_add(1, Relaxed) + 1;
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(700));
                    if app.state::<AppState>().anchor_gen.load(Relaxed) == gen {
                        persist(&app);
                    }
                });
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            hide_window,
            island_fit,
            start_drag,
            startup_state,
            live_state,
            quit_app,
            app_version,
            sys_info,
            open_path,
            open_uri,
            launch,
            media_info,
            media_thumb,
            media_cover,
            media_action,
            media_seek,
            media_volume,
            media_mute,
            discord_set,
            discord_clear,
            focus_start,
            focus_stop,
            weather,
            fortnite_stats,
            valorant_stats,
            apex_stats,
            ai_chat,
            check_update,
            install_update
        ])
        .build(tauri::generate_context!())
        .expect("REZ: erreur au lancement");

    app.run(|app, ev| {
        if let RunEvent::Exit = ev {
            // jamais de notifications Discord coupées après la fermeture de REZ
            if let Some(st) = app.try_state::<AppState>() {
                let prev = st.settings.lock().unwrap().focus_prev.clone();
                if !prev.is_empty() {
                    focus_end(app);
                }
            }
        }
    });
}
