//! Auto-update 1 clic : version.json sur GitHub -> télécharge REZ.exe -> vérifie SHA-256 -> remplace -> redémarre.
use crate::{err, net::client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

// Remplacé automatiquement par scripts/setup-github.ps1
pub const VERSION_URL: &str = "https://raw.githubusercontent.com/GITHUB_USER/REZ/main/version.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct Remote {
    pub version: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Serialize)]
pub struct Check {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub notes: String,
}

pub async fn check(current: &semver::Version) -> Result<(Check, Remote), String> {
    if VERSION_URL.contains("GITHUB_USER") {
        return Err("Dépôt GitHub non configuré (scripts/setup-github.ps1)".into());
    }
    let url = format!("{VERSION_URL}?t={}", crate::now_ms());
    let r: Remote = client()
        .get(url)
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let latest = semver::Version::parse(r.version.trim().trim_start_matches('v')).map_err(err)?;
    let check = Check {
        current: current.to_string(),
        latest: latest.to_string(),
        available: latest > *current,
        notes: r.notes.clone(),
    };
    Ok((check, r))
}

pub async fn install(r: Remote) -> Result<(), String> {
    if !r.url.starts_with("https://github.com/") {
        return Err("URL de mise à jour non autorisée".into());
    }
    if r.sha256.trim().len() != 64 {
        return Err("SHA-256 absent de version.json".into());
    }
    let bytes = client()
        .get(&r.url)
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?
        .bytes()
        .await
        .map_err(err)?;

    let hex: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if !hex.eq_ignore_ascii_case(r.sha256.trim()) {
        return Err("Signature SHA-256 invalide : mise à jour annulée".into());
    }
    if bytes.len() < 2 || &bytes[..2] != b"MZ" {
        return Err("Fichier téléchargé invalide".into());
    }

    let exe = std::env::current_exe().map_err(err)?;
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");
    fs::write(&new, &bytes).map_err(err)?;
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(err)?; // Windows autorise le renommage d'un exe en cours
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(e.to_string());
    }

    // relance après la fermeture de l'instance actuelle (single-instance)
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("cmd")
        .raw_arg(format!("/C ping -n 3 127.0.0.1 >NUL & start \"\" \"{}\"", exe.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(err)?;
    Ok(())
}

pub fn cleanup_old() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(5));
        if let Ok(exe) = std::env::current_exe() {
            let _ = fs::remove_file(exe.with_extension("old"));
            let _ = fs::remove_file(exe.with_extension("new"));
        }
    });
}
