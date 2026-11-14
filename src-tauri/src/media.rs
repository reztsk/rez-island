//! Multimédia via APIs Windows publiques (SMTC + volume système). Aucun accès aux jeux.
use base64::Engine;
use serde::Serialize;
use std::sync::Mutex;
use windows::core::{Result as WinResult, GUID};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Props,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

#[derive(Serialize, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub playing: bool,
    pub has_session: bool,
    pub volume: f32,
    pub muted: bool,
    pub position: f64,
    pub duration: f64,
    pub can_seek: bool,
    /// Identifiant de la pochette en cache (vide = aucune). Le front la récupère via media_thumb.
    pub thumb: String,
}

struct Thumb {
    key: String,
    data: String,
    tries: u8,
}
static THUMB: Mutex<Thumb> = Mutex::new(Thumb { key: String::new(), data: String::new(), tries: 0 });

fn session() -> WinResult<Session> {
    Manager::RequestAsync()?.get()?.GetCurrentSession()
}

fn endpoint() -> WinResult<IAudioEndpointVolume> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        dev.Activate(CLSCTX_ALL, None)
    }
}

/// Pochette fournie par le lecteur (Spotify, navigateur…) -> data URI.
fn read_thumb(p: &Props) -> Option<String> {
    let s = p.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let size = s.Size().ok()? as u32;
    if size == 0 || size > 8 << 20 {
        return None;
    }
    let ct = s.ContentType().map(|h| h.to_string()).unwrap_or_default();
    let r = DataReader::CreateDataReader(&s).ok()?;
    r.LoadAsync(size).ok()?.get().ok()?;
    let mut buf = vec![0u8; size as usize];
    r.ReadBytes(&mut buf).ok()?;
    let mime = if ct.starts_with("image/") {
        ct
    } else if buf.starts_with(&[0x89, b'P']) {
        "image/png".into()
    } else {
        "image/jpeg".into()
    };
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&buf)))
}

/// Temps "maintenant" au format DateTime WinRT (100 ns depuis 1601).
fn win_now() -> i64 {
    crate::now_ms() as i64 * 10_000 + 116_444_736_000_000_000
}

/// (position, début, fin) en unités de 100 ns, position extrapolée si lecture en cours.
fn timeline(s: &Session, playing: bool) -> Option<(i64, i64, i64)> {
    let t = s.GetTimelineProperties().ok()?;
    let start = t.StartTime().map(|d| d.Duration).unwrap_or(0);
    let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
    let mut pos = t.Position().map(|d| d.Duration).unwrap_or(0);
    let upd = t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
    if playing && upd > 0 {
        pos += (win_now() - upd).clamp(0, 6 * 3600 * 10_000_000);
    }
    Some((pos.clamp(start, end.max(start)), start, end))
}

pub fn info() -> MediaInfo {
    let mut m = MediaInfo::default();
    if let Ok(s) = session() {
        m.has_session = true;
        m.app = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        if let Ok(pi) = s.GetPlaybackInfo() {
            m.playing = pi.PlaybackStatus().map(|x| x == Status::Playing).unwrap_or(false);
            m.can_seek = pi.Controls().and_then(|c| c.IsPlaybackPositionEnabled()).unwrap_or(false);
        }
        if let Ok(p) = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
            m.title = p.Title().map(|h| h.to_string()).unwrap_or_default();
            m.artist = p.Artist().map(|h| h.to_string()).unwrap_or_default();
            m.album = p.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
            // pochette relue 3 fois max par piste (le lecteur la met parfois à jour après le titre)
            let key = format!("{}|{}|{}", m.app, m.title, m.artist);
            let mut t = THUMB.lock().unwrap();
            if t.key != key {
                *t = Thumb { key, data: read_thumb(&p).unwrap_or_default(), tries: 1 };
            } else if t.tries < 3 {
                t.tries += 1;
                if let Some(d) = read_thumb(&p) {
                    t.data = d;
                }
            }
            if !t.data.is_empty() {
                m.thumb = format!("{}#{}", t.key, t.data.len());
            }
        }
        if let Some((pos, start, end)) = timeline(&s, m.playing) {
            m.duration = (end - start) as f64 / 1e7;
            m.position = (pos - start) as f64 / 1e7;
        }
    }
    if let Ok(v) = endpoint() {
        unsafe {
            m.volume = v.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            m.muted = v.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        }
    }
    m
}

pub fn thumb() -> String {
    THUMB.lock().unwrap().data.clone()
}

pub fn action(a: &str) -> Result<(), String> {
    let s = session().map_err(|_| "Aucun lecteur actif".to_string())?;
    let op = match a {
        "toggle" => s.TryTogglePlayPauseAsync(),
        "next" => s.TrySkipNextAsync(),
        "prev" => s.TrySkipPreviousAsync(),
        _ => return Err("Action inconnue".into()),
    };
    op.and_then(|o| o.get()).map(|_| ()).map_err(crate::err)
}

/// Avance / recule de `delta` secondes, ou va à `to` secondes.
pub fn seek(delta: f64, to: Option<f64>) -> Result<(), String> {
    let s = session().map_err(|_| "Aucun lecteur actif".to_string())?;
    let playing = s
        .GetPlaybackInfo()
        .and_then(|p| p.PlaybackStatus())
        .map(|x| x == Status::Playing)
        .unwrap_or(false);
    let (pos, start, end) = timeline(&s, playing).ok_or("Position indisponible")?;
    let target = match to {
        Some(t) => start + (t * 1e7) as i64,
        None => pos + (delta * 1e7) as i64,
    };
    let target = if end > start { target.clamp(start, end - 10_000_000) } else { target.max(start) };
    let ok = s
        .TryChangePlaybackPositionAsync(target)
        .and_then(|o| o.get())
        .map_err(crate::err)?;
    if ok { Ok(()) } else { Err("Ce lecteur ne permet pas l'avance rapide".into()) }
}

pub fn set_volume(v: f32) -> Result<(), String> {
    let ep = endpoint().map_err(crate::err)?;
    unsafe { ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), &GUID::zeroed()).map_err(crate::err) }
}

pub fn set_mute(m: bool) -> Result<(), String> {
    let ep = endpoint().map_err(crate::err)?;
    unsafe { ep.SetMute(BOOL::from(m), &GUID::zeroed()).map_err(crate::err) }
}
