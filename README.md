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
- Système : CPU / RAM / GPU / VRAM / températures via les compteurs de performance Windows, DXGI et D3DKMT (comme le Gestionnaire des tâches). **Aucun pilote noyau** (les pilotes de capteurs type WinRing0 sont bloqués par Vanguard & co).
- Vocal Discord : RPC local officiel, autorisé par toi dans Discord (scopes `rpc`, `rpc.voice.read`, `rpc.voice.write`).
- Mélangeur audio : sessions audio Windows (comme le mélangeur de volume). Aucun handle sur les jeux.
- Téléchargements : manifestes Steam `steamapps/appmanifest_*.acf` (jamais `steamapps/common`), fichiers partiels du dossier Téléchargements, API Web locale de qBittorrent, débit réseau.
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
      ├─ audio.rs          mélangeur audio par application
      ├─ hw.rs             GPU, VRAM, températures, débit réseau
      ├─ downloads.rs      suivi des téléchargements
      ├─ voice.rs          thread Discord unique : présence + vocal, réessais silencieux
      ├─ discord.rs        connexion IPC Discord non bloquante
      ├─ net.rs            météo, stats, chat IA (Claude, ChatGPT, Gemini)
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
| Position | Paramètres → Haut / Gauche / Droite. Les panneaux s'ouvrent toujours vers l'intérieur de l'écran |
| Déplacer l'île | Paramètres → « Déverrouiller l'emplacement », glisser la pilule ou l'en-tête (multi-écrans), puis 🔒 : l'île reste exactement là |
| Opacité | Paramètres → curseur « Opacité du verre » |
| Lecteur | Pochette HD + icône de l'app, ±10 s, piste précédente / suivante, barre de progression cliquable |
| Mélangeur audio | Multimédia → Mélangeur audio : un volume (et un muet) par application qui joue du son |
| Système | Accueil → jauges CPU, GPU, RAM, VRAM + températures CPU / GPU (si Windows les fournit) |
| Téléchargements | Accueil → Téléch. : Steam (% exact + temps restant), qBittorrent, navigateurs. Progression aussi dans la pilule |
| Accueil | Onglets Aperçu (widgets Focus, Système, Discord, Jeu, raccourcis) / Météo (heure par heure, 5 jours) / Outils |
| Mode Focus | Accueil → Outils : minuteur. Les notifications Discord sont masquées jusqu'à la fin |
| Discord | Assistant intégré (vue Discord) : colle l'Application ID, REZ le vérifie auprès de Discord et l'enregistre. Connexion auto, réessais discrets si Discord n'est pas prêt. Photo de profil, présence auto selon le jeu, présence perso |
| Vocal Discord | Client ID + Client Secret de ton app (redirection `http://localhost`), puis « Activer le vocal » : membres du salon, qui parle (cercle vert), micro / casque / quitter |
| Trackers | Stats → choisir le jeu → lier son ID. Valorant : rang + dernière partie avec alliés et adversaires. Apex : statut en direct |
| Taille | Paramètres → Compact / Large (transition animée) |
| Pilule permanente | Paramètres → « Pilule toujours visible » (désactivé = REZ disparaît complètement une fois réduit) |
| Quitter | clic droit tray → Quitter, ou Paramètres → Quitter |
| Épingler un dossier | glisser-déposer sur REZ, ou formulaire Accès rapide |
| Chat IA | Optionnel. Vue IA → Claude / ChatGPT / Gemini → « Obtenir une clé » ouvre la page officielle. Modèles par défaut : `claude-opus-5-5`, `gpt-5-mini`, `gemini-2.5-flash` |
| Stats Fortnite | clé gratuite sur fortnite-api.com → Paramètres |
| Stats Valorant | clé gratuite HenrikDev (api.henrikdev.xyz, via leur Discord) → Paramètres |
| Stats Apex | clé gratuite sur apexlegendsapi.com → Paramètres |
| Discord | assistant dans la vue Discord : créer l'app REZ sur discord.com/developers, coller l'Application ID |
| Démarrage auto | Paramètres → Lancer avec Windows (démarre masqué dans le tray) |

Les clés API sont chiffrées avec DPAPI (liées à ta session Windows). Elles ne sont jamais renvoyées à l'interface.
