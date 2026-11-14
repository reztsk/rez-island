//! Mélangeur audio : sessions audio Windows (comme le mélangeur de volume du système).
//! Le volume est réglé dans le moteur audio de Windows : aucun handle sur les applis ni les jeux.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use windows::core::{Interface, GUID};
use windows::Win32::Foundation::S_OK;
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

struct Sess {
    key: String,
    vol: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
    active: bool,
}

fn sessions() -> Result<Vec<Sess>, String> {
    let names = crate::games::names_by_pid();
    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(crate::err)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(crate::err)?;
        let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None).map_err(crate::err)?;
        let list = mgr.GetSessionEnumerator().map_err(crate::err)?;
        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(c) = list.GetSession(i) else { continue };
            let state = c.GetState().unwrap_or(AudioSessionStateExpired);
            if state == AudioSessionStateExpired {
                continue;
            }
            let Ok(c2) = c.cast::<IAudioSessionControl2>() else { continue };
            let key = if c2.IsSystemSoundsSession() == S_OK {
                "#system".to_string()
            } else {
                match names.get(&c2.GetProcessId().unwrap_or(0)) {
                    Some(n) => n.clone(),
                    None => continue, // processus terminé
                }
            };
            let Ok(vol) = c.cast::<ISimpleAudioVolume>() else { continue };
            out.push(Sess { key, vol, meter: c.cast().ok(), active: state == AudioSessionStateActive });
        }
    }
    Ok(out)
}

/// Une ligne par application : volume, muet, niveau instantané, en lecture.
pub fn list() -> Result<Vec<Value>, String> {
    let mut by: BTreeMap<String, (f32, bool, f32, bool)> = BTreeMap::new();
    for s in sessions()? {
        let (v, m, p) = unsafe {
            (
                s.vol.GetMasterVolume().unwrap_or(1.0),
                s.vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                s.meter.as_ref().and_then(|x| x.GetPeakValue().ok()).unwrap_or(0.0),
            )
        };
        let e = by.entry(s.key).or_insert((v, m, 0.0, false));
        e.2 = e.2.max(p);
        e.3 |= s.active;
    }
    let mut v: Vec<Value> = by
        .into_iter()
        .map(|(k, (vol, muted, peak, active))| json!({ "key": k, "volume": vol, "muted": muted, "peak": peak, "active": active }))
        .collect();
    // en lecture d'abord, puis par nom
    v.sort_by_key(|x| (!x["active"].as_bool().unwrap_or(false), x["key"] == "#system"));
    Ok(v)
}

/// Règle le volume et/ou le muet de toutes les sessions d'une application.
pub fn set(key: &str, volume: Option<f32>, muted: Option<bool>) -> Result<(), String> {
    let mut found = false;
    for s in sessions()?.into_iter().filter(|s| s.key == key) {
        found = true;
        unsafe {
            if let Some(v) = volume {
                s.vol.SetMasterVolume(v.clamp(0.0, 1.0), &GUID::zeroed()).map_err(crate::err)?;
            }
            if let Some(m) = muted {
                s.vol.SetMute(BOOL::from(m), &GUID::zeroed()).map_err(crate::err)?;
            }
        }
    }
    if found { Ok(()) } else { Err("Cette application ne joue plus de son".into()) }
}
