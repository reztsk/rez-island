//! Discord Rich Presence via l'IPC local officiel (named pipe). Aucune lecture de jeu.
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

pub struct Ipc {
    pipe: File,
    /// Profil renvoyé par Discord au handshake (id, username, global_name, avatar).
    pub user: Value,
}

impl Ipc {
    pub fn connect(client_id: &str) -> Result<Self, String> {
        for i in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{i}");
            if let Ok(pipe) = OpenOptions::new().read(true).write(true).open(&path) {
                let mut ipc = Ipc { pipe, user: Value::Null };
                ipc.send(0, &json!({ "v": 1, "client_id": client_id }))?;
                let (op, v) = ipc.recv()?;
                if op == 2 || v["evt"] == "ERROR" {
                    let msg = v["message"].as_str().or(v["data"]["message"].as_str()).unwrap_or("handshake refusé");
                    return Err(format!("Discord : {msg}"));
                }
                let u = &v["data"]["user"];
                ipc.user = json!({
                    "id": u["id"],
                    "username": u["username"],
                    "name": u["global_name"].as_str().or(u["username"].as_str()).unwrap_or("Discord"),
                    "avatar": avatar_url(u),
                });
                return Ok(ipc);
            }
        }
        Err("Discord n'est pas lancé".into())
    }

    fn send(&mut self, op: u32, payload: &Value) -> Result<(), String> {
        let body = payload.to_string().into_bytes();
        let mut buf = Vec::with_capacity(8 + body.len());
        buf.extend_from_slice(&op.to_le_bytes());
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);
        self.pipe.write_all(&buf).map_err(crate::err)
    }

    fn recv(&mut self) -> Result<(u32, Value), String> {
        let mut h = [0u8; 8];
        self.pipe.read_exact(&mut h).map_err(crate::err)?;
        let op = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        if len > 1 << 20 {
            return Err("Trame Discord invalide".into());
        }
        let mut data = vec![0u8; len];
        self.pipe.read_exact(&mut data).map_err(crate::err)?;
        Ok((op, serde_json::from_slice(&data).unwrap_or(Value::Null)))
    }

    /// `activity` = objet activité Discord, ou Null pour effacer.
    pub fn set_activity(&mut self, activity: &Value) -> Result<(), String> {
        let nonce = format!("rez-{}", crate::now_ms());
        self.send(1, &json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": nonce
        }))?;
        // ignore les éventuels événements jusqu'à la réponse à notre nonce
        for _ in 0..8 {
            let (op, r) = self.recv()?;
            if op == 2 {
                return Err("Discord a fermé la connexion".into());
            }
            if r["evt"] == "ERROR" {
                return Err(r["data"]["message"].as_str().unwrap_or("Erreur Discord").to_string());
            }
            if r["nonce"] == nonce.as_str() {
                return Ok(());
            }
        }
        Ok(())
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
