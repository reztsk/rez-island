//! Vocal Discord via le RPC local officiel (scopes rpc + rpc.voice.read/write, autorisés par l'utilisateur
//! dans Discord). Salon vocal en direct : membres, qui parle, micro / casque, quitter.
//! Connexion IPC séparée de la Rich Presence, gérée par un seul thread (lecture non bloquante).
use crate::{now_ms, AppState};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;

pub enum Cmd {
    Auth(Sender<Result<(), String>>),
    Mute(bool),
    Deaf(bool),
    Leave,
    Reset,
}

const SCOPES: [&str; 3] = ["rpc", "rpc.voice.read", "rpc.voice.write"];
const CH_EVENTS: [&str; 5] = ["VOICE_STATE_CREATE", "VOICE_STATE_UPDATE", "VOICE_STATE_DELETE", "SPEAKING_START", "SPEAKING_STOP"];

// ---------- connexion IPC ----------
struct Conn {
    pipe: File,
    n: u64,
    queue: Vec<Value>,
}

impl Conn {
    fn open(client_id: &str) -> Result<Conn, String> {
        for i in 0..10 {
            let Ok(pipe) = OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) else { continue };
            let mut c = Conn { pipe, n: 0, queue: vec![] };
            c.send(0, &json!({ "v": 1, "client_id": client_id }))?;
            match c.frame(Duration::from_secs(5))? {
                Some((1, v)) if v["evt"] == "READY" => return Ok(c),
                Some((_, v)) => {
                    let m = v["message"].as_str().or(v["data"]["message"].as_str()).unwrap_or("handshake refusé");
                    return Err(format!("Discord : {m}"));
                }
                None => return Err("Discord ne répond pas".into()),
            }
        }
        Err("Discord n'est pas lancé".into())
    }

    fn send(&mut self, op: u32, v: &Value) -> Result<(), String> {
        let body = v.to_string().into_bytes();
        let mut buf = Vec::with_capacity(8 + body.len());
        buf.extend_from_slice(&op.to_le_bytes());
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        self.pipe.write_all(&buf).map_err(|_| "Connexion Discord perdue".to_string())
    }

    fn avail(&self) -> Result<u32, String> {
        let mut n = 0u32;
        unsafe { PeekNamedPipe(HANDLE(self.pipe.as_raw_handle()), None, 0, None, Some(&mut n), None) }
            .map_err(|_| "Connexion Discord perdue".to_string())?;
        Ok(n)
    }

    /// Une trame si disponible avant `timeout` (0 = non bloquant).
    fn frame(&mut self, timeout: Duration) -> Result<Option<(u32, Value)>, String> {
        let t0 = Instant::now();
        loop {
            if self.avail()? >= 8 {
                let mut h = [0u8; 8];
                self.pipe.read_exact(&mut h).map_err(crate::err)?;
                let op = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
                let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
                if len > 4 << 20 {
                    return Err("Trame Discord invalide".into());
                }
                let mut d = vec![0u8; len];
                self.pipe.read_exact(&mut d).map_err(crate::err)?;
                if op == 2 {
                    return Err("Discord a fermé la connexion".into());
                }
                return Ok(Some((op, serde_json::from_slice(&d).unwrap_or(Value::Null))));
            }
            if t0.elapsed() >= timeout {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Commande RPC -> `data` de la réponse. Les événements reçus entre-temps sont mis en file.
    fn call(&mut self, cmd: &str, args: Value, evt: Option<&str>, timeout: Duration) -> Result<Value, String> {
        self.n += 1;
        let nonce = format!("rezv-{}-{}", now_ms(), self.n);
        let mut msg = json!({ "cmd": cmd, "args": args, "nonce": nonce });
        if let Some(e) = evt {
            msg["evt"] = json!(e);
        }
        self.send(1, &msg)?;
        let t0 = Instant::now();
        loop {
            let left = timeout.checked_sub(t0.elapsed()).ok_or_else(|| format!("{cmd} : Discord ne répond pas"))?;
            let Some((_, f)) = self.frame(left)? else { continue };
            if f["nonce"] == nonce.as_str() {
                if f["evt"] == "ERROR" {
                    let code = f["data"]["code"].as_i64().unwrap_or(0);
                    let m = f["data"]["message"].as_str().unwrap_or("erreur");
                    return Err(format!("{code}|{m}"));
                }
                return Ok(f["data"].clone());
            }
            if f["evt"].is_string() {
                self.queue.push(f);
            }
        }
    }
}

fn short(e: &str) -> String {
    e.split_once('|').map(|(_, m)| m.to_string()).unwrap_or_else(|| e.to_string())
}

// ---------- état du vocal ----------
#[derive(Default)]
struct Member {
    name: String,
    avatar: String,
    mute: bool,
    deaf: bool,
    speaking: bool,
}

#[derive(Default)]
struct Voice {
    status: &'static str,
    error: String,
    me: String,
    channel: Option<(String, String, String)>, // id, nom, serveur
    members: BTreeMap<String, Member>,
    mute: bool,
    deaf: bool,
}

impl Voice {
    fn snapshot(&self) -> Value {
        let mut members: Vec<Value> = self
            .members
            .iter()
            .map(|(id, m)| {
                json!({ "id": id, "name": m.name, "avatar": m.avatar, "mute": m.mute, "deaf": m.deaf,
                        "speaking": m.speaking, "me": *id == self.me })
            })
            .collect();
        members.sort_by_key(|m| (!m["me"].as_bool().unwrap_or(false), m["name"].as_str().unwrap_or("").to_lowercase()));
        json!({
            "status": self.status,
            "error": self.error,
            "channel": self.channel.as_ref().map(|(id, n, g)| json!({ "id": id, "name": n, "guild": g })),
            "members": members,
            "mute": self.mute,
            "deaf": self.deaf,
        })
    }

    fn upsert(&mut self, vs: &Value) {
        let u = &vs["user"];
        let Some(id) = u["id"].as_str() else { return };
        let s = &vs["voice_state"];
        let b = |k: &str| s[k].as_bool().unwrap_or(false);
        let name = vs["nick"]
            .as_str()
            .filter(|n| !n.is_empty())
            .or(u["global_name"].as_str())
            .or(u["username"].as_str())
            .unwrap_or("?")
            .to_string();
        let speaking = self.members.get(id).map(|m| m.speaking).unwrap_or(false);
        self.members.insert(
            id.to_string(),
            Member {
                name,
                avatar: crate::discord::avatar_url(u),
                mute: b("mute") || b("self_mute") || b("suppress"),
                deaf: b("deaf") || b("self_deaf"),
                speaking,
            },
        );
    }
}

// ---------- OAuth ----------
#[derive(Clone)]
struct Creds {
    cid: String,
    secret: String,
    token: Value, // { access, refresh, exp }
}

fn creds(app: &AppHandle) -> (Creds, bool) {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap();
    let running = st.live.lock().unwrap().discord_running;
    let token = serde_json::from_str(&s.discord_token).unwrap_or(Value::Null);
    (Creds { cid: s.discord_client_id.trim().into(), secret: s.discord_secret.trim().into(), token }, running)
}

fn save_token(app: &AppHandle, t: &Value) {
    let st = app.state::<AppState>();
    st.settings.lock().unwrap().discord_token = if t.is_null() { String::new() } else { t.to_string() };
    crate::persist(app);
}

fn token_from(v: &Value, old_refresh: &str) -> Value {
    json!({
        "access": v["access_token"],
        "refresh": v["refresh_token"].as_str().unwrap_or(old_refresh),
        "exp": now_ms() + v["expires_in"].as_u64().unwrap_or(604_800) * 1000,
    })
}

fn oauth(form: Vec<(&'static str, String)>) -> Result<Value, String> {
    tauri::async_runtime::block_on(crate::net::discord_oauth(form))
}

fn exchange(c: &Creds, code: &str) -> Result<Value, String> {
    let base = |with_redirect: bool| {
        let mut f = vec![
            ("client_id", c.cid.clone()),
            ("client_secret", c.secret.clone()),
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
        ];
        if with_redirect {
            f.push(("redirect_uri", "http://localhost".to_string()));
        }
        f
    };
    match oauth(base(true)) {
        Err(e) if e.to_lowercase().contains("redirect") => oauth(base(false)),
        r => r,
    }
    .map(|v| token_from(&v, ""))
}

fn refresh(c: &Creds) -> Result<Value, String> {
    let r = c.token["refresh"].as_str().unwrap_or("").to_string();
    if r.is_empty() {
        return Err("pas de jeton".into());
    }
    oauth(vec![
        ("client_id", c.cid.clone()),
        ("client_secret", c.secret.clone()),
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", r.clone()),
    ])
    .map(|v| token_from(&v, &r))
}

/// AUTHENTICATE avec le jeton (rafraîchi si expiré / refusé). Renvoie l'id utilisateur.
fn authenticate(app: &AppHandle, conn: &mut Conn, c: &mut Creds) -> Result<String, String> {
    if c.token["exp"].as_u64().unwrap_or(0) < now_ms() + 60_000 {
        if let Ok(t) = refresh(c) {
            save_token(app, &t);
            c.token = t;
        }
    }
    let auth = |conn: &mut Conn, c: &Creds| {
        conn.call("AUTHENTICATE", json!({ "access_token": c.token["access"] }), None, Duration::from_secs(10))
    };
    let r = match auth(conn, c) {
        Err(e) if e.starts_with("4009|") || e.to_lowercase().contains("token") => match refresh(c) {
            Ok(t) => {
                save_token(app, &t);
                c.token = t;
                auth(conn, c)
            }
            Err(_) => {
                save_token(app, &Value::Null);
                return Err("Autorisation expirée : réactive le vocal".into());
            }
        },
        r => r,
    }?;
    Ok(r["user"]["id"].as_str().unwrap_or("").to_string())
}

fn enter(conn: &mut Conn, v: &mut Voice, ch: &Value) {
    let Some(id) = ch["id"].as_str() else { return };
    let mut guild = String::new();
    if let Some(g) = ch["guild_id"].as_str() {
        if let Ok(d) = conn.call("GET_GUILD", json!({ "guild_id": g, "timeout": 2 }), None, Duration::from_secs(4)) {
            guild = d["name"].as_str().unwrap_or("").to_string();
        }
    } else {
        guild = "Appel privé".into();
    }
    let name = ch["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Salon vocal").to_string();
    v.channel = Some((id.to_string(), name, guild));
    v.members.clear();
    for vs in ch["voice_states"].as_array().into_iter().flatten() {
        v.upsert(vs);
    }
    for e in CH_EVENTS {
        let _ = conn.call("SUBSCRIBE", json!({ "channel_id": id }), Some(e), Duration::from_secs(5));
    }
}

fn leave(conn: &mut Conn, v: &mut Voice) {
    if let Some((id, _, _)) = v.channel.take() {
        for e in CH_EVENTS {
            let _ = conn.call("UNSUBSCRIBE", json!({ "channel_id": id }), Some(e), Duration::from_secs(3));
        }
    }
    v.members.clear();
}

fn setup(conn: &mut Conn, v: &mut Voice) -> Result<(), String> {
    for e in ["VOICE_CHANNEL_SELECT", "VOICE_SETTINGS_UPDATE"] {
        conn.call("SUBSCRIBE", json!({}), Some(e), Duration::from_secs(5)).map_err(|e| short(&e))?;
    }
    if let Ok(s) = conn.call("GET_VOICE_SETTINGS", json!({}), None, Duration::from_secs(5)) {
        v.mute = s["mute"].as_bool().unwrap_or(false);
        v.deaf = s["deaf"].as_bool().unwrap_or(false);
    }
    let ch = conn.call("GET_SELECTED_VOICE_CHANNEL", json!({}), None, Duration::from_secs(5)).map_err(|e| short(&e))?;
    if !ch.is_null() {
        enter(conn, v, &ch);
    }
    Ok(())
}

fn on_event(conn: &mut Conn, v: &mut Voice, f: &Value) {
    let d = &f["data"];
    match f["evt"].as_str().unwrap_or("") {
        "VOICE_CHANNEL_SELECT" => {
            let new = d["channel_id"].as_str().map(str::to_string);
            if new.as_deref() != v.channel.as_ref().map(|c| c.0.as_str()) {
                leave(conn, v);
                if let Some(id) = new {
                    if let Ok(ch) = conn.call("GET_CHANNEL", json!({ "channel_id": id }), None, Duration::from_secs(5)) {
                        enter(conn, v, &ch);
                    }
                }
            }
        }
        "VOICE_STATE_CREATE" | "VOICE_STATE_UPDATE" => v.upsert(d),
        "VOICE_STATE_DELETE" => {
            if let Some(id) = d["user"]["id"].as_str() {
                v.members.remove(id);
            }
        }
        "SPEAKING_START" | "SPEAKING_STOP" => {
            let on = f["evt"] == "SPEAKING_START";
            if let Some(m) = d["user_id"].as_str().and_then(|id| v.members.get_mut(id)) {
                m.speaking = on;
            }
        }
        "VOICE_SETTINGS_UPDATE" => {
            if let Some(m) = d["mute"].as_bool() {
                v.mute = m;
            }
            if let Some(m) = d["deaf"].as_bool() {
                v.deaf = m;
            }
        }
        _ => {}
    }
}

/// Autorisation (une fois) : Discord affiche la fenêtre « Autoriser REZ ».
fn authorize(app: &AppHandle, conn: &mut Option<Conn>, v: &mut Voice) -> Result<(), String> {
    let (mut c, running) = creds(app);
    if !running {
        return Err("Lance Discord d'abord".into());
    }
    if c.cid.is_empty() || c.secret.is_empty() {
        return Err("Client ID et Client Secret requis".into());
    }
    *conn = None;
    let mut cn = Conn::open(&c.cid)?;
    v.status = "authorizing";
    publish(app, v);
    let d = cn
        .call("AUTHORIZE", json!({ "client_id": c.cid, "scopes": SCOPES }), None, Duration::from_secs(180))
        .map_err(|e| {
            let m = short(&e);
            if m.to_lowercase().contains("cancel") || e.starts_with("5000|") { "Autorisation refusée dans Discord".to_string() } else { m }
        })?;
    let code = d["code"].as_str().ok_or("Discord n'a pas renvoyé de code")?;
    let t = exchange(&c, code).map_err(|e| format!("OAuth : {e}"))?;
    save_token(app, &t);
    c.token = t;
    v.me = authenticate(app, &mut cn, &mut c)?;
    setup(&mut cn, v)?;
    *conn = Some(cn);
    Ok(())
}

fn publish(app: &AppHandle, v: &Voice) {
    let snap = v.snapshot();
    let st = app.state::<AppState>();
    let mut last = st.voice.lock().unwrap();
    if *last != snap {
        *last = snap.clone();
        drop(last);
        let _ = app.emit("rez://voice", snap);
    }
}

/// Thread du vocal : ne fait rien tant que le vocal n'est pas autorisé.
pub fn run(app: AppHandle, rx: Receiver<Cmd>) {
    let mut conn: Option<Conn> = None;
    let mut v = Voice { status: "off", ..Default::default() };
    let mut retry_at = 0u64;
    loop {
        let wait = if conn.is_some() { 80 } else { 1500 };
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(Cmd::Auth(reply)) => {
                let r = authorize(&app, &mut conn, &mut v);
                if let Err(e) = &r {
                    v.error = e.clone();
                } else {
                    v.error.clear();
                }
                let _ = reply.send(r);
            }
            Ok(Cmd::Reset) => {
                conn = None;
                v = Voice { status: "off", ..Default::default() };
                retry_at = 0;
            }
            Ok(cmd) => {
                if let Some(c) = conn.as_mut() {
                    let r = match cmd {
                        Cmd::Mute(m) => c.call("SET_VOICE_SETTINGS", json!({ "mute": m }), None, Duration::from_secs(5)).map(|d| {
                            v.mute = d["mute"].as_bool().unwrap_or(m);
                        }),
                        Cmd::Deaf(m) => c.call("SET_VOICE_SETTINGS", json!({ "deaf": m }), None, Duration::from_secs(5)).map(|d| {
                            v.deaf = d["deaf"].as_bool().unwrap_or(m);
                        }),
                        Cmd::Leave => c
                            .call("SELECT_VOICE_CHANNEL", json!({ "channel_id": null, "force": true }), None, Duration::from_secs(5))
                            .map(|_| leave(c, &mut v)),
                        _ => Ok(()),
                    };
                    v.error = r.err().map(|e| short(&e)).unwrap_or_default();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        let (mut c, running) = creds(&app);
        if !running || c.cid.is_empty() || c.secret.is_empty() || c.token.is_null() {
            conn = None;
            v.channel = None;
            v.members.clear();
            v.status = if !running {
                "closed"
            } else if c.cid.is_empty() || c.secret.is_empty() {
                "need_secret"
            } else {
                "need_auth"
            };
            publish(&app, &v);
            continue;
        }
        if conn.is_none() && now_ms() >= retry_at {
            v.status = "connecting";
            let r = Conn::open(&c.cid).and_then(|mut cn| {
                v.me = authenticate(&app, &mut cn, &mut c)?;
                setup(&mut cn, &mut v)?;
                Ok(cn)
            });
            match r {
                Ok(cn) => {
                    conn = Some(cn);
                    v.error.clear();
                }
                Err(e) => {
                    v.error = short(&e);
                    v.status = "error";
                    retry_at = now_ms() + 10_000;
                }
            }
        }
        if let Some(cn) = conn.as_mut() {
            v.status = "ready";
            let mut lost = false;
            loop {
                match cn.frame(Duration::ZERO) {
                    Ok(Some((_, f))) => cn.queue.push(f),
                    Ok(None) => break,
                    Err(_) => {
                        lost = true;
                        break;
                    }
                }
            }
            let q: Vec<Value> = std::mem::take(&mut cn.queue);
            for f in &q {
                on_event(cn, &mut v, f);
            }
            if lost {
                conn = None;
                v.channel = None;
                v.members.clear();
                retry_at = now_ms() + 3_000;
            }
        }
        publish(&app, &v);
    }
}
