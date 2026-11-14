# REZ — Dynamic Island Windows 11 (Tauri)

Overlay glassmorphism sombre, tray, raccourci global, auto-update 1 clic. Ce projet n'utilise pas Electron.

## Anti-cheat (zéro risque de ban)
- Aucune lecture ou écriture mémoire, aucun accès aux dossiers des jeux, aucune injection.
- Multimédia : API Windows publique (SMTC + volume système).
- Discord : IPC local officiel (Rich Presence).
- Stats : APIs publiques par pseudo (fortnite-api.com, HenrikDev pour Valorant, apexlegendsapi.com) + profils tracker publics (R6, Rocket League).
- Jeu en cours : détecté par le seul **nom** du processus (comme le Gestionnaire des tâches). REZ n'ouvre jamais le processus du jeu.
- Pochettes : celles fournies par le lecteur via SMTC, ou en HD via iTunes Search (API publique).
- Mode Focus : coupe les notifications Windows de Discord (même réglage que Paramètres > Notifications), puis les restaure.
- Overlay = simple fenêtre "toujours au-dessus". Les jeux en **plein écran exclusif** passent par-dessus → mettre le jeu en **plein écran fenêtré / sans bordure**.

## Structure
```
REZ/
├─ package.json            scripts npm (dev, build, icon, setup-github, release)
├─ version.json            lu par l'auto-update (GitHub raw)
├─ scripts/
│  ├─ make-icon.ps1        génère le logo 1024px
│  ├─ setup-github.ps1     crée le dépôt GitHub + configure l'URL d'update
│  └─ release.ps1          bump → build → SHA-256 → release GitHub → version.json
├─ src/                    frontend (HTML/CSS/JS vanilla)
│  ├─ index.html  style.css  app.js  logo.png
└─ src-tauri/
   ├─ Cargo.toml  build.rs  tauri.conf.json
   ├─ capabilities/default.json
   ├─ icons/               icon.ico + PNG (générés)
   └─ src/
      ├─ main.rs           fenêtre, tray, raccourci, commandes
      ├─ settings.rs       paramètres JSON (%APPDATA%\com.rez.island)
      ├─ secure.rs         chiffrement DPAPI des clés API
      ├─ media.rs          lecture/pause/suivant + volume
      ├─ discord.rs        Rich Presence IPC
      ├─ net.rs            météo, stats, chat IA (Claude)
      └─ updater.rs        auto-update 1 clic
```

## Prérequis (une fois)
```bash
winget install Rustlang.Rustup
```
```bash
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```
```bash
winget install OpenJS.NodeJS.LTS
```
```bash
winget install GitHub.cli
```
```bash
gh auth login
```
WebView2 est déjà inclus dans Windows 11.

## Dev / Build
```bash
npm install
```
```bash
npm run dev
```
```bash
npm run build
```
Sorties :
- `src-tauri/target/release/rez.exe` → exe portable (celui que l'auto-update remplace)
- `src-tauri/target/release/bundle/nsis/REZ_x.y.z_x64-setup.exe` → installeur (installation par utilisateur, sans admin)

Logo régénéré : `npm run icon`

## GitHub (une fois)
```bash
npm run setup-github -- -User TON_PSEUDO_GITHUB
```
→ écrit l'URL `https://raw.githubusercontent.com/TON_PSEUDO/REZ/main/version.json` dans `updater.rs`, crée le dépôt **public** et pousse le code.
Ensuite, recompile (`npm run build`) pour que l'exe connaisse l'URL. La première release le fait automatiquement.

## Publier une mise à jour (1 commande)
```bash
npm run release -- -Version 1.0.1 -Notes "Nouveautés"
```
Ce script : met à jour la version (tauri.conf.json, Cargo.toml, package.json), compile, calcule le SHA-256, crée la release GitHub `v1.0.1` (`REZ.exe` + installeur), puis pousse `version.json`.
Au démarrage, chaque REZ installé lit `version.json`. Si une version plus récente existe, une carte « Installer » apparaît : REZ télécharge le nouvel exe, vérifie le SHA-256 (sinon il annule), remplace l'exe et redémarre.

## Utilisation
| Action | Comment |
|---|---|
| Île | Pilule collée en haut au centre (heure, musique en cours / météo). Clic → l'île s'ouvre |
| Ouvrir / réduire | `Ctrl+B` (modifiable), clic sur la pilule ou l'icône tray, `Échap` pour réduire |
| Masquer / afficher l'île | `Ctrl+N` (modifiable, Retour arrière pour le désactiver). REZ reste dans le tray |
| Déplacer l'île | Paramètres → « Déverrouiller l'emplacement », puis glisser la pilule ou l'en-tête (multi-écrans). 🔒 pour la reverrouiller |
| Opacité | Paramètres → curseur « Opacité du verre » |
| Lecteur | Pochette HD + icône de l'app, ±10 s, piste précédente / suivante, barre de progression cliquable |
| Accueil | Onglets Aperçu (widgets Focus, Système, Discord, Jeu, raccourcis) / Météo (heure par heure, 5 jours) / Outils |
| Mode Focus | Accueil → Outils : minuteur. Les notifications Discord sont masquées jusqu'à la fin |
| Discord | Connexion auto quand Discord est lancé : photo de profil, présence auto selon le jeu, présence perso |
| Trackers | Stats → choisir le jeu → lier son ID. Valorant : rang + dernière partie avec alliés et adversaires. Apex : statut en direct |
| Taille | Paramètres → Compact / Large (transition animée) |
| Pilule permanente | Paramètres → « Pilule toujours visible » (désactivé = REZ disparaît complètement une fois réduit) |
| Quitter | clic droit tray → Quitter, ou Paramètres → Quitter |
| Épingler un dossier | glisser-déposer sur REZ, ou formulaire Accès rapide |
| Chat IA | clé API Anthropic dans Paramètres (modèle par défaut `claude-opus-5-5`) |
| Stats Fortnite | clé gratuite sur fortnite-api.com → Paramètres |
| Stats Valorant | clé gratuite HenrikDev (api.henrikdev.xyz, via leur Discord) → Paramètres |
| Stats Apex | clé gratuite sur apexlegendsapi.com → Paramètres |
| Discord | créer une app sur discord.com/developers → Client ID dans Paramètres |
| Démarrage auto | Paramètres → Lancer avec Windows (démarre masqué dans le tray) |

Les clés API sont chiffrées avec DPAPI (liées à ta session Windows). Elles ne sont jamais renvoyées à l'interface.
