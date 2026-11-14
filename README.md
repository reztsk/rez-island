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
- Performance PC : compteurs PDH + liste des processus tenue par Windows (temps CPU, mémoire privée). **Aucun handle ouvert sur un processus**, donc jamais sur un jeu.
- Batteries : propriété Bluetooth publiée par Windows, Windows.Gaming.Input (manettes Xbox), HID++ Logitech en **lecture seule** (comme G HUB / Solaar), batterie du PC. REZ n'envoie jamais de commande inconnue à un appareil.
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
      ├─ clip.rs           historique du presse-papiers (mémoire seule)
      ├─ settings.rs       paramètres JSON (%APPDATA%\com.rez.island)
      ├─ secure.rs         chiffrement DPAPI des clés API
      ├─ media.rs          lecture/pause/suivant + volume
      ├─ audio.rs          mélangeur audio par application
      ├─ hw.rs             GPU, VRAM, températures, débit réseau
      ├─ perf.rs           menu Performance : cœurs, fréquence, disque, réseau, applis gourmandes
      ├─ devices.rs        périphériques sans fil + batterie / charge
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
| Position | Réglages (⚙ en haut) → Haut / Gauche / Droite. Les panneaux s'ouvrent toujours vers l'intérieur de l'écran |
| Déplacer l'île | Cadenas en haut de l'île (ou Réglages → « Déverrouiller l'emplacement »), glisser la pilule ou l'en-tête n'importe où : haut, bas, côtés, coins, multi-écrans. Re-clic sur le cadenas : l'île reste exactement là, même après redémarrage |
| Opacité | Réglages → curseur « Opacité du verre » |
| Thèmes | Réglages → Apparence : Violet, Cyan, Rose, Vert, Orange, Graphite |
| Réglages | Roue crantée ⚙ (en-tête, pied de l'accueil ou tuile) : page dédiée Apparence, Accueil & widgets, Modules du dock, Position, Raccourcis, Services, Mises à jour, Système |
| Lecteur | Pochette HD + icône de l'app, ±10 s, piste précédente / suivante, barre de progression cliquable |
| Mélangeur audio | Multimédia → Mélangeur audio : un volume (et un muet) par application qui joue du son |
| Système | Accueil → jauges CPU, GPU, RAM, VRAM + températures CPU / GPU (si Windows les fournit). Clic → Performance PC |
| Performance PC | Dock → courbes en direct (CPU, GPU, RAM, VRAM, disque, réseau), charge par cœur, fréquence, processus / threads / handles, applications les plus gourmandes (CPU ou mémoire) |
| Périphériques | Dock → casque, manette, souris, clavier sans fil : % de batterie, icône par type, vert si bon, orange en charge ou bas, rouge si faible (alerte). Résumé sur l'accueil |
| Calculatrice | Tuile Calcul : calculatrice intégrée (clavier physique, historique, √, xʸ, π, %) + conversion d'unités (longueur, masse, température, volume, vitesse, surface, données, temps, énergie) |
| Téléchargements | Widget Téléchargements → page dédiée : Steam (% exact + temps restant), qBittorrent, navigateurs. Progression aussi dans la pilule |
| Accueil | Grille de widgets en verre fumé : Minuteur, Lecteur, Presse-papiers, Notes, Anti-veille, Météo, Horloge, Focus, Système, Batteries, Téléchargements, Discord, Jeu, Dossiers + tuiles d'accès rapide |
| Personnaliser | ✎ en bas de l'accueil : glisser les tuiles pour les déplacer, × pour retirer, « Ajouter un widget ». Ou Réglages → Accueil & widgets (activer / désactiver, monter / descendre) |
| Minuteur | Widget : − / ▶ / +, anneau de progression, compte à rebours dans la pilule, son à la fin |
| Presse-papiers | Widget : les 30 derniers textes copiés (en mémoire uniquement, jamais sur disque), clic pour recopier. Les copies des gestionnaires de mots de passe sont ignorées |
| Anti-veille | Widget : garde l'écran et le PC allumés (interrupteur) |
| Mises à jour | Réglages → Mises à jour : « Vérifier les mises à jour » + installation automatique (île réduite, hors jeu, fichier vérifié SHA-256) |
| Modules | Réglages → Modules du dock : masquer Multimédia, Discord, Stats, Performance, Batteries, IA ou Fichiers |
| Mode Focus | Widget Focus : − / + pour la durée, ▶ pour lancer. Les notifications Discord sont masquées jusqu'à la fin |
| Discord | Assistant intégré (vue Discord) : colle l'Application ID, REZ le vérifie auprès de Discord et l'enregistre. Connexion auto, réessais discrets si Discord n'est pas prêt. Photo de profil, présence auto selon le jeu, présence perso |
| Vocal Discord | Client ID + Client Secret de ton app (redirection `http://localhost`), puis « Activer le vocal » : membres du salon, qui parle (cercle vert), micro / casque / quitter |
| Trackers | Stats → choisir le jeu → lier son ID. Valorant : rang + dernière partie avec alliés et adversaires. Apex : statut en direct |
| Taille | Réglages → Compact / Large (transition animée) |
| Pilule permanente | Réglages → « Pilule toujours visible » (désactivé = REZ disparaît complètement une fois réduit) |
| Quitter | clic droit tray → Quitter, ou Réglages → Quitter |
| Épingler un dossier | glisser-déposer sur REZ, ou formulaire Accès rapide |
| Chat IA | Optionnel. Vue IA → Claude / ChatGPT / Gemini → « Obtenir une clé » ouvre la page officielle. Modèles par défaut : `claude-opus-5-5`, `gpt-5-mini`, `gemini-2.5-flash` |
| Stats Fortnite | clé gratuite sur fortnite-api.com → Réglages |
| Stats Valorant | clé gratuite HenrikDev (api.henrikdev.xyz, via leur Discord) → Réglages |
| Stats Apex | clé gratuite sur apexlegendsapi.com → Réglages |
| Discord | assistant dans la vue Discord : créer l'app REZ sur discord.com/developers, coller l'Application ID |
| Démarrage auto | Réglages → Lancer avec Windows (démarre masqué dans le tray) |

Les clés API sont chiffrées avec DPAPI (liées à ta session Windows). Elles ne sont jamais renvoyées à l'interface.
