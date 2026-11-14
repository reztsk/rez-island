//! Presse-papiers : historique texte gardé uniquement en mémoire (jamais écrit sur disque).
//! Respecte les marqueurs Windows des gestionnaires de mots de passe
//! (ExcludeClipboardContentFromMonitorProcessing / CanIncludeInClipboardHistory = 0).
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};

const CF_UNICODETEXT: u32 = 13;
const MAX_ITEMS: usize = 30;
const MAX_CHARS: usize = 20_000;

static HIST: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

struct Open;
impl Open {
    fn new() -> Option<Open> {
        for _ in 0..5 {
            if unsafe { OpenClipboard(HWND::default()) }.is_ok() {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        None
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn excluded() -> bool {
    unsafe {
        for f in [w!("ExcludeClipboardContentFromMonitorProcessing"), w!("Clipboard Viewer Ignore")] {
            let id = RegisterClipboardFormatW(f);
            if id != 0 && IsClipboardFormatAvailable(id).is_ok() {
                return true;
            }
        }
        let id = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if id != 0 && IsClipboardFormatAvailable(id).is_ok() {
            if let Ok(h) = GetClipboardData(id) {
                let g = HGLOBAL(h.0);
                let p = GlobalLock(g) as *const u32;
                let deny = !p.is_null() && GlobalSize(g) >= 4 && *p == 0;
                let _ = GlobalUnlock(g);
                if deny {
                    return true;
                }
            }
        }
    }
    false
}

fn read_text() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        let _o = Open::new()?;
        if excluded() {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let g = HGLOBAL(h.0);
        let p = GlobalLock(g) as *const u16;
        if p.is_null() {
            return None;
        }
        let max = GlobalSize(g) / 2;
        let mut n = 0;
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n.min(MAX_CHARS)));
        let _ = GlobalUnlock(g);
        Some(s)
    }
}

pub fn write_text(text: &str) -> Result<(), String> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _o = Open::new().ok_or("Presse-papiers occupé")?;
        EmptyClipboard().map_err(crate::err)?;
        let g = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(crate::err)?;
        let p = GlobalLock(g) as *mut u16;
        if p.is_null() {
            return Err("Mémoire indisponible".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(g);
        SetClipboardData(CF_UNICODETEXT, HANDLE(g.0)).map_err(crate::err)?;
    }
    Ok(())
}

pub fn list() -> Value {
    let h = HIST.lock().unwrap();
    Value::Array(h.iter().map(|(t, at)| json!({ "text": t, "at": at })).collect())
}

pub fn clear() {
    HIST.lock().unwrap().clear();
}

pub fn remove(i: usize) {
    let mut h = HIST.lock().unwrap();
    if i < h.len() {
        h.remove(i);
    }
}

fn push(t: String) -> bool {
    if t.trim().is_empty() {
        return false;
    }
    let mut h = HIST.lock().unwrap();
    if h.first().map(|(x, _)| x == &t).unwrap_or(false) {
        return false;
    }
    h.retain(|(x, _)| x != &t);
    h.insert(0, (t, crate::now_ms()));
    h.truncate(MAX_ITEMS);
    true
}

/// Surveille le numéro de séquence du presse-papiers (appel système trivial) toutes les 700 ms.
pub fn run(app: AppHandle) {
    let mut last = unsafe { GetClipboardSequenceNumber() };
    if let Some(t) = read_text() {
        push(t);
    }
    loop {
        std::thread::sleep(Duration::from_millis(700));
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq == last {
            continue;
        }
        last = seq;
        std::thread::sleep(Duration::from_millis(60)); // laisse l'appli source finir d'écrire
        if let Some(t) = read_text() {
            if push(t) {
                let _ = app.emit("rez://clip", list());
            }
        }
    }
}
