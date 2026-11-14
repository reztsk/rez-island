//! Périphériques sans fil + niveau de batterie (et charge en cours).
//! Sources publiques uniquement, comme Windows et les logiciels des fabricants :
//!  - batterie Bluetooth publiée par Windows (Paramètres > Bluetooth),
//!  - Windows.Gaming.Input (manettes Xbox),
//!  - protocole HID++ de Logitech en LECTURE seule (comme G HUB / Solaar),
//!  - batterie du PC portable.
//! Aucun pilote, aucun accès aux jeux ni à leur mémoire : zéro risque anti-cheat.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CM_Get_Device_Interface_ListW,
    CM_Get_Device_Interface_List_SizeW, CM_Locate_DevNodeW, CM_GETIDLIST_FILTER_PRESENT,
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetPreparsedData, HidD_GetProductString, HidP_GetCaps,
    HIDD_ATTRIBUTES, HIDP_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Devices::Properties::{DEVPROPKEY, DEVPROPTYPE};
use windows::Win32::Foundation::{CloseHandle, BOOL, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

const HID_GUID: GUID = GUID::from_u128(0x4D1E55B2_F16F_11CF_88CB_001111000030);
/// Niveau de batterie Bluetooth (octet 0-100) publié par Windows.
const BT_BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104EA319_6EE2_4701_BD47_8DDBF425BBE5), pid: 2 };
const FRIENDLY_NAME: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };
const DEV_NAME: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9ebac0), pid: 10 };

#[derive(Clone)]
struct Dev {
    id: String,
    name: String,
    kind: &'static str,
    pct: Option<u8>,
    charging: bool,
    full: bool,
    approx: bool,
    online: bool,
    src: &'static str,
}

impl Dev {
    fn json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "kind": self.kind, "pct": self.pct, "charging": self.charging,
            "full": self.full, "approx": self.approx, "online": self.online, "src": self.src,
        })
    }
}

/// Type d'appareil deviné d'après son nom (icône).
fn kind_of(name: &str) -> &'static str {
    let n = name.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|k| n.contains(k));
    if has(&["headset", "casque", "headphone", "buds", "airpods", "écouteur", "earbud", "wh-", "wf-", "arctis", "hyperx cloud", "kraken"]) {
        "headset"
    } else if has(&["controller", "manette", "gamepad", "xbox", "dualsense", "dualshock", "joy-con"]) {
        "gamepad"
    } else if has(&["keyboard", "clavier", "keys", "kbd"]) {
        "keyboard"
    } else if has(&["mouse", "souris", "mx master", "mx anywhere", "viper", "deathadder", "superlight"]) {
        "mouse"
    } else if has(&["speaker", "enceinte", "soundbar", "jbl", "boom"]) {
        "speaker"
    } else if has(&["pen", "stylet", "stylus"]) {
        "pen"
    } else {
        "other"
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

// ---------- Bluetooth (propriété publiée par Windows) ----------
fn devnode_prop(dn: u32, key: &DEVPROPKEY) -> Option<Vec<u8>> {
    unsafe {
        let mut ty = DEVPROPTYPE::default();
        let mut size = 0u32;
        let _ = CM_Get_DevNode_PropertyW(dn, key, &mut ty, None, &mut size, 0);
        if size == 0 || size > 4096 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if CM_Get_DevNode_PropertyW(dn, key, &mut ty, Some(buf.as_mut_ptr()), &mut size, 0) != CR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }
}

fn prop_str(dn: u32, key: &DEVPROPKEY) -> Option<String> {
    let b = devnode_prop(dn, key)?;
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let s = String::from_utf16_lossy(&w).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn bluetooth(out: &mut Vec<Dev>) {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_ID_List_SizeW(&mut len, PCWSTR::null(), CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS || len == 0 {
            return;
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(PCWSTR::null(), &mut buf, CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS {
            return;
        }
        let mut best: HashMap<String, Dev> = HashMap::new();
        for id in multi_sz(&buf) {
            let up = id.to_ascii_uppercase();
            if !(up.starts_with("BTHENUM\\") || up.starts_with("BTHLE\\") || up.starts_with("BTHLEDEVICE\\")) {
                continue;
            }
            let w = wide(&id);
            let mut dn = 0u32;
            if CM_Locate_DevNodeW(&mut dn, PCWSTR(w.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
                continue;
            }
            let Some(b) = devnode_prop(dn, &BT_BATTERY) else { continue };
            let Some(&pct) = b.first() else { continue };
            if pct > 100 {
                continue;
            }
            let name = prop_str(dn, &FRIENDLY_NAME).or_else(|| prop_str(dn, &DEV_NAME)).unwrap_or_else(|| "Appareil Bluetooth".into());
            let key = name.to_lowercase();
            best.entry(key.clone()).or_insert_with(|| Dev {
                id: format!("bt:{key}"),
                kind: kind_of(&name),
                name,
                pct: Some(pct),
                charging: false,
                full: false,
                approx: false,
                online: true,
                src: "Bluetooth",
            });
        }
        out.extend(best.into_values());
    }
}

// ---------- manettes Xbox (Windows.Gaming.Input) ----------
fn gamepads(out: &mut Vec<Dev>) {
    use windows::core::Interface;
    use windows::Gaming::Input::{Gamepad, IGameController, RawGameController};
    use windows::System::Power::BatteryStatus;
    let Ok(pads) = Gamepad::Gamepads() else { return };
    for i in 0..pads.Size().unwrap_or(0) {
        let Ok(p) = pads.GetAt(i) else { continue };
        let Ok(r) = p.TryGetBatteryReport() else { continue };
        let st = r.Status().unwrap_or(BatteryStatus::NotPresent);
        if st == BatteryStatus::NotPresent {
            continue; // manette filaire
        }
        let rem = r.RemainingCapacityInMilliwattHours().and_then(|v| v.Value()).ok();
        let cap = r.FullChargeCapacityInMilliwattHours().and_then(|v| v.Value()).ok();
        let pct = match (rem, cap) {
            (Some(a), Some(b)) if b > 0 => Some((a as f64 / b as f64 * 100.0).round().clamp(0.0, 100.0) as u8),
            _ => None,
        };
        let name = p
            .cast::<IGameController>()
            .ok()
            .and_then(|g| RawGameController::FromGameController(&g).ok())
            .and_then(|c| c.DisplayName().ok())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Manette Xbox".into());
        out.push(Dev {
            id: format!("pad:{i}"),
            name,
            kind: "gamepad",
            pct,
            charging: st == BatteryStatus::Charging,
            full: st == BatteryStatus::Idle && pct == Some(100),
            approx: false,
            online: true,
            src: "Xbox",
        });
    }
}

// ---------- batterie du PC portable ----------
fn laptop(out: &mut Vec<Dev>) {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() || s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
        return;
    }
    let ac = s.ACLineStatus == 1;
    out.push(Dev {
        id: "pc".into(),
        name: "Batterie du PC".into(),
        kind: "laptop",
        pct: Some(s.BatteryLifePercent),
        charging: s.BatteryFlag & 8 != 0,
        full: ac && s.BatteryLifePercent >= 100,
        approx: false,
        online: true,
        src: if ac { "Secteur" } else { "Sur batterie" },
    });
}

// ---------- HID : Logitech HID++ (lecture seule) + récepteurs sans fil ----------
struct HidInfo {
    path: String,
    vid: u16,
    pid: u16,
    page: u16,
    usage: u16,
    out_len: u16,
    product: String,
}

fn hid_paths() -> Vec<String> {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &HID_GUID, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS
            || len < 2
        {
            return vec![];
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&HID_GUID, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return vec![];
        }
        multi_sz(&buf)
    }
}

fn open_file(path: &str, access: u32, overlapped: bool) -> Option<HANDLE> {
    let w = wide(path);
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            if overlapped { FILE_FLAG_OVERLAPPED } else { Default::default() },
            None,
        )
        .ok()
        .filter(|h| !h.is_invalid())
    }
}

/// Attributs d'une collection HID (ouverte SANS droit de lecture/écriture : simple description).
fn hid_info(path: &str) -> Option<HidInfo> {
    let h = open_file(path, 0, false)?;
    let r = unsafe {
        let mut a = HIDD_ATTRIBUTES { Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
        let mut caps = HIDP_CAPS::default();
        let mut pp = PHIDP_PREPARSED_DATA::default();
        let ok = HidD_GetAttributes(h, &mut a).as_bool() && HidD_GetPreparsedData(h, &mut pp).as_bool();
        if ok {
            let _ = HidP_GetCaps(pp, &mut caps);
            let _ = HidD_FreePreparsedData(pp);
        }
        let mut name = [0u16; 128];
        let product = if HidD_GetProductString(h, name.as_mut_ptr() as *mut _, 256).as_bool() {
            let n = name.iter().position(|&c| c == 0).unwrap_or(128);
            String::from_utf16_lossy(&name[..n]).trim().to_string()
        } else {
            String::new()
        };
        ok.then(|| HidInfo {
            path: path.to_string(),
            vid: a.VendorID,
            pid: a.ProductID,
            page: caps.UsagePage,
            usage: caps.Usage,
            out_len: caps.OutputReportByteLength,
            product,
        })
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    r
}

/// Collection HID ouverte en lecture/écriture asynchrone (délais garantis).
struct Hid {
    h: HANDLE,
    ev: HANDLE,
    out_len: usize,
}

impl Hid {
    fn open(path: &str, out_len: u16) -> Option<Hid> {
        let h = open_file(path, (GENERIC_READ | GENERIC_WRITE).0, true)?;
        let ev = unsafe { CreateEventW(None, BOOL(1), BOOL(0), PCWSTR::null()) };
        match ev {
            Ok(ev) => Some(Hid { h, ev, out_len: out_len.max(20) as usize }),
            Err(_) => {
                unsafe {
                    let _ = CloseHandle(h);
                }
                None
            }
        }
    }

    /// Lance une E/S et attend au plus `ms` ; annulée proprement sinon.
    fn io(&self, buf: &mut [u8], write: bool, ms: u32) -> Option<usize> {
        unsafe {
            let _ = ResetEvent(self.ev);
            let mut ov = OVERLAPPED { hEvent: self.ev, ..Default::default() };
            let mut n = 0u32;
            let started = if write {
                WriteFile(self.h, Some(buf), None, Some(&mut ov))
            } else {
                ReadFile(self.h, Some(buf), None, Some(&mut ov))
            };
            if let Err(e) = started {
                if e.code() != windows::Win32::Foundation::ERROR_IO_PENDING.to_hresult() {
                    return None;
                }
            }
            if WaitForSingleObject(self.ev, ms) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(self.h, Some(&ov));
                let _ = GetOverlappedResult(self.h, &ov, &mut n, BOOL(1));
                return None;
            }
            GetOverlappedResult(self.h, &ov, &mut n, BOOL(0)).ok()?;
            Some(n as usize)
        }
    }

    /// Requête HID++ 2.0 (rapport long 0x11) ; ignore le trafic des autres logiciels (G HUB…).
    fn hidpp(&self, dev: u8, feat: u8, func: u8, params: &[u8]) -> Option<[u8; 16]> {
        const SW: u8 = 0x0B;
        let fs = (func << 4) | SW;
        let mut out = vec![0u8; self.out_len];
        out[..4].copy_from_slice(&[0x11, dev, feat, fs]);
        out[4..4 + params.len().min(16)].copy_from_slice(&params[..params.len().min(16)]);
        self.io(&mut out, true, 400)?;
        let end = Instant::now() + Duration::from_millis(650);
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            let mut r = [0u8; 64];
            let n = self.io(&mut r, false, left.as_millis() as u32 + 1)?;
            if n < 20 || r[0] != 0x11 || r[1] != dev {
                continue;
            }
            if r[2] == feat && r[3] == fs {
                let mut p = [0u8; 16];
                p.copy_from_slice(&r[4..20]);
                return Some(p);
            }
            if r[2] == 0xFF && r[3] == feat && r[4] == fs {
                return None; // erreur HID++ (fonction non supportée…)
            }
        }
    }

    fn feature(&self, dev: u8, id: u16) -> Option<u8> {
        let r = self.hidpp(dev, 0, 0, &[(id >> 8) as u8, id as u8])?;
        (r[0] != 0).then_some(r[0])
    }
}

impl Drop for Hid {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.h);
            let _ = CloseHandle(self.ev);
        }
    }
}

/// Tension (mV) -> % approximatif (courbe Li-ion des appareils Logitech).
fn volt_pct(mv: u16) -> u8 {
    const T: [(u16, f64); 12] = [
        (4150, 100.0), (4050, 90.0), (3990, 80.0), (3930, 70.0), (3870, 60.0), (3820, 50.0),
        (3790, 40.0), (3760, 30.0), (3730, 20.0), (3690, 10.0), (3650, 5.0), (3500, 0.0),
    ];
    if mv >= T[0].0 {
        return 100;
    }
    for w in T.windows(2) {
        let ((v1, p1), (v0, p0)) = (w[0], w[1]);
        if mv >= v0 {
            return (p0 + (p1 - p0) * (mv - v0) as f64 / (v1 - v0) as f64).round() as u8;
        }
    }
    0
}

/// (%, en charge, plein, approximatif) via les fonctions batterie HID++ connues.
fn hidpp_battery(h: &Hid, dev: u8) -> Option<(Option<u8>, bool, bool, bool)> {
    // ping (IRoot.getProtocolVersion) : pas de réponse = appareil éteint / hors de portée
    h.hidpp(dev, 0, 1, &[0, 0, 0x5A])?;
    if let Some(i) = h.feature(dev, 0x1004) {
        // Unified Battery
        let r = h.hidpp(dev, i, 1, &[])?;
        let st = r[2];
        let mut pct = r[0];
        let mut approx = false;
        if pct == 0 && r[1] != 0 {
            approx = true;
            pct = if r[1] & 8 != 0 { 90 } else if r[1] & 4 != 0 { 60 } else if r[1] & 2 != 0 { 25 } else { 5 };
        }
        return Some((Some(pct.min(100)), st == 1 || st == 2, st == 3, approx));
    }
    if let Some(i) = h.feature(dev, 0x1000) {
        // Battery Level Status
        let r = h.hidpp(dev, i, 0, &[])?;
        let st = r[2];
        let full = st == 3;
        let pct = if r[0] == 0 && full { 100 } else { r[0].min(100) };
        return Some((Some(pct), matches!(st, 1 | 2 | 4), full, false));
    }
    if let Some(i) = h.feature(dev, 0x1001) {
        // Battery Voltage
        let r = h.hidpp(dev, i, 0, &[])?;
        let mv = u16::from_be_bytes([r[0], r[1]]);
        let charging = r[2] & 0x80 != 0;
        let full = charging && r[2] & 0x07 == 2;
        return Some(((mv > 0).then(|| volt_pct(mv)), charging && !full, full, true));
    }
    if let Some(i) = h.feature(dev, 0x1F20) {
        // ADC Measurement (casques G PRO X, G733, G535…)
        let r = h.hidpp(dev, i, 0, &[])?;
        let mv = u16::from_be_bytes([r[0], r[1]]);
        if mv == 0 {
            return None;
        }
        let flags = r[2];
        let full = flags & 0x04 != 0 && flags & 0x02 != 0 && volt_pct(mv) >= 95;
        return Some((Some(volt_pct(mv)), flags & 0x02 != 0 && !full, full, true));
    }
    Some((None, false, false, false))
}

/// Nom + type annoncés par l'appareil (fonction HID++ 0x0005).
fn hidpp_name(h: &Hid, dev: u8) -> Option<(String, &'static str)> {
    let i = h.feature(dev, 0x0005)?;
    let len = h.hidpp(dev, i, 0, &[])?[0] as usize;
    let mut name = Vec::new();
    while name.len() < len.min(64) {
        let r = h.hidpp(dev, i, 1, &[name.len() as u8])?;
        let take = (len - name.len()).min(16);
        name.extend_from_slice(&r[..take]);
    }
    let kind = match h.hidpp(dev, i, 2, &[]).map(|r| r[0]) {
        Some(0) | Some(2) => "keyboard",
        Some(3) | Some(4) | Some(5) => "mouse",
        Some(8) => "headset",
        Some(6) => "pen",
        _ => "other",
    };
    let s = String::from_utf8_lossy(&name).trim_matches(char::from(0)).trim().to_string();
    (!s.is_empty()).then_some((s, kind))
}

/// Récepteurs Logitech (Unifying / Bolt / Lightspeed) : appareils aux index 1..6.
const RECEIVERS: [u16; 12] = [0xC52B, 0xC532, 0xC534, 0xC539, 0xC53A, 0xC53D, 0xC53F, 0xC541, 0xC545, 0xC547, 0xC548, 0xC54D];

fn logitech(infos: &[HidInfo], out: &mut Vec<Dev>) {
    let mut done = std::collections::HashSet::new();
    for c in infos.iter().filter(|c| c.vid == 0x046D && c.out_len >= 20 && matches!((c.page, c.usage), (0xFF43, 0x0202) | (0xFF00, 0x0002))) {
        if !done.insert(c.pid) {
            continue;
        }
        let Some(h) = Hid::open(&c.path, c.out_len) else { continue };
        let receiver = RECEIVERS.contains(&c.pid);
        let slots: Vec<u8> = if receiver { (1..=6).collect() } else { vec![0xFF] };
        for dev in slots {
            let bat = hidpp_battery(&h, dev);
            if receiver && bat.is_none() {
                continue; // emplacement vide ou appareil éteint
            }
            let (name, kind) = match (receiver, bat.is_some()) {
                (false, _) if !c.product.is_empty() => (c.product.clone(), kind_of(&c.product)),
                _ => hidpp_name(&h, dev).unwrap_or_else(|| ("Appareil Logitech".into(), "other")),
            };
            let kind = if kind == "other" { kind_of(&name) } else { kind };
            let (pct, charging, full, approx) = bat.unwrap_or((None, false, false, false));
            out.push(Dev {
                id: format!("lg:{:04x}:{dev}", c.pid),
                name: name.replace("Logitech ", "").trim().to_string(),
                kind,
                pct,
                charging,
                full,
                approx,
                online: bat.is_some(),
                src: "Logitech",
            });
        }
    }
}

/// Récepteurs 2,4 GHz d'autres marques : détectés, mais leur batterie passe par un protocole
/// propriétaire non documenté (REZ n'envoie jamais de commande inconnue à un appareil).
fn other_receivers(infos: &[HidInfo], out: &mut Vec<Dev>) {
    let mut seen = std::collections::HashSet::new();
    for c in infos {
        if c.vid == 0x046D || c.product.is_empty() || !seen.insert((c.vid, c.pid)) {
            continue;
        }
        let p = c.product.to_lowercase();
        if !["receiver", "dongle", "wireless", "2.4g", "sans fil", "récepteur"].iter().any(|k| p.contains(k)) {
            continue;
        }
        let mut kind = kind_of(&c.product);
        if kind == "other" {
            let usages: Vec<(u16, u16)> = infos.iter().filter(|x| x.vid == c.vid && x.pid == c.pid).map(|x| (x.page, x.usage)).collect();
            kind = if usages.contains(&(1, 2)) { "mouse" } else if usages.contains(&(1, 6)) { "keyboard" } else { "other" };
        }
        let name = c
            .product
            .split_whitespace()
            .filter(|w| !matches!(w.to_lowercase().as_str(), "receiver" | "dongle" | "récepteur"))
            .collect::<Vec<_>>()
            .join(" ");
        out.push(Dev {
            id: format!("hid:{:04x}:{:04x}", c.vid, c.pid),
            name: if name.is_empty() { c.product.clone() } else { name },
            kind,
            pct: None,
            charging: false,
            full: false,
            approx: false,
            online: true,
            src: "2,4 GHz",
        });
    }
}

fn scan() -> Vec<Value> {
    let mut out = Vec::new();
    laptop(&mut out);
    let infos: Vec<HidInfo> = hid_paths().iter().filter_map(|p| hid_info(p)).collect();
    logitech(&infos, &mut out);
    gamepads(&mut out);
    bluetooth(&mut out);
    other_receivers(&infos, &mut out);
    // appareils avec batterie connue d'abord, puis par nom
    out.sort_by(|a, b| (a.pct.is_none(), !a.online, &a.name).cmp(&(b.pct.is_none(), !b.online, &b.name)));
    out.iter().map(Dev::json).collect()
}

/// Liste des périphériques (cache de 20 s : la batterie évolue lentement).
pub fn list(force: bool) -> Vec<Value> {
    static CACHE: Mutex<Option<(Instant, Vec<Value>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    if let Some((t, v)) = c.as_ref() {
        if !force && t.elapsed() < Duration::from_secs(20) {
            return v.clone();
        }
    }
    let v = scan();
    *c = Some((Instant::now(), v.clone()));
    v
}

/// Windows.Gaming.Input remplit la liste des manettes de façon asynchrone : premier appel à vide.
pub fn warm_up() {
    let _ = windows::Gaming::Input::Gamepad::Gamepads();
}
