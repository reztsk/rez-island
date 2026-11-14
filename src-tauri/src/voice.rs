//! Thread Discord unique : Rich Presence + vocal en direct sur UNE connexion à l'IPC local officiel.
//! Vocal : scopes rpc + rpc.voice.read/write, autorisés une fois par l'utilisateur dans Discord.
//! Discord fermé, lent ou ID refusé : REZ réessaie discrètement en arrière-plan, sans jamais bloquer.
use crate::discord::{valid_id, Conn, OpenErr};
use crate::{now_ms, AppState};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub enum Cmd {
    /// Présence voulue (Null = aucune).
    Activity(Value),
    /// Réessayer tout de suite (réglages changés, action de l'utilisateur).
    Kick,
    Auth(Sender<Result<(), String>>),
    Mute(bool),
    Deaf(bool),
    Leave,
    Reset,
}

const SCOPES: [&str; 3] = ["rpc", "rpc.voice.read", "rpc.voice.write"];
const CH_EVENTS: [&str; 5] = ["VOICE_STATE_CREATE", "VOICE_STATE_UPDATE", "VOICE_STATE_DELETE", "SPEAKING_START", "SPEAKING_STOP"];
const HANDSHAKE: Duration = Duration::from_secs(12);

/// Délai avant la prochaine tentative (2 s -> 30 s).
fn backoff(fails: u32) -> u64 {
    [2_000, 4_000, 8_000, 15_000, 30_000][(fails.max(1) - 1).min(4) as usize]
}

/// Message lisible pour une connexion initiée par l'utilisateur.
pub fn open_msg(e: OpenErr) -> String {
    match e {
        OpenErr::NotRunning => "Lance Discord, puis réessaie".into(),
        OpenErr::NoReply => "Discord démarre encore : réessaie dans quelques secondes".into(),
        OpenErr::InvalidId => "Discord ne reconnaît pas cet Application ID".into(),
        OpenErr::Other(m) => m,
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

/// Autorisation (une fois) : Discord affiche la fenêtre « Autoriser REZ ». Réutilise la connexion du thread.
fn authorize(app: &AppHandle, conn: &mut Option<Conn>, conn_cid: &mut String, v: &mut Voice) -> Result<(), String> {
    let (mut c, running) = creds(app);
    if !running {
        return Err("Lance Discord, puis réessaie".into());
    }
    if !valid_id(&c.cid) {
        return Err("Configure d'abord l'Application ID (étape 1)".into());
    }
    if c.secret.is_empty() {
        return Err("Colle le Client Secret".into());
    }
    if conn.is_none() || *conn_cid != c.cid {
        *conn = Some(Conn::open(&c.cid, HANDSHAKE).map_err(open_msg)?);
        *conn_cid = c.cid.clone();
    }
    let cn = conn.as_mut().unwrap();
    v.status = "authorizing";
    publish(app, v);
    let d = cn
        .call("AUTHORIZE", json!({ "client_id": c.cid, "scopes": SCOPES }), None, Duration::from_secs(180))
        .map_err(|e| {
            let m = short(&e);
            if m.to_lowercase().contains("cancel") || e.starts_with("5000|") { "Autorisation refusée dans Discord".to_string() } else { m }
        })?;
    let code = d["code"].as_str().ok_or("Discord n'a pas renvoyé de code")?;
    let t = exchange(&c, code).map_err(|e| {
        if e.to_lowercase().contains("redirect") {
            "Ajoute la redirection http://localhost dans OAuth2 (étape 3)".to_string()
        } else if e.to_lowercase().contains("client") {
            "Client Secret refusé : recopie-le depuis OAuth2".to_string()
        } else {
            format!("OAuth : {e}")
        }
    })?;
    save_token(app, &t);
    c.token = t;
    v.me = authenticate(app, cn, &mut c)?;
    setup(cn, v)?;
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

fn idle_voice(v: &mut Voice, status: &'static str) {
    v.channel = None;
    v.members.clear();
    v.status = status;
}

/// Thread Discord : connexion auto dès que Discord tourne, présence, vocal si autorisé.
pub fn run(app: AppHandle, rx: Receiver<Cmd>) {
    let mut conn: Option<Conn> = None;
    let mut conn_cid = String::new();
    let mut v = Voice { status: "off", ..Default::default() };
    let mut want = Value::Null; // présence voulue
    let mut sent: Option<Value> = None; // présence réellement affichée
    let mut pstate: &'static str = "off";
    let (mut fails, mut retry_at) = (0u32, 0u64);
    let mut authed = false;
    let (mut vfails, mut vretry_at) = (0u32, 0u64);
    loop {
        let wait = if conn.is_some() { 80 } else { 1000 };
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(Cmd::Activity(a)) => want = a,
            Ok(Cmd::Kick) => {
                (fails, retry_at, vfails, vretry_at) = (0, 0, 0, 0);
            }
            Ok(Cmd::Auth(reply)) => {
                let r = authorize(&app, &mut conn, &mut conn_cid, &mut v);
                match &r {
                    Ok(()) => {
                        authed = true;
                        v.error.clear();
                        (fails, vfails) = (0, 0);
                    }
                    Err(e) => {
                        v.error = e.clone();
                        authed = false;
                    }
                }
                let _ = reply.send(r);
            }
            Ok(Cmd::Reset) => {
                // nouveau jeton / déconnexion : une connexion neuve oublie l'ancienne authentification
                conn = None;
                sent = None;
                authed = false;
                v = Voice { status: "off", ..Default::default() };
                (fails, retry_at, vfails, vretry_at) = (0, 0, 0, 0);
            }
            Ok(cmd) => {
                if let (Some(c), true) = (conn.as_mut(), authed) {
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
        if c.cid != conn_cid {
            conn = None;
            authed = false;
            sent = None;
            (fails, retry_at, vfails, vretry_at) = (0, 0, 0, 0);
            conn_cid = c.cid.clone();
        }
        if !running || !valid_id(&c.cid) {
            conn = None;
            authed = false;
            sent = None;
            (fails, retry_at) = (0, 0);
            pstate = if !running {
                "off"
            } else if c.cid.is_empty() {
                "no_id"
            } else {
                "invalid_id"
            };
            crate::set_presence(&app, pstate, Value::Null, None);
            idle_voice(&mut v, if running { "need_id" } else { "closed" });
            publish(&app, &v);
            continue;
        }
        if conn.is_none() && now_ms() >= retry_at {
            match Conn::open(&c.cid, HANDSHAKE) {
                Ok(cn) => {
                    conn = Some(cn);
                    fails = 0;
                    sent = None;
                    authed = false;
                    (vfails, vretry_at) = (0, 0);
                    pstate = "ready";
                }
                Err(OpenErr::InvalidId) => {
                    pstate = "invalid_id";
                    retry_at = now_ms() + 60_000;
                }
                Err(_) => {
                    pstate = "waiting";
                    fails += 1;
                    retry_at = now_ms() + backoff(fails);
                }
            }
        }
        let has_auth = !c.secret.is_empty() && !c.token.is_null();
        let Some(cn) = conn.as_mut() else {
            crate::set_presence(&app, pstate, Value::Null, None);
            let st = if pstate == "invalid_id" {
                "need_id"
            } else if c.secret.is_empty() {
                "need_secret"
            } else if c.token.is_null() {
                "need_auth"
            } else {
                "connecting"
            };
            idle_voice(&mut v, st);
            publish(&app, &v);
            continue;
        };

        // vocal : authentification automatique si REZ a déjà été autorisé
        if !has_auth {
            authed = false;
            idle_voice(&mut v, if c.secret.is_empty() { "need_secret" } else { "need_auth" });
        } else if !authed && now_ms() >= vretry_at {
            v.status = "connecting";
            let r = authenticate(&app, cn, &mut c).and_then(|me| {
                v.me = me;
                setup(cn, &mut v)
            });
            match r {
                Ok(()) => {
                    authed = true;
                    vfails = 0;
                    v.error.clear();
                }
                Err(_) => {
                    vfails += 1;
                    vretry_at = now_ms() + backoff(vfails);
                }
            }
        }
        if authed {
            v.status = "ready";
        }

        // présence
        let mut lost = false;
        if sent.as_ref() != Some(&want) {
            match cn.set_activity(&want) {
                Ok(()) => sent = Some(want.clone()),
                // refus explicite de Discord : inutile d'insister avec la même présence
                Err(e) if e.contains('|') && !e.starts_with("0|") => sent = Some(want.clone()),
                Err(_) => lost = true,
            }
        }
        // événements
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
        if authed {
            for f in &q {
                on_event(cn, &mut v, f);
            }
        }
        let user = cn.user.clone();
        if lost {
            conn = None;
            authed = false;
            sent = None;
            pstate = "waiting";
            fails = 1;
            retry_at = now_ms() + backoff(1);
            let vs = if has_auth { "connecting" } else { v.status };
            idle_voice(&mut v, vs);
            crate::set_presence(&app, pstate, Value::Null, None);
        } else {
            crate::set_presence(&app, pstate, user, sent.clone());
        }
        publish(&app, &v);
    }
}
