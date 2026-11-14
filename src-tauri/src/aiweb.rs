//! Comptes IA sans clé API : la page officielle (chatgpt.com, claude.ai, gemini.google.com) s'ouvre
//! dans une fenêtre REZ isolée (profil WebView2 dédié, séparé de l'île). La connexion se fait sur le
//! site officiel : REZ ne voit jamais le mot de passe, et la page n'a accès à aucune commande de REZ
//! (aucune capacité Tauri pour ces fenêtres). Le statut se lit via la présence du cookie de session.
use crate::{err, settings, AppState};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// (id, nom, page d'accueil, domaines du compte)
const PROVS: &[(&str, &str, &str, &[&str])] = &[
    ("openai", "ChatGPT", "https://chatgpt.com/", &["chatgpt.com", "openai.com"]),
    ("claude", "Claude", "https://claude.ai/new", &["claude.ai", "anthropic.com"]),
    ("gemini", "Gemini", "https://gemini.google.com/app", &["google.com"]),
];

fn find(p: &str) -> Result<(&'static str, &'static str, &'static str, &'static [&'static str]), String> {
    PROVS.iter().copied().find(|x| x.0 == p).ok_or_else(|| "Fournisseur inconnu".to_string())
}

fn is_session(prov: &str, name: &str) -> bool {
    match prov {
        "openai" => name.starts_with("__Secure-next-auth.session-token"),
        "claude" => name == "sessionKey",
        _ => name == "__Secure-1PSID" || name == "SID",
    }
}

fn on_domain(domain: Option<&str>, domains: &[&str]) -> bool {
    let d = domain.unwrap_or("").trim_start_matches('.');
    domains.iter().any(|x| d == *x || d.ends_with(&format!(".{x}")))
}

fn data_dir(app: &AppHandle) -> PathBuf {
    app.path().app_local_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("REZ")).join("ai-web")
}

fn label(p: &str) -> String {
    format!("ai-{p}")
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Met à jour le statut Connecté / Déconnecté d'après les cookies du profil (hors thread principal).
fn refresh(app: &AppHandle, w: &WebviewWindow) {
    let Ok(cookies) = w.cookies() else { return };
    let connected: Vec<String> = PROVS
        .iter()
        .filter(|(id, _, _, doms)| cookies.iter().any(|c| on_domain(c.domain(), doms) && is_session(id, c.name())))
        .map(|p| p.0.to_string())
        .collect();
    set_accounts(app, connected);
}

fn set_accounts(app: &AppHandle, connected: Vec<String>) {
    let st = app.state::<AppState>();
    let snapshot = {
        let mut s = st.settings.lock().unwrap();
        if s.ai_accounts == connected {
            return;
        }
        s.ai_accounts = connected.clone();
        s.clone()
    };
    let _ = settings::save(app, &snapshot);
    let _ = app.emit("rez://ai-acc", connected);
}

/// Surveille la fenêtre tant qu'elle est ouverte (connexion / déconnexion faite sur le site).
fn watch(app: AppHandle, lbl: String) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        let Some(w) = app.get_webview_window(&lbl) else { return };
        refresh(&app, &w);
    });
}

/// Ouvre (ou ramène) la fenêtre officielle. Avec `prompt`, la question est envoyée dans la page
/// (paramètre ?q= de ChatGPT et Claude ; copiée dans le presse-papiers pour Gemini).
pub fn open(app: &AppHandle, prov: &str, prompt: Option<String>) -> Result<bool, String> {
    let (id, name, home, _) = find(prov)?;
    let q = prompt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut copied = false;
    let url = match (id, &q) {
        ("openai", Some(q)) => format!("https://chatgpt.com/?q={}", enc(q)),
        ("claude", Some(q)) => format!("https://claude.ai/new?q={}", enc(q)),
        (_, Some(q)) => {
            copied = crate::clip::write_text(q).is_ok();
            home.to_string()
        }
        _ => home.to_string(),
    };
    let url: tauri::Url = url.parse().map_err(err)?;
    let lbl = label(id);
    if let Some(w) = app.get_webview_window(&lbl) {
        if q.is_some() {
            let _ = w.navigate(url);
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(copied);
    }
    let a = app.clone();
    WebviewWindowBuilder::new(app, &lbl, WebviewUrl::External(url))
        .title(format!("{name} · REZ"))
        .inner_size(480.0, 760.0)
        .min_inner_size(360.0, 480.0)
        .data_directory(data_dir(app))
        .on_page_load(move |w, p| {
            if matches!(p.event(), tauri::webview::PageLoadEvent::Finished) {
                let a = a.clone();
                std::thread::spawn(move || refresh(&a, &w));
            }
        })
        .build()
        .map_err(err)?;
    watch(app.clone(), lbl);
    Ok(copied)
}

/// Déconnexion : efface les cookies du compte dans le profil REZ (le compte lui-même n'est pas touché).
pub fn logout(app: &AppHandle, prov: &str) -> Result<(), String> {
    let (id, _, home, doms) = find(prov)?;
    let lbl = label(id);
    let existing = app.get_webview_window(&lbl).or_else(|| {
        PROVS.iter().find_map(|p| app.get_webview_window(&label(p.0)))
    });
    let (w, temp) = match existing {
        Some(w) => (w, false),
        None => {
            let w = WebviewWindowBuilder::new(app, "ai-clean", WebviewUrl::External("about:blank".parse().map_err(err)?))
                .visible(false)
                .skip_taskbar(true)
                .data_directory(data_dir(app))
                .build()
                .map_err(err)?;
            (w, true)
        }
    };
    let app = app.clone();
    let home: tauri::Url = home.parse().map_err(err)?;
    std::thread::spawn(move || {
        if let Ok(cookies) = w.cookies() {
            for c in cookies.into_iter().filter(|c| on_domain(c.domain(), doms)) {
                let _ = w.delete_cookie(c);
            }
        }
        let mut now = app.state::<AppState>().settings.lock().unwrap().ai_accounts.clone();
        now.retain(|p| p != id);
        set_accounts(&app, now);
        if temp {
            let _ = w.destroy();
        } else if w.label() == lbl {
            let _ = w.navigate(home);
        }
    });
    Ok(())
}
