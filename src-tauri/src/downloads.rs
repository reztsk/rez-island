//! Téléchargements en cours, sans jamais toucher aux jeux :
//! - Steam : manifestes `steamapps/appmanifest_*.acf` (tenue de compte de Steam, jamais `steamapps/common`).
//! - Navigateurs / torrents : fichiers partiels du dossier Téléchargements (.crdownload, .part, .!qB…).
//! - qBittorrent : API Web locale officielle (si activée).
//! - Epic, EA, Battle.net, Ubisoft : pas de progression publique -> débit réseau du PC.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

struct Track {
    bytes: u64,
    at: u64,
    speed: f64,
    seen: u64,
}
static TRACK: Mutex<Option<HashMap<String, Track>>> = Mutex::new(None);

/// Vitesse lissée (octets/s) d'un élément suivi.
fn speed(key: &str, bytes: u64) -> f64 {
    let now = crate::now_ms();
    let mut g = TRACK.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    m.retain(|_, t| now - t.seen < 60_000);
    let t = m.entry(key.to_string()).or_insert(Track { bytes, at: now, speed: 0.0, seen: now });
    t.seen = now;
    let dt = now.saturating_sub(t.at) as f64 / 1000.0;
    if dt >= 1.0 {
        let inst = bytes.saturating_sub(t.bytes) as f64 / dt;
        t.speed = if t.speed == 0.0 { inst } else { t.speed * 0.5 + inst * 0.5 };
        t.bytes = bytes;
        t.at = now;
    }
    t.speed
}

fn item(source: &str, name: &str, done: u64, total: u64, spd: f64, state: &str) -> Value {
    let eta = if total > done && spd > 1.0 { Some(((total - done) as f64 / spd).round() as u64) } else { None };
    let pct = if total > 0 { Some((done as f64 / total as f64 * 100.0).clamp(0.0, 100.0)) } else { None };
    json!({ "source": source, "name": name, "done": done, "total": total, "pct": pct, "speed": spd, "eta": eta, "state": state })
}

// ---------- Steam ----------
fn reg_sz(key: &str, val: &str) -> Option<String> {
    let (k, v) = (HSTRING::from(key), HSTRING::from(val));
    let mut buf = vec![0u16; 520];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, &k, &v, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut len)) };
    if r != ERROR_SUCCESS {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// Paires "clé" "valeur" d'un fichier VDF/ACF (format texte de Steam).
fn vdf_pairs(txt: &str) -> impl Iterator<Item = (String, String)> + '_ {
    txt.lines().filter_map(|l| {
        let p: Vec<&str> = l.trim().split('"').collect();
        (p.len() >= 5).then(|| (p[1].to_lowercase(), p[3].replace("\\\\", "\\")))
    })
}

fn steam_libraries() -> Vec<PathBuf> {
    static C: Mutex<(u64, Vec<PathBuf>)> = Mutex::new((0, Vec::new()));
    let mut c = C.lock().unwrap();
    if crate::now_ms() - c.0 < 60_000 && !c.1.is_empty() {
        return c.1.clone();
    }
    let mut libs = Vec::new();
    if let Some(root) = reg_sz(r"Software\Valve\Steam", "SteamPath") {
        let root = PathBuf::from(root.replace('/', "\\"));
        libs.push(root.join("steamapps"));
        if let Ok(t) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
            for (k, v) in vdf_pairs(&t) {
                if k == "path" {
                    let p = PathBuf::from(v).join("steamapps");
                    if !libs.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())) {
                        libs.push(p);
                    }
                }
            }
        }
    }
    *c = (crate::now_ms(), libs.clone());
    libs
}

fn steam(out: &mut Vec<Value>) {
    // drapeaux EAppState : 256 UpdateRunning, 512 UpdatePaused, 1024 UpdateStarted, 1048576 Downloading,
    // 2097152 Staging, 4194304 Committing
    const RUN: u64 = 256 | 1024 | 1_048_576 | 2_097_152 | 4_194_304;
    for lib in steam_libraries() {
        let Ok(rd) = std::fs::read_dir(&lib) else { continue };
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_lowercase();
            if !(f.starts_with("appmanifest_") && f.ends_with(".acf")) {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
            let mut m: HashMap<String, String> = HashMap::new();
            for (k, v) in vdf_pairs(&t) {
                m.entry(k).or_insert(v);
            }
            let n = |k: &str| m.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let flags = n("stateflags");
            let paused = flags & 512 != 0;
            if flags & RUN == 0 && !paused {
                continue;
            }
            let name = m.get("name").cloned().unwrap_or_else(|| "Jeu Steam".into());
            let (todl, dl, tost, st) = (n("bytestodownload"), n("bytesdownloaded"), n("bytestostage"), n("bytesstaged"));
            let key = format!("steam:{f}");
            if todl > 0 && dl < todl {
                let s = if paused { 0.0 } else { speed(&key, dl) };
                out.push(item("Steam", &name, dl, todl, s, if paused { "En pause" } else { "Téléchargement" }));
            } else if tost > 0 && st < tost {
                let s = if paused { 0.0 } else { speed(&key, st) };
                out.push(item("Steam", &name, st, tost, s, if paused { "En pause" } else { "Installation" }));
            }
        }
    }
}

// ---------- fichiers partiels (navigateurs, torrents) ----------
const PARTIAL: [(&str, &str); 8] = [
    ("crdownload", "Navigateur"),
    ("part", "Firefox"),
    ("partial", "Navigateur"),
    ("download", "Navigateur"),
    ("opdownload", "Opera"),
    ("!qb", "qBittorrent"),
    ("!ut", "µTorrent"),
    ("bc!", "BitComet"),
];

fn partial_files(dir: &Path, skip_qb: bool, out: &mut Vec<Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some((stem, ext)) = name.rsplit_once('.') else { continue };
        let ext = ext.to_lowercase();
        let Some(&(_, source)) = PARTIAL.iter().find(|(x, _)| *x == ext) else { continue };
        if skip_qb && ext == "!qb" {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let fresh = md.modified().ok().and_then(|t| now.duration_since(t).ok()).map(|d| d < Duration::from_secs(45)).unwrap_or(false);
        if !md.is_file() || !fresh || md.len() == 0 {
            continue; // fichier partiel abandonné
        }
        let key = format!("file:{}", e.path().display());
        let s = speed(&key, md.len());
        out.push(item(source, stem, md.len(), 0, s, "Téléchargement"));
    }
}

// ---------- qBittorrent (API Web locale) ----------
fn qbittorrent(out: &mut Vec<Value>) -> bool {
    let r = tauri::async_runtime::block_on(async {
        let req = crate::net::client()
            .get("http://127.0.0.1:8080/api/v2/torrents/info?filter=downloading")
            .timeout(Duration::from_millis(1200));
        let r = req.send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        r.json::<Value>().await.ok()
    });
    let Some(Value::Array(list)) = r else { return false };
    for t in list.iter().take(6) {
        let size = t["size"].as_u64().unwrap_or(0);
        let done = (t["progress"].as_f64().unwrap_or(0.0) * size as f64) as u64;
        let paused = t["state"].as_str().map(|s| s.contains("paused") || s.contains("stopped")).unwrap_or(false);
        out.push(item(
            "qBittorrent",
            t["name"].as_str().unwrap_or("Torrent"),
            done,
            size,
            t["dlspeed"].as_f64().unwrap_or(0.0),
            if paused { "En pause" } else { "Téléchargement" },
        ));
    }
    true
}

const LAUNCHERS: [(&str, &str); 5] = [
    ("epicgameslauncher.exe", "Epic Games"),
    ("eadesktop.exe", "EA App"),
    ("battle.net.exe", "Battle.net"),
    ("upc.exe", "Ubisoft Connect"),
    ("riotclientservices.exe", "Riot Client"),
];

/// (téléchargements, débit réseau octets/s)
pub fn scan(procs: &HashSet<String>, downloads_dir: Option<&Path>) -> (Vec<Value>, f64) {
    let mut out = Vec::new();
    if procs.contains("steam.exe") {
        steam(&mut out);
    }
    let qb = procs.contains("qbittorrent.exe") && qbittorrent(&mut out);
    if let Some(d) = downloads_dir {
        partial_files(d, qb, &mut out);
    }
    let net = crate::hw::net_down();
    // launchers sans progression publique : débit réseau si un gros téléchargement semble en cours
    if net > 1_500_000.0 {
        let explained: f64 = out.iter().filter_map(|v| v["speed"].as_f64()).sum();
        if explained < net * 0.5 {
            if let Some((_, l)) = LAUNCHERS.iter().find(|(exe, _)| procs.contains(*exe)) {
                out.push(json!({ "source": l, "name": format!("Téléchargement {l}"), "done": 0, "total": 0, "pct": null,
                    "speed": net, "eta": null, "state": "Débit réseau (progression non fournie)", "guess": true }));
            }
        }
    }
    (out, net)
}
