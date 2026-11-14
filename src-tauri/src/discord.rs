//! Discord Rich Presence via l'IPC local officiel (named pipe). Aucune lecture de jeu.
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

pub struct Ipc {
    pipe: File,
    start: u64,
}

impl Ipc {
    pub fn connect(client_id: &str) -> Result<Self, String> {
        for i in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{i}");
            if let Ok(pipe) = OpenOptions::new().read(true).write(true).open(&path) {
                let mut ipc = Ipc { pipe, start: crate::now_ms() / 1000 };
                ipc.send(0, &json!({ "v": 1, "client_id": client_id }))?;
                let (op, v) = ipc.recv()?;
                if op == 2 || v["evt"] == "ERROR" {
                    let msg = v["message"].as_str().or(v["data"]["message"].as_str()).unwrap_or("handshake refusé");
                    return Err(format!("Discord : {msg}"));
                }
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

    pub fn set_activity(&mut self, act: Option<(&str, &str)>) -> Result<(), String> {
        let activity = match act {
            Some((details, state)) => json!({
                "details": details,
                "state": state,
                "timestamps": { "start": self.start },
                "assets": { "large_image": "rez", "large_text": "REZ Dynamic Island" }
            }),
            None => Value::Null,
        };
        let nonce = format!("rez-{}", crate::now_ms());
        self.send(1, &json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": nonce
        }))?;
        let (_, r) = self.recv()?;
        if r["evt"] == "ERROR" {
            return Err(r["data"]["message"].as_str().unwrap_or("Erreur Discord").to_string());
        }
        Ok(())
    }
}
