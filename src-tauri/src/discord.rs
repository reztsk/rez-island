//! Connexion à l'IPC local officiel de Discord (named pipe). Aucune lecture de jeu.
//! Lecture non bloquante (PeekNamedPipe) + délais partout : Discord lent ou absent ne bloque jamais REZ.
use crate::now_ms;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;

/// Pourquoi la connexion n'a pas abouti (toutes ces causes sont réessayées en silence).
#[derive(Debug, PartialEq)]
pub enum OpenErr {
    /// Pas de pipe Discord (Discord fermé ou encore en démarrage).
    NotRunning,
    /// Discord n'a pas répondu à temps.
    NoReply,
    /// Discord refuse cet identifiant d'application (code 4000).
    InvalidId,
    Other(String),
}

/// Identifiant d'application plausible (snowflake de 17 à 20 chiffres).
pub fn valid_id(id: &str) -> bool {
    (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

pub struct Conn {
    pipe: File,
    n: u64,
    /// Événements reçus pendant l'attente d'une réponse.
    pub queue: Vec<Value>,
    /// Profil renvoyé au handshake (id, username, name, avatar).
    pub user: Value,
}

impl Conn {
    pub fn open(client_id: &str, timeout: Duration) -> Result<Conn, OpenErr> {
        if !valid_id(client_id) {
            return Err(OpenErr::InvalidId);
        }
        for i in 0..10 {
            let Ok(pipe) = OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) else { continue };
            let mut c = Conn { pipe, n: 0, queue: vec![], user: Value::Null };
            c.send(0, &json!({ "v": 1, "client_id": client_id })).map_err(OpenErr::Other)?;
            let t0 = Instant::now();
            loop {
                let left = timeout.saturating_sub(t0.elapsed());
                match c.raw_frame(left) {
                    Ok(Some((1, v))) if v["evt"] == "READY" => {
                        let u = &v["data"]["user"];
                        c.user = json!({
                            "id": u["id"],
                            "username": u["username"],
                            "name": u["global_name"].as_str().or(u["username"].as_str()).unwrap_or("Discord"),
                            "avatar": avatar_url(u),
                        });
                        return Ok(c);
                    }
                    Ok(Some((_, v))) if v["code"].as_i64() == Some(4000) => return Err(OpenErr::InvalidId),
                    Ok(Some((2, v))) => {
                        let m = v["message"].as_str().unwrap_or("connexion refusée");
                        return Err(if m.to_lowercase().contains("client id") { OpenErr::InvalidId } else { OpenErr::Other(m.into()) });
                    }
                    Ok(Some(_)) => {} // trame inattendue : on attend READY
                    Ok(None) => return Err(OpenErr::NoReply),
                    Err(e) => return Err(OpenErr::Other(e)),
                }
                if t0.elapsed() >= timeout {
                    return Err(OpenErr::NoReply);
                }
            }
        }
        Err(OpenErr::NotRunning)
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

    /// Trame brute (op 2 = fermeture incluse) si disponible avant `timeout`.
    fn raw_frame(&mut self, timeout: Duration) -> Result<Option<(u32, Value)>, String> {
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
                return Ok(Some((op, serde_json::from_slice(&d).unwrap_or(Value::Null))));
            }
            if t0.elapsed() >= timeout {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Une trame si disponible avant `timeout` (0 = non bloquant). Fermeture par Discord = erreur.
    pub fn frame(&mut self, timeout: Duration) -> Result<Option<(u32, Value)>, String> {
        match self.raw_frame(timeout)? {
            Some((2, _)) => Err("Discord a fermé la connexion".into()),
            f => Ok(f),
        }
    }

    /// Commande RPC -> `data` de la réponse. Les événements reçus entre-temps sont mis en file.
    /// Erreur Discord : "code|message".
    pub fn call(&mut self, cmd: &str, args: Value, evt: Option<&str>, timeout: Duration) -> Result<Value, String> {
        self.n += 1;
        let nonce = format!("rez-{}-{}", now_ms(), self.n);
        let mut msg = json!({ "cmd": cmd, "args": args, "nonce": nonce });
        if let Some(e) = evt {
            msg["evt"] = json!(e);
        }
        self.send(1, &msg)?;
        let t0 = Instant::now();
        loop {
            let left = timeout.checked_sub(t0.elapsed()).ok_or_else(|| format!("0|{cmd} : Discord ne répond pas"))?;
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

    /// `activity` = objet activité Discord, ou Null pour effacer.
    pub fn set_activity(&mut self, activity: &Value) -> Result<(), String> {
        self.call("SET_ACTIVITY", json!({ "pid": std::process::id(), "activity": activity }), None, Duration::from_secs(6)).map(|_| ())
    }
}

/// Photo de profil réelle (CDN Discord) ou avatar par défaut.
pub fn avatar_url(u: &Value) -> String {
    let id = u["id"].as_str().unwrap_or("");
    match u["avatar"].as_str() {
        Some(h) if !h.is_empty() => format!("https://cdn.discordapp.com/avatars/{id}/{h}.png?size=128"),
        _ => {
            let n = id.parse::<u64>().map(|x| (x >> 22) % 6).unwrap_or(0);
            format!("https://cdn.discordapp.com/embed/avatars/{n}.png")
        }
    }
}

/// Activité Rich Presence standard REZ.
pub fn activity(details: &str, state: &str, start: Option<u64>, end: Option<u64>) -> Value {
    let mut ts = json!({});
    if let Some(s) = start {
        ts["start"] = json!(s);
    }
    if let Some(e) = end {
        ts["end"] = json!(e);
    }
    json!({
        "details": details,
        "state": state,
        "timestamps": ts,
        "assets": { "large_image": "rez", "large_text": "REZ Dynamic Island" }
    })
}
