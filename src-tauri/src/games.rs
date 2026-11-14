//! Détection du jeu en cours par NOM de processus uniquement (comme le Gestionnaire des tâches).
//! Aucun handle ouvert sur les jeux, aucune lecture mémoire, aucun accès à leurs dossiers.
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// (exe en minuscules, id, nom affiché)
const GAMES: &[(&str, &str, &str)] = &[
    ("fortniteclient-win64-shipping.exe", "fortnite", "Fortnite"),
    ("rainbowsix.exe", "r6", "Rainbow Six Siege"),
    ("rainbowsix_be.exe", "r6", "Rainbow Six Siege"),
    ("rainbowsix_vulkan.exe", "r6", "Rainbow Six Siege"),
    ("valorant-win64-shipping.exe", "valorant", "Valorant"),
    ("r5apex.exe", "apex", "Apex Legends"),
    ("r5apex_dx12.exe", "apex", "Apex Legends"),
    ("rocketleague.exe", "rl", "Rocket League"),
    ("cs2.exe", "cs2", "Counter-Strike 2"),
    ("league of legends.exe", "lol", "League of Legends"),
    ("overwatch.exe", "ow", "Overwatch 2"),
    ("gta5.exe", "gta", "GTA V"),
    ("gta5_enhanced.exe", "gta", "GTA V"),
    ("minecraft.windows.exe", "minecraft", "Minecraft"),
    ("eldenring.exe", "eldenring", "Elden Ring"),
];
const DISCORD: &[&str] = &["discord.exe", "discordptb.exe", "discordcanary.exe"];

#[derive(Default, Clone, PartialEq)]
pub struct Scan {
    pub discord: bool,
    pub game: Option<(&'static str, &'static str)>,
}

pub fn scan() -> Scan {
    let mut out = Scan::default();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            if DISCORD.contains(&name.as_str()) {
                out.discord = true;
            }
            if out.game.is_none() {
                if let Some(g) = GAMES.iter().find(|g| g.0 == name) {
                    out.game = Some((g.1, g.2));
                }
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}
