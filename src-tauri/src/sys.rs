//! Widgets système (CPU / RAM) + Mode Focus (coupe les notifications Windows de Discord).
use std::collections::BTreeMap;
use std::sync::Mutex;
use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_SUCCESS, FILETIME};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// (cpu %, ram %, ram utilisée Go, ram totale Go)
pub fn info() -> (f64, f64, f64, f64) {
    static LAST: Mutex<(u64, u64)> = Mutex::new((0, 0));
    let mut cpu = 0.0;
    unsafe {
        let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u)).is_ok() {
            let (idle, total) = (ft(i), ft(k) + ft(u));
            let mut l = LAST.lock().unwrap();
            if l.1 > 0 && total > l.1 {
                let (di, dt) = (idle.saturating_sub(l.0) as f64, (total - l.1) as f64);
                cpu = ((1.0 - di / dt) * 100.0).clamp(0.0, 100.0);
            }
            *l = (idle, total);
        }
    }
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let (mut used, mut total) = (0.0, 0.0);
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
        total = m.ullTotalPhys as f64 / 1073741824.0;
        used = total - m.ullAvailPhys as f64 / 1073741824.0;
    }
    let pct = if total > 0.0 { used / total * 100.0 } else { 0.0 };
    (cpu, pct, used, total)
}

// ---------- Mode Focus ----------
// Même réglage que Paramètres Windows > Notifications > Discord (HKCU, sans admin), restauré à la fin.
const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\";
const APPS: [&str; 3] = [
    "com.squirrel.Discord.Discord",
    "com.squirrel.DiscordPTB.DiscordPTB",
    "com.squirrel.DiscordCanary.DiscordCanary",
];

fn get(app: &str) -> i64 {
    let (key, val) = (HSTRING::from(format!("{BASE}{app}")), HSTRING::from("Enabled"));
    let mut data = 0u32;
    let mut len = 4u32;
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, &key, &val, RRF_RT_REG_DWORD, None, Some(&mut data as *mut u32 as _), Some(&mut len))
    };
    if r == ERROR_SUCCESS { data as i64 } else { -1 }
}

fn set(app: &str, v: i64) {
    let (key, val) = (HSTRING::from(format!("{BASE}{app}")), HSTRING::from("Enabled"));
    unsafe {
        if v < 0 {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &key, &val);
        } else {
            let d = v as u32;
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, &key, &val, REG_DWORD.0, Some(&d as *const u32 as _), 4);
        }
    }
}

/// Coupe les notifications Discord ; renvoie les valeurs d'origine (-1 = absente).
pub fn focus_mute() -> BTreeMap<String, i64> {
    APPS.iter()
        .map(|a| {
            let prev = get(a);
            set(a, 0);
            (a.to_string(), prev)
        })
        .collect()
}

pub fn focus_restore(prev: &BTreeMap<String, i64>) {
    for (a, v) in prev {
        set(a, *v);
    }
}

/// Anti-veille : un fil dédié garde l'état (SetThreadExecutionState est lié au thread appelant).
pub fn keep_awake(on: bool) {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::OnceLock;
    use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
    static TX: OnceLock<Mutex<Sender<bool>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<bool>();
        std::thread::spawn(move || {
            for on in rx {
                let f = if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED } else { ES_CONTINUOUS };
                unsafe { SetThreadExecutionState(f) };
            }
        });
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap().send(on);
}
