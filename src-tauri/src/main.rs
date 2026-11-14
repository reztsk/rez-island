#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod discord;
mod downloads;
mod games;
mod hw;
mod media;
mod net;
mod secure;
mod settings;
mod sys;
mod updater;
mod voice;

use serde_json::{json, Value};
use settings::Settings;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::Sender;
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
    /// off | no_id | invalid_id | waiting | ready (géré par le thread Discord).
    dstate: &'static str,
    connected: bool,
    user: Value,
    manual: bool,
    manual_since: u64,
    /// Présence réellement affichée dans Discord.
    last_sent: Option<Value>,
    /// Dernière présence transmise au thread Discord.
    wanted: Option<Value>,
    downloads: Vec<Value>,
    net: f64,
}

pub struct AppState {
    settings: Mutex<Settings>,
    live: Mutex<Live>,
    update: Mutex<Option<updater::Remote>>,
    /// Jusqu'à quand (ms) les déplacements de fenêtre viennent de REZ et non de l'utilisateur.
    self_move: AtomicU64,
    anchor_gen: AtomicU64,
    voice_tx: Mutex<Sender<voice::Cmd>>,
    /// Dernier état du vocal Discord envoyé au front.
    voice: Mutex<Value>,
}

fn voice_send(app: &AppHandle, cmd: voice::Cmd) {
    let _ = app.state::<AppState>().voice_tx.lock().unwrap().send(cmd);
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

// ---------- fenêtre : préréglage (haut / gauche / droite) ou position libre mémorisée ----------
type Rect = (i32, i32, i32, i32);

fn monitors(w: &WebviewWindow) -> Vec<Rect> {
    w.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.size().height as i32))
        .collect()
}

fn rect_at(mons: &[Rect], x: i32, y: i32) -> Option<Rect> {
    mons.iter().copied().find(|&(mx, my, mw, mh)| x >= mx && x < mx + mw && y >= my && y < my + mh)
}

/// Écran où se trouve l'île (sinon l'écran principal).
fn current_rect(w: &WebviewWindow, mons: &[Rect]) -> Option<Rect> {
    if let (Ok(p), Ok(s)) = (w.outer_position(), w.outer_size()) {
        if let Some(r) = rect_at(mons, p.x + s.width as i32 / 2, p.y + s.height as i32 / 2) {
            return Some(r);
        }
    }
    w.primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.size().height as i32))
        .or_else(|| mons.first().copied())
}

/// Ancre + alignement d'un préréglage sur l'écran `m`.
fn preset(dock: &str, m: Rect) -> ((i32, i32), &'static str) {
    let (x, y, w, h) = m;
    match dock {
        "left" => ((x, y + h / 2), "lm"),
        "right" => ((x + w, y + h / 2), "rm"),
        _ => ((x + w / 2, y), "ct"),
    }
}

/// Place la fenêtre (`width`/`height` px physiques) : l'ancre reste fixe et l'île grandit
/// vers l'intérieur de l'écran. Verrouiller ne déplace JAMAIS l'île. Renvoie l'alignement.
fn place(w: &WebviewWindow, width: i32, height: i32) -> String {
    let st = w.app_handle().state::<AppState>();
    let (dock, anchor, align) = {
        let s = st.settings.lock().unwrap();
        (s.dock.clone(), s.anchor, s.align.clone())
    };
    st.self_move.store(now_ms() + 400, Relaxed);
    let mons = monitors(w);
    let mut target: Option<(Rect, (i32, i32), String)> = None;
    if let ("free", Some((ax, ay))) = (dock.as_str(), anchor) {
        let b = align.as_bytes();
        let px = if b[0] == b'r' { ax - 1 } else { ax };
        let py = if b[1] == b'b' { ay - 1 } else { ay };
        if let Some(m) = rect_at(&mons, px, py) {
            target = Some((m, (ax, ay), align));
        }
    }
    if target.is_none() {
        let Some(m) = current_rect(w, &mons) else { return "ct".into() };
        let (a, al) = preset(if dock == "free" { "top" } else { &dock }, m);
        target = Some((m, a, al.to_string()));
    }
    let ((mx, my, mw, mh), (ax, ay), al) = target.unwrap();
    let b = al.as_bytes();
    let x = match b[0] {
        b'l' => ax,
        b'r' => ax - width,
        _ => ax - width / 2,
    };
    let y = match b[1] {
        b'b' => ay - height,
        b'm' => ay - height / 2,
        _ => ay,
    };
    let x = x.min(mx + mw - width).max(mx);
    let y = y.min(my + mh - height).max(my);
    let _ = w.set_position(PhysicalPosition::new(x, y));
    al
}

/// Glisser-déposer : nouvelle ancre selon la zone de l'écran où l'île est posée.
fn anchor_from(mons: &[Rect], x: i32, y: i32, wd: i32, ht: i32) -> Option<((i32, i32), String)> {
    let (cx, cy) = (x + wd / 2, y + ht / 2);
    let (mx, my, mw, mh) = rect_at(mons, cx, cy)?;
    let rx = (cx - mx) as f64 / mw as f64;
    let ry = (cy - my) as f64 / mh as f64;
    let h = if rx < 0.34 { 'l' } else if rx > 0.66 { 'r' } else { 'c' };
    let v = if ry > 0.5 { 'b' } else { 't' };
    let ax = match h {
        'l' => x,
        'r' => x + wd,
        _ => cx,
    };
    let ay = if v == 'b' { y + ht } else { y };
    Some(((ax, ay), format!("{h}{v}")))
}

fn replace_window(w: &WebviewWindow) {
    if let Ok(sz) = w.outer_size() {
        let al = place(w, sz.width as i32, sz.height as i32);
        emit_align(w.app_handle(), &al);
    }
}

fn emit_align(app: &AppHandle, align: &str) {
    let dock = app.state::<AppState>().settings.lock().unwrap().dock.clone();
    let _ = app.emit("rez://align", json!({ "dock": dock, "align": align }));
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

/// Transmet la présence voulue au thread Discord (jamais bloquant). `force` = réessayer la connexion maintenant.
fn sync_discord(app: &AppHandle, force: bool) {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();
    let want = {
        let mut l = st.live.lock().unwrap();
        let w = desired_activity(&l, &s);
        if l.wanted.as_ref() == Some(&w) {
            None
        } else {
            l.wanted = Some(w.clone());
            Some(w)
        }
    };
    if let Some(w) = want {
        voice_send(app, voice::Cmd::Activity(w));
    }
    if force {
        voice_send(app, voice::Cmd::Kick);
    }
}

/// Appelé par le thread Discord : état de la connexion + présence affichée.
pub fn set_presence(app: &AppHandle, state: &'static str, user: Value, sent: Option<Value>) {
    let changed = {
        let st = app.state::<AppState>();
        let mut l = st.live.lock().unwrap();
        let connected = state == "ready";
        let changed = l.dstate != state || l.connected != connected || l.user != user || l.last_sent != sent;
        l.dstate = state;
        l.connected = connected;
        l.user = user;
        l.last_sent = sent;
        changed
    };
    if changed {
        let _ = app.emit("rez://live", snapshot(app));
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
            "state": if l.dstate.is_empty() { "off" } else { l.dstate },
            "user": l.user,
            "manual": l.manual,
            "activity": if act.is_null() { Value::Null } else { json!({ "details": act["details"], "state": act["state"] }) },
        },
        "focus_until": if focus_until > now_s() { focus_until } else { 0 },
        "downloads": l.downloads,
        "net": (l.net / 1000.0).round() * 1000.0,
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

/// Boucle légère (4 s, 2 s pendant un téléchargement) : jeu en cours (noms de processus), Discord,
/// téléchargements, fin du Mode Focus.
fn monitor(app: AppHandle) {
    let mut last = Value::Null;
    let dl_dir = app.path().download_dir().ok();
    loop {
        let scan = games::scan();
        let (dls, net) = downloads::scan(&scan.procs, dl_dir.as_deref());
        let busy = !dls.is_empty();
        {
            let st = app.state::<AppState>();
            let mut l = st.live.lock().unwrap();
            l.discord_running = scan.discord;
            l.downloads = dls;
            l.net = net;
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
        std::thread::sleep(Duration::from_secs(if busy { 2 } else { 4 }));
    }
}

fn kick_discord(app: &AppHandle) {
    sync_discord(app, true);
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
    s.align = old.align.clone();
    s.focus_until = old.focus_until;
    s.focus_prev = old.focus_prev.clone();
    s.discord_token = old.discord_token.clone();
    if s.discord_client_id.trim() != old.discord_client_id.trim() || s.discord_secret != old.discord_secret {
        s.discord_token.clear(); // autre app Discord -> nouvelle autorisation vocale
    }
    if !matches!(s.ai_provider.as_str(), "claude" | "openai" | "gemini") {
        s.ai_provider = "claude".into();
    }
    if !matches!(s.dock.as_str(), "top" | "left" | "right" | "free") || (s.dock == "free" && s.anchor.is_none()) {
        s.dock = old.dock.clone();
    }
    let dock_changed = s.dock != old.dock;
    if dock_changed && s.dock != "free" {
        s.anchor = None;
        s.align = preset(&s.dock, (0, 0, 2, 2)).1.to_string();
    }
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
    // (re)verrouiller garde la position actuelle : seul un changement de préréglage déplace l'île
    settings::save(&app, &s)?;
    *state.settings.lock().unwrap() = s.clone();
    if dock_changed {
        if let Some(w) = app.get_webview_window("main") {
            replace_window(&w);
        }
    }
    if s.discord_token != old.discord_token {
        voice_send(&app, voice::Cmd::Reset);
    }
    if s.discord_client_id != old.discord_client_id || s.discord_auto != old.discord_auto {
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
fn island_fit(app: AppHandle, width: f64, height: f64, focus: bool) -> Value {
    let mut al = "ct".to_string();
    if let Some(w) = app.get_webview_window("main") {
        let scale = w.scale_factor().unwrap_or(1.0);
        let _ = w.set_size(LogicalSize::new(width, height));
        al = place(&w, (width * scale).round() as i32, (height * scale).round() as i32);
        if focus {
            let _ = w.set_focus();
        }
    }
    let dock = app.state::<AppState>().settings.lock().unwrap().dock.clone();
    json!({ "dock": dock, "align": al })
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
async fn sys_info() -> Result<Value, String> {
    blocking(|| {
        let (cpu, ram, used, total) = sys::info();
        let mut v = hw::sensors();
        v["cpu"] = json!(cpu);
        v["ram"] = json!(ram);
        v["used"] = json!(used);
        v["total"] = json!(total);
        Ok(v)
    })
    .await
}

// ---------- commandes : mélangeur audio ----------
#[tauri::command]
async fn mixer_list() -> Result<Vec<Value>, String> {
    blocking(audio::list).await
}

#[tauri::command]
async fn mixer_set(key: String, volume: Option<f32>, muted: Option<bool>) -> Result<(), String> {
    blocking(move || audio::set(&key, volume, muted)).await
}

// ---------- commandes : vocal Discord ----------
#[tauri::command]
fn voice_state(state: State<AppState>) -> Value {
    state.voice.lock().unwrap().clone()
}

/// Autorise REZ dans Discord (fenêtre affichée par Discord), une seule fois.
#[tauri::command]
async fn voice_auth(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        voice_send(&app, voice::Cmd::Auth(tx));
        rx.recv_timeout(Duration::from_secs(240)).map_err(|_| "Délai dépassé".to_string())?
    })
    .await
}

#[tauri::command]
fn voice_cmd(app: AppHandle, action: String, on: bool) -> Result<(), String> {
    let cmd = match action.as_str() {
        "mute" => voice::Cmd::Mute(on),
        "deaf" => voice::Cmd::Deaf(on),
        "leave" => voice::Cmd::Leave,
        "logout" => {
            app.state::<AppState>().settings.lock().unwrap().discord_token.clear();
            persist(&app);
            voice::Cmd::Reset
        }
        _ => return Err("Action inconnue".into()),
    };
    voice_send(&app, cmd);
    Ok(())
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
/// Présence personnalisée : enregistrée tout de suite, affichée dès que Discord est connecté.
#[tauri::command]
fn discord_set(app: AppHandle, details: String, state: String) -> Value {
    let st = app.state::<AppState>();
    {
        let mut s = st.settings.lock().unwrap();
        s.discord_details = details;
        s.discord_state = state;
    }
    persist(&app);
    {
        let mut l = st.live.lock().unwrap();
        l.manual = true;
        l.manual_since = now_s();
    }
    sync_discord(&app, true);
    snapshot(&app)
}

#[tauri::command]
fn discord_clear(app: AppHandle) -> Value {
    app.state::<AppState>().live.lock().unwrap().manual = false;
    sync_discord(&app, true);
    snapshot(&app)
}

/// Assistant : vérifie auprès de Discord (IPC local) qu'un Application ID est accepté.
/// Erreurs : "closed" (Discord fermé), "noreply" (Discord occupé), "invalid" (ID refusé) ou un message.
#[tauri::command]
async fn discord_test(app: AppHandle, cid: String) -> Result<Value, String> {
    let cid = cid.trim().to_string();
    if !discord::valid_id(&cid) {
        return Err("invalid".into());
    }
    if !app.state::<AppState>().live.lock().unwrap().discord_running && !games::scan().discord {
        return Err("closed".into());
    }
    blocking(move || match discord::Conn::open(&cid, Duration::from_secs(15)) {
        Ok(c) => Ok(c.user.clone()),
        Err(discord::OpenErr::NotRunning) => Err("closed".into()),
        Err(discord::OpenErr::NoReply) => Err("noreply".into()),
        Err(discord::OpenErr::InvalidId) => Err("invalid".into()),
        Err(discord::OpenErr::Other(m)) => Err(m),
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
    let (provider, k, model) = {
        let st = app.state::<AppState>();
        let s = st.settings.lock().unwrap();
        let k = match s.ai_provider.as_str() {
            "openai" => s.openai_key.clone(),
            "gemini" => s.gemini_key.clone(),
            _ => s.ai_key.clone(),
        };
        (s.ai_provider.clone(), k, s.ai_model.clone())
    };
    match provider.as_str() {
        "openai" => net::openai(&k, &model, messages).await,
        "gemini" => net::gemini(&k, &model, messages).await,
        _ => net::claude(&k, &model, messages).await,
    }
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
/// `rez --diag` : affiche capteurs, mélangeur et téléchargements détectés (dépannage).
fn diag() {
    let _ = sys::info();
    let _ = hw::sensors();
    let _ = hw::net_down();
    std::thread::sleep(Duration::from_millis(1500));
    let scan = games::scan();
    let dl = std::env::var("USERPROFILE").map(|p| Path::new(&p).join("Downloads")).ok();
    let (dls, net) = downloads::scan(&scan.procs, dl.as_deref());
    let out = json!({
        "sys": sys::info(),
        "sensors": hw::sensors(),
        "mixer": audio::list(),
        "downloads": dls,
        "net": net,
        "discord": scan.discord,
        "game": scan.game.map(|g| g.1),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

fn main() {
    // diagnostic : rez --diag-discord <application_id> -> réponse de l'IPC Discord et durée
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--diag-discord") {
        let cid = args.get(i + 1).cloned().unwrap_or_default();
        let t0 = std::time::Instant::now();
        let r = discord::Conn::open(&cid, Duration::from_secs(12)).map(|c| c.user["name"].clone());
        println!("{:?} en {} ms", r.map_err(|e| format!("{e:?}")), t0.elapsed().as_millis());
        return;
    }
    if std::env::args().any(|a| a == "--diag") {
        diag();
        return;
    }
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
            let (vtx, vrx) = std::sync::mpsc::channel();
            app.manage(AppState {
                settings: Mutex::new(s),
                live: Mutex::new(Live::default()),
                update: Mutex::new(None),
                self_move: AtomicU64::new(0),
                anchor_gen: AtomicU64::new(0),
                voice_tx: Mutex::new(vtx),
                voice: Mutex::new(json!({ "status": "off" })),
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
            let h = handle.clone();
            std::thread::spawn(move || voice::run(h, vrx));
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
                // glisser-déposer par l'utilisateur (île déverrouillée) -> position libre, sauvegardée après 700 ms
                let app = w.app_handle().clone();
                let st = app.state::<AppState>();
                if now_ms() < st.self_move.load(Relaxed) {
                    return;
                }
                if st.settings.lock().unwrap().locked {
                    return;
                }
                let Some(win) = app.get_webview_window("main") else { return };
                let (wd, ht) = win.outer_size().map(|z| (z.width as i32, z.height as i32)).unwrap_or((0, 0));
                let Some((anchor, align)) = anchor_from(&monitors(&win), p.x, p.y, wd, ht) else { return };
                let changed = {
                    let mut s = st.settings.lock().unwrap();
                    let changed = s.dock != "free" || s.align != align;
                    s.dock = "free".into();
                    s.anchor = Some(anchor);
                    s.align = align.clone();
                    changed
                };
                if changed {
                    emit_align(&app, &align);
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
            mixer_list,
            mixer_set,
            voice_state,
            voice_auth,
            voice_cmd,
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
            discord_test,
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
