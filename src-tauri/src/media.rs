//! Multimédia via APIs Windows publiques (SMTC + volume système). Aucun accès aux jeux.
use serde::Serialize;
use windows::core::{Result as WinResult, GUID};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

#[derive(Serialize, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub app: String,
    pub playing: bool,
    pub has_session: bool,
    pub volume: f32,
    pub muted: bool,
}

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

pub fn info() -> MediaInfo {
    let mut m = MediaInfo::default();
    if let Ok(s) = session() {
        m.has_session = true;
        if let Ok(p) = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
            m.title = p.Title().map(|h| h.to_string()).unwrap_or_default();
            m.artist = p.Artist().map(|h| h.to_string()).unwrap_or_default();
        }
        if let Ok(pi) = s.GetPlaybackInfo() {
            m.playing = pi.PlaybackStatus().map(|x| x == Status::Playing).unwrap_or(false);
        }
        m.app = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
    }
    if let Ok(v) = endpoint() {
        unsafe {
            m.volume = v.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            m.muted = v.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        }
    }
    m
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

pub fn set_volume(v: f32) -> Result<(), String> {
    let ep = endpoint().map_err(crate::err)?;
    unsafe { ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), &GUID::zeroed()).map_err(crate::err) }
}

pub fn set_mute(m: bool) -> Result<(), String> {
    let ep = endpoint().map_err(crate::err)?;
    unsafe { ep.SetMute(BOOL::from(m), &GUID::zeroed()).map_err(crate::err) }
}
