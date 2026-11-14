// REZ - frontend (vanilla, zéro dépendance)
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (s) => document.querySelector(s);
const $$ = (s) => document.querySelectorAll(s);
const store = {
  get: (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } },
  set: (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} },
};

// Tailles de l'île (px logiques) : pilule / ouverte
const SIZES = {
  compact: { pill: [210, 34], open: [360, 392] },
  large:   { pill: [244, 40], open: [420, 456] },
};
const ANIM = 440;

let S = {};                 // paramètres (clés masquées)
let live = { game: null, discord: {}, focus_until: 0 };
let mode = 'pill';          // 'pill' | 'open'
let winVisible = false;
let winW = 210, winH = 34;
let animId = 0, lastCollapse = 0;
let view = 'home', sub = 'ov';
let clockTimer = null, mediaTimer = null, sysTimer = null;
let lastWeather = 0, weatherTxt = '';
let media = null, mediaAt = 0;
let chat = [];
const isl = $('#island');

// ---------- utilitaires ----------
let toastT;
function toast(msg, bad = false) {
  const t = $('#toast');
  t.textContent = msg;
  t.className = 'on' + (bad ? ' bad' : '');
  clearTimeout(toastT);
  toastT = setTimeout(() => (t.className = ''), 2600);
}
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
async function call(cmd, args) {
  try { return await invoke(cmd, args); }
  catch (e) { toast(String(e), true); throw e; }
}
const sizeKey = () => (S.size === 'large' ? 'large' : 'compact');
const nowS = () => Date.now() / 1000;
const mmss = (s) => { s = Math.max(0, Math.round(s)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
const num = (v, d = 0) => (v == null || v === '' ? '—' : Number(v).toLocaleString('fr-FR', { maximumFractionDigits: d }));
const safeUrl = (u) => (typeof u === 'string' && /^https:\/\//.test(u) ? u : '');

// ---------- île : pilule <-> ouverte ----------
async function fit(w, h, focus = false) {
  winW = w; winH = h;
  try { await invoke('island_fit', { width: w, height: h, focus }); } catch {}
}
function setVars(m) {
  const [w, h] = SIZES[sizeKey()][m];
  isl.style.setProperty('--w', w + 'px');
  isl.style.setProperty('--h', h + 'px');
}
async function render(m, focus = false) {
  const id = ++animId;
  mode = m;
  const [w, h] = SIZES[sizeKey()][m];
  await fit(Math.max(w, winW), Math.max(h, winH), focus);
  isl.classList.toggle('open', m === 'open');
  setVars(m);
  setTimeout(() => { if (id === animId && (winW !== w || winH !== h)) fit(w, h); }, ANIM + 20);
  if (m === 'open') onView();
  schedule();
  return id;
}
function expand() { render('open', true); }
function collapse() {
  if (mode !== 'open') return;
  lastCollapse = Date.now();
  render('pill').then((id) => {
    if (!S.pill_mode) setTimeout(() => { if (id === animId && mode === 'pill') invoke('hide_window'); }, ANIM + 40);
  });
}

listen('rez://open', () => expand());
listen('rez://toggle', () => {
  if (mode === 'open') collapse();
  else if (Date.now() - lastCollapse > 300) expand();
});
// Ctrl+N : l'île disparaît -> revient en pilule
listen('rez://collapse-now', () => {
  if (mode === 'open') { animId++; mode = 'pill'; isl.classList.remove('open'); setVars('pill'); fit(...SIZES[sizeKey()].pill); }
});
listen('rez://blur', () => { if (S.hide_on_blur && mode === 'open') collapse(); });
listen('rez://visible', (e) => { winVisible = !!e.payload; schedule(); });
listen('rez://live', (e) => onLive(e.payload));

// ---------- glisser-déposer (île déverrouillée) ----------
let drag = null, dragged = false;
function dragStart(e) {
  dragged = false;
  if (S.locked !== false || e.button !== 0) return;
  if (e.target.closest('input, textarea, select, .top-btns')) return;
  drag = { x: e.screenX, y: e.screenY };
}
document.addEventListener('pointermove', (e) => {
  if (!drag) return;
  if (!(e.buttons & 1)) { drag = null; return; }
  if (Math.hypot(e.screenX - drag.x, e.screenY - drag.y) > 4) {
    drag = null; dragged = true;
    invoke('start_drag').catch(() => {});
  }
});
document.addEventListener('pointerup', () => { drag = null; });
$('#pillFace').addEventListener('pointerdown', dragStart);
$('#topBar').addEventListener('pointerdown', dragStart);
$('#pillFace').onclick = () => { if (dragged) { dragged = false; return; } expand(); };
$('#btnCollapse').onclick = collapse;
$('#btnLock').onclick = () => setLocked(true);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !document.activeElement?.classList.contains('sc')) collapse();
});

// ---------- navigation ----------
function go(v) {
  if (v !== view) $('#views').scrollTop = 0;
  view = v;
  $$('.view').forEach((el) => el.classList.toggle('active', el.dataset.view === v));
  $$('.dock button').forEach((b) => b.classList.toggle('active', b.dataset.go === v));
  onView();
  schedule();
}
function goSub(s) {
  if (s !== sub) $('#views').scrollTop = 0;
  sub = s;
  $$('#homeTabs button').forEach((b) => b.classList.toggle('on', b.dataset.sub === s));
  $$('.sub').forEach((el) => el.classList.toggle('on', el.dataset.sub === s));
  onView();
  schedule();
}
document.addEventListener('click', (e) => {
  const b = e.target.closest('[data-go]');
  if (b) {
    if (b.dataset.go === 'stats' && live.game && GAMES[live.game.id]) selectGame(live.game.id);
    go(b.dataset.go);
  }
  const sb = e.target.closest('[data-sub-go]');
  if (sb) goSub(sb.dataset.subGo);
});
$('#homeTabs').addEventListener('click', (e) => { const b = e.target.closest('[data-sub]'); if (b) goSub(b.dataset.sub); });
function onView() {
  if (mode !== 'open') return;
  if (view === 'home' && Date.now() - lastWeather > 10 * 60e3) loadWeather();
  if (view === 'stats') loadStats(false);
  if (view === 'ai') setTimeout(() => $('#aIn').focus(), 60);
}

// ---------- timers (tout est en pause quand REZ est masqué) ----------
function focusLeft() { return live.focus_until ? live.focus_until - nowS() : 0; }
function schedule() {
  clearInterval(clockTimer); clearInterval(mediaTimer); clearInterval(sysTimer);
  clockTimer = mediaTimer = sysTimer = null;
  if (!winVisible) return;
  tick();
  clockTimer = setInterval(tick, mode === 'open' || focusLeft() > 0 ? 1000 : 5000);
  const fast = mode === 'open' && ((view === 'home' && sub === 'ov') || view === 'media');
  refreshMedia();
  mediaTimer = setInterval(refreshMedia, fast ? 2000 : 5000);
  if (mode === 'open' && view === 'home' && sub === 'ov') { refreshSys(); sysTimer = setInterval(refreshSys, 2500); }
}
setInterval(() => { if (winVisible && Date.now() - lastWeather > 15 * 60e3) loadWeather(); }, 60e3);

// ---------- horloge ----------
const fmtDate = new Intl.DateTimeFormat('fr-FR', { weekday: 'short', day: 'numeric', month: 'short' });
function tick() {
  const d = new Date();
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  $('#pillTime').textContent = hm;
  $('#time').textContent = hm;
  $('#bigTime').textContent = hm;
  $('#sec').textContent = String(d.getSeconds()).padStart(2, '0');
  $('#date').textContent = fmtDate.format(d).replace(/\./g, '');
  renderFocus();
  renderProgress();
  if (focusLeft() > 0 || mode === 'pill') renderPill();
}

// ---------- météo ----------
const WMO = [
  [[0], '☀️', 'Dégagé'], [[1], '🌤️', 'Peu nuageux'], [[2], '⛅', 'Nuageux'], [[3], '☁️', 'Couvert'],
  [[45, 48], '🌫️', 'Brouillard'], [[51, 53, 55, 56, 57], '🌦️', 'Bruine'], [[61, 63, 65, 66, 67], '🌧️', 'Pluie'],
  [[71, 73, 75, 77], '❄️', 'Neige'], [[80, 81, 82], '🌧️', 'Averses'], [[85, 86], '🌨️', 'Neige'], [[95, 96, 99], '⛈️', 'Orage'],
];
function wmo(code, day = 1) {
  const m = WMO.find(([c]) => c.includes(code)) || [[], '🌡️', '—'];
  return [(code <= 1 && !day) ? '🌙' : m[1], m[2]];
}
const fmtDay = new Intl.DateTimeFormat('fr-FR', { weekday: 'short' });
async function loadWeather(city) {
  let w;
  try { w = await invoke('weather', city ? { city } : {}); }
  catch (e) {
    if (city) toast(String(e), true);
    else $('#wDesc').textContent = 'Météo indisponible';
    return;
  }
  lastWeather = Date.now();
  if (city) {
    S.city = w.city;                // la ville saisie devient la ville par défaut (sauvée côté backend)
    $('#pCity').value = w.city;
    toast(`Météo : ${w.city}`);
  }
  const [ico, desc] = wmo(w.code, w.is_day);
  const place = [w.city, w.country].filter(Boolean).join(', ');
  $('#wIco').textContent = $('#wxIco').textContent = ico;
  $('#wTemp').textContent = `${Math.round(w.temp)}°`;
  $('#wDesc').textContent = `${desc} · ${w.city}`;
  $('#wxTemp').textContent = `${Math.round(w.temp)}°`;
  $('#wxDesc').textContent = `${desc} · ${place}`;
  $('#wxMax').textContent = `↑ ${Math.round(w.max)}°`;
  $('#wxMin').textContent = `↓ ${Math.round(w.min)}°`;
  if (document.activeElement !== $('#wxCity')) $('#wxCity').value = w.city;
  const box = (l, v) => `<div><small>${l}</small><b>${v}</b></div>`;
  $('#wxGrid').innerHTML = box('Ressenti', `${Math.round(w.feels)}°`) + box('Vent', `${Math.round(w.wind)} km/h`) + box('Humidité', `${num(w.humidity)} %`)
    + box('UV max', num(w.uv, 1)) + box('Lever', esc(w.sunrise) || '—') + box('Coucher', esc(w.sunset) || '—');
  $('#wxHours').innerHTML = (w.hourly || []).map((h, i) => `<div><span>${i ? esc(h.time) + 'h' : 'Now'}</span><span class="i">${wmo(h.code, h.is_day)[0]}</span><b>${Math.round(h.temp)}°</b>${h.rain ? `<small>${h.rain}%</small>` : ''}</div>`).join('');
  $('#wxDays').innerHTML = (w.days || []).map((d, i) => `<div><span>${i ? fmtDay.format(new Date(d.date + 'T12:00')).replace('.', '') : "Auj."}</span><span>${wmo(d.code)[0]}</span><small>${d.rain ? d.rain + ' %' : ''}</small><span><em>${Math.round(d.min)}°</em> ${Math.round(d.max)}°</span></div>`).join('');
  weatherTxt = `${ico} ${Math.round(w.temp)}°`;
  renderPill();
}
$('#wxForm').onsubmit = (e) => { e.preventDefault(); const c = $('#wxCity').value.trim(); if (c) loadWeather(c); };
$('#pCity').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); const c = $('#pCity').value.trim(); if (c) loadWeather(c); } });

// ---------- multimédia ----------
const ICON_PLAY = '<path d="M7 4l13 8-13 8z"/>';
const ICON_PAUSE = '<path d="M7 4h3v16H7zM14 4h3v16h-3z"/>';
const ICON_VOL = '<path d="M11 5L6 9H2v6h4l5 4V5zM15.5 8.5a5 5 0 010 7M19 5a10 10 0 010 14"/>';
const ICON_MUTE = '<path d="M11 5L6 9H2v6h4l5 4V5zM22 9l-6 6M16 9l6 6"/>';
const G_SPOT = '<svg viewBox="0 0 24 24"><path d="M6 9.5c4-1.3 8.5-1 12 1M7 13c3.3-1 6.7-.7 9.5.8M8 16.3c2.6-.7 5-.5 7 .6"/></svg>';
const G_PLAY = '<svg viewBox="0 0 24 24" style="fill:#fff;stroke:none"><path d="M9 7l8 5-8 5z"/></svg>';
const G_NOTE = '<svg viewBox="0 0 24 24"><path d="M9 18V6l10-2v12"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="16" r="2"/></svg>';
const APPS = [
  ['spotify', 'Spotify', '#1DB954', G_SPOT], ['youtube', 'YouTube', '#FF0033', G_PLAY], ['deezer', 'Deezer', '#A238FF', G_NOTE],
  ['applemusic', 'Apple Music', '#FA2D48', G_NOTE], ['itunes', 'iTunes', '#FA2D48', G_NOTE], ['zune', 'Lecteur Windows', '#0078D4', G_PLAY],
  ['tidal', 'TIDAL', '#1f1f1f', G_NOTE], ['amazon', 'Amazon Music', '#25D1DA', G_NOTE], ['soundcloud', 'SoundCloud', '#FF5500', G_NOTE],
  ['chrome', 'Chrome', '#4285F4', 'C'], ['msedge', 'Edge', '#0C88D7', 'E'], ['firefox', 'Firefox', '#FF7139', 'F'],
  ['opera', 'Opera', '#FF1B2D', 'O'], ['brave', 'Brave', '#FB542B', 'B'], ['vivaldi', 'Vivaldi', '#EF3939', 'V'],
  ['vlc', 'VLC', '#FF8800', G_PLAY], ['discord', 'Discord', '#5865F2', 'D'], ['twitch', 'Twitch', '#9146FF', G_PLAY], ['musicbee', 'MusicBee', '#E8A400', G_NOTE],
];
const BROWSERS = ['chrome', 'msedge', 'firefox', 'opera', 'brave', 'vivaldi'];
function appInfo(id) {
  if (!id) return null;
  const s = id.toLowerCase();
  const a = APPS.find(([k]) => s.includes(k));
  if (a) return { key: a[0], name: a[1], color: a[2], glyph: a[3] };
  const name = id.split(/[!\\.]/)[0].replace(/_.*$/, '');
  return { key: 'x', name, color: '#6d28d9', glyph: esc(name.charAt(0).toUpperCase() || '♪') };
}

let thumbKey = '', thumbData = '', thumbVideo = false, coverKey = '', hdUrl = '';
const hdCache = new Map();
function setCovers() {
  const src = hdUrl || thumbData;
  const a = appInfo(media?.has_session ? media.app : '');
  for (const c of $$('.cover')) {
    const img = c.querySelector('img');
    if (src && img.getAttribute('src') !== src) img.src = src;
    c.classList.toggle('has', !!src);
    c.querySelector('.ph').textContent = (media?.title || '♪').trim().charAt(0).toUpperCase() || '♪';
    const b = c.querySelector('.app-badge');
    b.classList.toggle('on', !!a);
    if (a) { b.style.background = a.color; b.innerHTML = a.glyph; b.title = a.name; }
  }
  const pa = $('#pillArt');
  if (src && pa.getAttribute('src') !== src) pa.src = src;
  pa.dataset.ok = src ? '1' : '';
}
async function updateCover(m) {
  if (!m.has_session) { thumbKey = coverKey = thumbData = hdUrl = ''; setCovers(); return; }
  if (m.thumb !== thumbKey) {
    thumbKey = m.thumb;
    thumbData = m.thumb ? await invoke('media_thumb').catch(() => '') : '';
    thumbVideo = false;
    if (thumbData) { const im = new Image(); im.onload = () => { thumbVideo = im.naturalWidth / im.naturalHeight > 1.4; renderMediaMeta(); }; im.src = thumbData; }
  }
  const key = `${m.app}|${m.title}|${m.artist}`;
  if (key !== coverKey) {
    coverKey = key;
    hdUrl = hdCache.get(key) || '';
    if (!hdCache.has(key) && m.title && m.artist) {
      hdCache.set(key, '');
      invoke('media_cover', { title: m.title, artist: m.artist }).then((u) => {
        u = safeUrl(u);
        hdCache.set(key, u);
        if (coverKey === key && u) { hdUrl = u; setCovers(); }
      }).catch(() => {});
    }
  }
  setCovers();
}
function renderMediaMeta() {
  const m = media || {};
  const a = appInfo(m.app);
  const browser = a && BROWSERS.includes(a.key);
  $('#mApp').textContent = a ? (browser && thumbVideo ? `${a.name} · vidéo` : a.name) : '';
}
function renderPill() {
  const left = focusLeft();
  const playing = media?.playing && media?.has_session;
  const info = $('#pillInfo');
  const dot = $('#pillDot');
  dot.classList.toggle('focus', left > 0);
  dot.classList.toggle('game', left <= 0 && !!live.game);
  info.classList.toggle('focus', left > 0);
  const showArt = left <= 0 && playing && $('#pillArt').dataset.ok === '1';
  $('#pillArt').classList.toggle('hidden', !showArt);
  $('#pillBars').classList.toggle('hidden', !(left <= 0 && playing));
  if (left > 0) info.textContent = `Focus ${mmss(left)}`;
  else if (playing) info.textContent = media.title || appInfo(media.app)?.name || '';
  else if (live.game) info.textContent = `🎮 ${live.game.name}`;
  else info.textContent = weatherTxt;
}
function curPos() {
  if (!media?.duration) return 0;
  const p = media.position + (media.playing ? (Date.now() - mediaAt) / 1000 : 0);
  return Math.min(p, media.duration);
}
let seekDrag = false;
function renderProgress() {
  const m = media;
  const has = m?.has_session && m.duration > 0;
  const pos = has ? curPos() : 0;
  const pct = has ? (pos / m.duration) * 100 : 0;
  $('#npProg').style.width = pct + '%';
  if (!seekDrag) {
    setRange($('#mSeek'), has ? Math.round(pct * 10) / 10 : 0, 10);
    $('#mPos').textContent = mmss(pos);
  }
  $('#mDur').textContent = has ? mmss(m.duration) : '0:00';
  $('#mSeek').disabled = !(has && m.can_seek);
}
function setRange(el, v, k = 1) { el.value = v * k; el.style.setProperty('--p', v + '%'); }
async function refreshMedia() {
  let m;
  try { m = await invoke('media_info'); } catch { return; }
  media = m; mediaAt = Date.now();
  await updateCover(m);
  renderPill();
  if (mode !== 'open') return;
  const title = m.has_session ? (m.title || 'Titre inconnu') : 'Aucune lecture';
  const artist = m.has_session ? ([m.artist, m.album].filter(Boolean).join(' · ') || appInfo(m.app)?.name || '—') : 'Lance Spotify, YouTube…';
  $('#mTitle').textContent = $('#npTitle').textContent = title;
  $('#mArtist').textContent = $('#npArtist').textContent = artist;
  $('#mCover').classList.toggle('playing', m.playing);
  for (const i of $$('.play-ico')) i.innerHTML = m.playing ? ICON_PAUSE : ICON_PLAY;
  $('#mMuteIco').innerHTML = m.muted ? ICON_MUTE : ICON_VOL;
  renderMediaMeta();
  renderProgress();
  if (!volDrag) {
    const v = Math.round(m.volume * 100);
    setRange($('#mVol'), v);
    $('#mVolVal').textContent = v;
  }
}
async function mediaAct(a) { try { await call('media_action', { action: a }); setTimeout(refreshMedia, 300); } catch {} }
async function mediaSeek(delta, to = null) {
  try { await call('media_seek', { delta, to }); setTimeout(refreshMedia, 250); } catch {}
}
document.addEventListener('click', (e) => {
  const m = e.target.closest('[data-media]');
  if (m) { e.stopPropagation(); mediaAct(m.dataset.media); }
  const s = e.target.closest('[data-seek]');
  if (s) mediaSeek(+s.dataset.seek);
});
const seek = $('#mSeek');
seek.addEventListener('input', () => {
  seekDrag = true;
  const p = seek.value / 10;
  seek.style.setProperty('--p', p + '%');
  if (media?.duration) $('#mPos').textContent = mmss((p / 100) * media.duration);
});
seek.addEventListener('change', () => {
  seekDrag = false;
  if (media?.duration) mediaSeek(0, (seek.value / 1000) * media.duration);
});
let volDrag = false, volT;
$('#mMute').onclick = async () => { try { await call('media_mute', { muted: !media?.muted }); refreshMedia(); } catch {} };
const vol = $('#mVol');
vol.addEventListener('input', () => {
  volDrag = true;
  setRange(vol, vol.value);
  $('#mVolVal').textContent = vol.value;
  clearTimeout(volT);
  volT = setTimeout(() => invoke('media_volume', { value: vol.value / 100 }).catch(() => {}), 40);
});
vol.addEventListener('change', () => { volDrag = false; });

// ---------- système ----------
async function refreshSys() {
  try {
    const s = await invoke('sys_info');
    $('#cpuBar').style.width = s.cpu.toFixed(0) + '%';
    $('#cpuVal').textContent = s.cpu.toFixed(0) + '%';
    $('#ramBar').style.width = s.ram.toFixed(0) + '%';
    $('#ramVal').textContent = s.ram.toFixed(0) + '%';
    $('#ramVal').title = `${s.used.toFixed(1)} / ${s.total.toFixed(1)} Go`;
  } catch {}
}

// ---------- état live : Discord, jeu, focus ----------
let prevGame = null;
function onLive(l) {
  if (!l) return;
  const before = live.focus_until;
  live = l;
  const gid = l.game?.id || null;
  // fin de partie -> stats actualisées (le temps que l'API publique enregistre la game)
  if (prevGame && prevGame !== gid && GAMES[prevGame]?.load) {
    const g = prevGame;
    setTimeout(() => { delete statCache[g]; if (curGame === g && view === 'stats') loadStats(true); }, 90e3);
  }
  prevGame = gid;
  renderDiscord();
  renderGame();
  renderFocus();
  renderPill();
  if (!!before !== !!l.focus_until) schedule();
}
function setPP(img, ph, user) {
  const url = safeUrl(user?.avatar);
  img.classList.toggle('hidden', !url);
  ph.classList.toggle('hidden', !!url);
  if (url && img.getAttribute('src') !== url) img.src = url;
  ph.textContent = (user?.name || '?').charAt(0).toUpperCase();
}
function renderDiscord() {
  const d = live.discord || {};
  const u = d.connected ? d.user : null;
  setPP($('#dPP'), $('#dPPph'), u);
  setPP($('#wgPP'), $('#wgPPph'), u);
  const st = $('#dStatus');
  let txt, cls = '';
  if (d.connected) { txt = 'Connecté · Rich Presence prête'; cls = 'on'; }
  else if (!d.running) txt = 'Discord fermé';
  else { txt = d.error || 'Connexion…'; cls = 'wait'; }
  st.textContent = txt;
  st.className = 'status ' + cls;
  $('#dName').textContent = u?.name || 'Discord';
  $('#dUser').textContent = u ? '@' + (u.username || '') : (d.running ? 'Discord lancé' : 'Non connecté');
  $('#dBadge').classList.toggle('hidden', !d.connected);
  const a = d.activity;
  $('#dAct').textContent = a ? a.details : 'Aucune';
  $('#dActSub').textContent = a ? a.state : (S.discord_auto ? 'Lance un jeu : présence automatique' : 'Présence auto désactivée');
  $('#wgDName').textContent = u?.name || (d.running ? 'Discord' : 'Hors ligne');
  $('#wgDSub').textContent = a ? a.details : (u ? '@' + u.username : (d.running ? (d.error || 'Connexion…') : 'Discord fermé'));
}
function renderGame() {
  const g = live.game;
  $('#wgGame').textContent = g ? g.name : 'Aucun jeu';
  $('#wgGameSub').textContent = g ? `En jeu · ${Math.max(1, Math.round((nowS() - g.since) / 60))} min` : 'Stats & trackers';
  renderLiveBanner();
}

// ---------- Mode Focus ----------
let focusMin = store.get('rez.focusMin', 25);
function renderFocus() {
  const left = focusLeft();
  const on = left > 0;
  const total = Math.max(store.get('rez.focusTotal', 0), left, 1);
  const txt = on ? mmss(left) : `${focusMin}:00`;
  $('#focusVal').textContent = $('#wgFocusVal').textContent = txt;
  $('#wgFocusTxt').textContent = on ? 'Notifs Discord coupées' : 'Prêt · ' + focusMin + ' min';
  $('#wgFocus').classList.toggle('on', on);
  $('#focusCard').classList.toggle('on', on);
  $('#focusState').textContent = on ? 'Actif' : 'Inactif';
  $('#focusState').classList.toggle('on', on);
  $('#focusArc').style.strokeDashoffset = on ? 106.8 * (1 - left / total) : 106.8;
  const b = $('#focusBtn');
  b.textContent = on ? 'Arrêter' : 'Lancer';
  b.classList.toggle('primary', !on);
  b.classList.toggle('stop', on);
}
$('#focusPresets').addEventListener('click', (e) => {
  const b = e.target.closest('[data-min]');
  if (!b) return;
  focusMin = +b.dataset.min;
  $('#focusMin').value = focusMin;
  $$('#focusPresets button').forEach((x) => x.classList.toggle('on', x === b));
  store.set('rez.focusMin', focusMin);
  renderFocus();
});
$('#focusMin').addEventListener('input', () => {
  const v = Math.round(+$('#focusMin').value);
  if (v >= 1 && v <= 600) { focusMin = v; store.set('rez.focusMin', v); $$('#focusPresets button').forEach((x) => x.classList.toggle('on', +x.dataset.min === v)); renderFocus(); }
});
$('#focusBtn').onclick = async () => {
  const b = $('#focusBtn'); b.disabled = true;
  try {
    if (focusLeft() > 0) { await call('focus_stop'); toast('Mode Focus terminé'); }
    else {
      const until = await call('focus_start', { minutes: focusMin });
      store.set('rez.focusTotal', focusMin * 60);
      live.focus_until = until;
      toast(`Focus ${focusMin} min · notifications Discord masquées`);
    }
  } catch {} finally { b.disabled = false; renderFocus(); renderPill(); schedule(); }
};

// ---------- discord ----------
$('#dSet').onclick = async () => {
  const b = $('#dSet'); b.disabled = true;
  try {
    const l = await call('discord_set', { details: $('#dDetails').value.trim() || 'Sur REZ', state: $('#dState').value.trim() || 'Dynamic Island' });
    onLive(l);
    toast(live.game && S.discord_auto ? 'Enregistrée (le jeu détecté reste prioritaire)' : 'Rich Presence activée');
  } catch {} finally { b.disabled = false; }
};
$('#dClear').onclick = async () => { try { onLive(await call('discord_clear')); toast('Présence effacée'); } catch {} };
$('#dOpen').onclick = () => call('open_uri', { uri: 'discord://-/channels/@me' }).catch(() => {});

// ---------- stats & trackers publics ----------
const enc = encodeURIComponent;
const GAMES = {
  fortnite: { name: 'Fortnite', ph: 'Pseudo Epic', opts: [['epic', 'Epic'], ['psn', 'PSN'], ['xbl', 'Xbox']],
    load: (id, o) => invoke('fortnite_stats', { name: id, platform: o, window: 'lifetime' }), render: rFortnite,
    link: (id) => `https://fortnitetracker.com/profile/all/${enc(id)}` },
  valorant: { name: 'Valorant', ph: 'Pseudo#TAG', opts: [['eu', 'EU'], ['na', 'NA'], ['ap', 'AP'], ['kr', 'KR'], ['latam', 'LATAM'], ['br', 'BR']],
    load: (id, o) => invoke('valorant_stats', { id, region: o }), render: rValorant,
    link: (id) => `https://tracker.gg/valorant/profile/riot/${enc(id)}/overview` },
  apex: { name: 'Apex Legends', ph: 'Pseudo EA / Steam', opts: [['PC', 'PC'], ['PS4', 'PS'], ['X1', 'Xbox'], ['SWITCH', 'Switch']],
    load: (id, o) => invoke('apex_stats', { name: id, platform: o }), render: rApex,
    link: (id) => `https://apex.tracker.gg/apex/profile/origin/${enc(id)}/overview` },
  r6: { name: 'Rainbow Six Siege', ph: 'Pseudo Ubisoft', opts: [['ubi', 'PC'], ['psn', 'PSN'], ['xbl', 'Xbox']],
    link: (id, o) => `https://r6.tracker.network/r6siege/profile/${o}/${enc(id)}/overview` },
  rl: { name: 'Rocket League', ph: 'Pseudo', opts: [['epic', 'Epic'], ['steam', 'Steam'], ['psn', 'PSN'], ['xbl', 'Xbox']],
    link: (id, o) => `https://rocketleague.tracker.network/rocket-league/profile/${o}/${enc(id)}/overview` },
};
let curGame = store.get('rez.game', 'fortnite');
if (!GAMES[curGame]) curGame = 'fortnite';
const statCache = {};
const box = (l, v) => `<div class="sbox"><small>${l}</small><div>${v}</div></div>`;
function selectGame(g) {
  curGame = g;
  store.set('rez.game', g);
  $$('#sGames button').forEach((b) => b.classList.toggle('on', b.dataset.game === g));
  const G = GAMES[g], P = S.players || {};
  $('#sId').placeholder = G.ph;
  $('#sId').value = P[g] || '';
  $('#sOpt').innerHTML = G.opts.map(([v, l]) => `<option value="${v}">${l}</option>`).join('');
  $('#sOpt').value = P[g + '_opt'] || G.opts[0][0];
  $('#sLink').textContent = P[g] ? 'Mettre à jour' : 'Lier ce joueur';
  renderLiveBanner();
  if (view === 'stats' && mode === 'open') loadStats(false);
}
function renderLiveBanner() {
  const b = $('#sLive');
  const g = live.game;
  const on = g && g.id === curGame;
  b.classList.toggle('hidden', !on);
  if (on) b.textContent = `En partie sur ${g.name} depuis ${Math.max(1, Math.round((nowS() - g.since) / 60))} min · stats de la game actualisées à la fin`;
}
$('#sGames').addEventListener('click', (e) => { const b = e.target.closest('[data-game]'); if (b) selectGame(b.dataset.game); });
$('#sForm').onsubmit = async (e) => {
  e.preventDefault();
  const id = $('#sId').value.trim();
  if (!id) return toast('Entre ton ID joueur', true);
  if (curGame === 'valorant' && !id.includes('#')) return toast('Format Valorant : Pseudo#TAG', true);
  S.players = { ...(S.players || {}), [curGame]: id, [curGame + '_opt']: $('#sOpt').value };
  delete statCache[curGame];
  if (await persist()) toast(`${GAMES[curGame].name} : ${id} lié`);
  loadStats(true);
};
$('#sRefresh').onclick = () => { delete statCache[curGame]; loadStats(true); };
async function loadStats(force) {
  const g = curGame, G = GAMES[g], P = S.players || {};
  const id = P[g], opt = P[g + '_opt'] || G.opts[0][0];
  const out = $('#sOut');
  if (!id) { out.innerHTML = `<p class="hint">Lie ton ID joueur ${esc(G.name)} : stats publiques uniquement, zéro accès au jeu (100 % safe anti-cheat).</p>`; return; }
  const linkBtn = `<button class="btn sm" data-uri="${esc(G.link(id, opt))}">Profil tracker</button>`;
  if (!G.load) {
    out.innerHTML = `<div class="card"><div class="stat-head"><b>${esc(id)}</b>${linkBtn}</div>
      <p class="hint">${esc(G.name)} n'a pas d'API publique officielle pour les stats : REZ ouvre ton profil tracker public dans le navigateur. Jeu lancé → détecté automatiquement (présence Discord).</p></div>`;
    return;
  }
  const c = statCache[g];
  if (!force && c && Date.now() - c.t < 120e3) { out.innerHTML = G.render(c.d, linkBtn); return; }
  out.innerHTML = `<p class="hint">Chargement ${esc(G.name)}…</p>`;
  try {
    const d = await G.load(id, opt);
    statCache[g] = { t: Date.now(), d };
    if (curGame === g) out.innerHTML = G.render(d, linkBtn);
  } catch (e) {
    if (curGame === g) out.innerHTML = `<div class="card err">${esc(e)}</div><div class="row">${linkBtn}</div>`;
  }
}
$('#sOut').addEventListener('click', (e) => { const b = e.target.closest('[data-uri]'); if (b) call('open_uri', { uri: b.dataset.uri }).catch(() => {}); });
function rFortnite(r, link) {
  const o = r.overall || {};
  const mode = (n, m) => m ? `<span>${n}</span><b>${num(m.wins)} V</b><b>${num(m.kd, 2)} K/D</b><b>${num(m.matches)} P</b>` : '';
  return `<div class="card">
    <div class="stat-head"><span><b>${esc(r.name)}</b> <span>· passe niv. ${num(r.level)}</span></span>${link}</div>
    <div class="sgrid">${box('Victoires', num(o.wins))}${box('Win %', num(o.winRate, 1))}${box('K/D', num(o.kd, 2))}
      ${box('Kills', num(o.kills))}${box('Parties', num(o.matches))}${box('Top 10', num(o.top10))}</div>
    <div class="modes">${mode('Solo', r.solo)}${mode('Duo', r.duo)}${mode('Section', r.squad)}</div></div>`;
}
function plRow(p) {
  return `<div class="pl${p.me ? ' me' : ''}">${safeUrl(p.icon) ? `<img src="${esc(p.icon)}" alt="">` : '<img alt="">'}
    <span>${esc(p.name)}<small>${esc(p.agent || '')}${p.rank ? ' · ' + esc(p.rank) : ''}</small></span><b>${num(p.k)}/${num(p.d)}/${num(p.a)}</b></div>`;
}
function rValorant(r, link) {
  const L = r.last;
  const rank = `${safeUrl(r.rank_img) ? `<img src="${esc(r.rank_img)}" alt="">` : ''}`;
  let html = `<div class="card">
    <div class="stat-head"><span><b>${esc(r.name)}#${esc(r.tag)}</b> <span>· niv. ${num(r.level)} · ${esc((r.region || '').toUpperCase())}</span></span>${rank}</div>
    <div class="sgrid">${box('Rang', esc(r.rank || 'Non classé'))}${box('RR', num(r.rr))}${box('Dernière', L?.me ? `${num(L.me.k)}/${num(L.me.d)}/${num(L.me.a)}` : '—')}</div>
    <div class="row" style="margin-top:.5rem">${link}</div></div>`;
  if (L) {
    const res = L.won === true ? '<span class="res win">Victoire</span>' : L.won === false ? '<span class="res loss">Défaite</span>' : '';
    html += `<div class="card">
      <div class="match-h"><span><b>${esc(L.map || 'Carte')}</b> <span class="hint">${esc(L.mode || '')}${L.score ? ' · ' + esc(L.score) : ''}</span></span>${res}</div>
      <div class="hint">${esc(L.date || '')}</div>
      <div class="team-t ally">Avec toi</div>${(L.allies || []).map(plRow).join('')}
      <div class="team-t enemy">Contre toi</div>${(L.enemies || []).map(plRow).join('')}</div>`;
  }
  return html;
}
function rApex(r, link) {
  const st = r.in_game ? `<span class="live">● En partie</span>` : r.online ? `<span class="live">● En ligne</span>` : '<span>Hors ligne</span>';
  const ls = (r.legend_stats || []).slice(0, 3).map((x) => box(esc(x.name), num(x.value))).join('');
  return `<div class="card">
    <div class="stat-head"><span><b>${esc(r.name)}</b> <span>· niv. ${num(r.level)}</span></span>${safeUrl(r.rank_img) ? `<img src="${esc(r.rank_img)}" alt="">` : ''}</div>
    <div class="sgrid">${box('Rang', esc(r.rank))}${box('RP', num(r.rp))}${box('Légende', esc(r.legend || '—'))}</div>
    <div class="row between" style="margin-top:.55rem;font-size:.85rem">${st}<span class="hint">${esc(r.state || '')}</span></div>
    ${ls ? `<div class="team-t">${esc(r.legend || 'Légende')} · trackers</div><div class="sgrid">${ls}</div>` : ''}
    <div class="row" style="margin-top:.5rem">${link}</div></div>`;
}

// ---------- chat IA ----------
function addMsg(role, text, cls = '') {
  const d = document.createElement('div');
  d.className = `msg ${role === 'user' ? 'user' : 'bot'} ${cls}`;
  d.textContent = text;
  $('#aLog').appendChild(d);
  $('#aLog').scrollTop = 1e9;
  return d;
}
const aIn = $('#aIn');
aIn.addEventListener('input', () => { aIn.style.height = 'auto'; aIn.style.height = aIn.scrollHeight + 'px'; });
aIn.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); $('#aForm').requestSubmit(); } });
$('#aForm').onsubmit = async (e) => {
  e.preventDefault();
  const text = aIn.value.trim();
  if (!text) return;
  aIn.value = ''; aIn.style.height = 'auto';
  chat.push({ role: 'user', content: text });
  addMsg('user', text);
  const wait = addMsg('bot', '', 'typing');
  $('#aSend').disabled = true;
  try {
    const reply = await invoke('ai_chat', { messages: chat });
    chat.push({ role: 'assistant', content: reply });
    wait.classList.remove('typing');
    wait.textContent = reply || '(réponse vide)';
  } catch (err) {
    chat.pop();
    wait.classList.remove('typing');
    wait.classList.add('err');
    wait.textContent = String(err);
  } finally {
    $('#aSend').disabled = false;
    $('#aLog').scrollTop = 1e9;
  }
};
$('#aClear').onclick = () => { chat = []; $('#aLog').innerHTML = ''; addMsg('bot', 'Conversation effacée.'); };

// ---------- outils : mini-apps ----------
document.addEventListener('click', (e) => { const b = e.target.closest('[data-launch]'); if (b) call('launch', { target: b.dataset.launch }).catch(() => {}); });
function calc(src) {
  const s = src.replace(/\s+/g, '').replace(/,/g, '.').replace(/[x×]/gi, '*').replace(/÷/g, '/');
  let i = 0;
  const peek = () => s[i];
  const fail = () => { throw new Error(); };
  function atom() {
    if (peek() === '-') { i++; return -atom(); }
    if (peek() === '+') { i++; return atom(); }
    let v;
    if (peek() === '(') { i++; v = expr(); if (s[i++] !== ')') fail(); }
    else { const m = /^\d*\.?\d+(?:e[+-]?\d+)?/i.exec(s.slice(i)); if (!m) fail(); i += m[0].length; v = parseFloat(m[0]); }
    if (peek() === '%') { i++; v /= 100; }
    return v;
  }
  function pow() { const b = atom(); if (peek() === '^') { i++; return b ** pow(); } return b; }
  function term() { let v = pow(); while (peek() === '*' || peek() === '/') { const o = s[i++]; const r = pow(); v = o === '*' ? v * r : v / r; } return v; }
  function expr() { let v = term(); while (peek() === '+' || peek() === '-') { const o = s[i++]; const r = term(); v = o === '+' ? v + r : v - r; } return v; }
  const v = expr();
  if (i !== s.length || !isFinite(v)) fail();
  return v;
}
$('#calcIn').addEventListener('input', () => {
  const t = $('#calcIn').value.trim();
  try { $('#calcOut').textContent = t ? '= ' + num(calc(t), 8) : '='; } catch { $('#calcOut').textContent = '= …'; }
});
$('#calcIn').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') { try { $('#calcIn').value = String(+calc($('#calcIn').value).toPrecision(12)); $('#calcOut').textContent = '='; } catch {} }
});
const notes = $('#notes');
notes.value = store.get('rez.notes', '');
let notesT;
notes.addEventListener('input', () => { clearTimeout(notesT); notesT = setTimeout(() => store.set('rez.notes', notes.value), 300); });

// ---------- fichiers ----------
const FOLDER = '<svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/></svg>';
function renderPins() {
  const pins = S.pins || [];
  $('#fGrid').innerHTML = pins.map((p, i) => `
    <button class="fitem" data-i="${i}" title="${esc(p.path)}">
      <span class="fi">${FOLDER}</span>
      <span class="fn"><b>${esc(p.name)}</b><small>${esc(p.path.startsWith('shell:') ? 'Système' : p.path)}</small></span>
      <span class="rm" data-rm="${i}" title="Retirer">✕</span>
    </button>`).join('');
  $('#quickPins').innerHTML = pins.slice(0, 4).map((p, i) => `<button data-pin="${i}" title="${esc(p.path)}">${FOLDER}<span>${esc(p.name)}</span></button>`).join('');
}
$('#quickPins').addEventListener('click', (e) => { const b = e.target.closest('[data-pin]'); if (b) call('open_path', { path: S.pins[+b.dataset.pin].path }).catch(() => {}); });
$('#fGrid').addEventListener('click', async (e) => {
  const rm = e.target.closest('[data-rm]');
  if (rm) {
    e.stopPropagation();
    S.pins.splice(+rm.dataset.rm, 1);
    await persist();
    return;
  }
  const it = e.target.closest('.fitem');
  if (it) call('open_path', { path: S.pins[+it.dataset.i].path }).catch(() => {});
});
async function addPin(path, name) {
  path = path.trim().replace(/^"|"$/g, '');
  if (!path) return;
  if ((S.pins || []).some((p) => p.path.toLowerCase() === path.toLowerCase())) return toast('Déjà épinglé');
  name = (name || '').trim() || path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path;
  S.pins = [...(S.pins || []), { name, path }];
  await persist();
  toast(`« ${name} » épinglé`);
}
$('#fForm').onsubmit = async (e) => {
  e.preventDefault();
  await addPin($('#fPath').value);
  $('#fPath').value = '';
};
listen('tauri://drag-enter', () => document.body.classList.add('drop'));
listen('tauri://drag-leave', () => document.body.classList.remove('drop'));
listen('tauri://drag-drop', async (e) => {
  document.body.classList.remove('drop');
  if (mode !== 'open') expand();
  for (const p of e.payload?.paths || []) await addPin(p);
  go('files');
});

// ---------- paramètres ----------
function applyLook() {
  const large = sizeKey() === 'large';
  document.documentElement.classList.toggle('large', large);
  $('#pSize').classList.toggle('large', large);
  $$('#pSize button').forEach((b) => b.classList.toggle('on', b.dataset.size === sizeKey()));
  const op = Math.round((S.opacity ?? 0.72) * 100);
  document.documentElement.style.setProperty('--alpha', (op / 100).toFixed(2));
  setRange($('#pOpacity'), op);
  $('#pOpacity').style.setProperty('--p', ((op - 25) / 75) * 100 + '%');
  $('#pOpVal').textContent = op + '%';
  const free = S.locked === false;
  document.body.classList.toggle('free', free);
  $('#btnLock').classList.toggle('hidden', !free);
  $('#pUnlock').checked = free;
  $('#pUnlockHint').textContent = free ? 'Glisse la pilule ou l\'en-tête pour déplacer l\'île. 🔒 pour la reverrouiller.' : 'Île ancrée en haut de l\'écran principal.';
}
function fillSettings() {
  $('#pShortcut').value = S.shortcut;
  $('#pHide').value = S.hide_shortcut || '';
  $('#pCity').value = S.city;
  $('#pDiscord').value = S.discord_client_id;
  $('#pDAuto').checked = S.discord_auto;
  $('#pFn').value = S.fortnite_key;
  $('#pHenrik').value = S.henrik_key;
  $('#pApex').value = S.apex_key;
  $('#pAi').value = S.ai_key;
  $('#pModel').value = S.ai_model;
  $('#pAuto').checked = S.autostart;
  $('#pBlur').checked = S.hide_on_blur;
  $('#pPill').checked = S.pill_mode;
  $('#dDetails').value = S.discord_details;
  $('#dState').value = S.discord_state;
  applyLook();
  renderPins();
}
async function persist(extra = {}) {
  try {
    S = await call('save_settings', { s: { ...S, ...extra } });
    fillSettings();
    return true;
  } catch { return false; }
}
async function setLocked(locked) {
  if (await persist({ locked })) toast(locked ? 'Île reverrouillée en haut' : 'Déverrouillée : glisse l\'île où tu veux');
  else applyLook();
}
$('#pUnlock').onchange = () => setLocked(!$('#pUnlock').checked);
$('#pSize').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-size]');
  if (!b || b.dataset.size === sizeKey()) return;
  S.size = b.dataset.size;
  applyLook();
  render(mode);
  await persist();
});
const opIn = $('#pOpacity');
opIn.addEventListener('input', () => {
  const v = +opIn.value;
  document.documentElement.style.setProperty('--alpha', (v / 100).toFixed(2));
  opIn.style.setProperty('--p', ((v - 25) / 75) * 100 + '%');
  $('#pOpVal').textContent = v + '%';
});
opIn.addEventListener('change', () => persist({ opacity: +opIn.value / 100 }));
$$('.sc').forEach((inp) => inp.addEventListener('keydown', (e) => {
  e.preventDefault();
  if (e.key === 'Backspace' && inp.id === 'pHide') { inp.value = ''; return; }
  if (/^(Control|Shift|Alt|Meta|OS)/.test(e.code)) return;
  const mods = [];
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');
  const key = e.code.replace(/^Key/, '').replace(/^Digit/, '');
  if (!mods.length && !/^F\d{1,2}$/.test(key)) return toast('Ajoute Ctrl, Alt ou Shift', true);
  inp.value = [...mods, key].join('+');
}));
$('#pSave').onclick = async () => {
  const city = $('#pCity').value.trim();
  const cityChanged = city && city !== S.city;
  const ok = await persist({
    shortcut: $('#pShortcut').value.trim(),
    hide_shortcut: $('#pHide').value.trim(),
    discord_client_id: $('#pDiscord').value.trim(),
    discord_auto: $('#pDAuto').checked,
    fortnite_key: $('#pFn').value.trim(),
    henrik_key: $('#pHenrik').value.trim(),
    apex_key: $('#pApex').value.trim(),
    ai_key: $('#pAi').value.trim(),
    ai_model: $('#pModel').value.trim() || 'claude-opus-5-5',
    autostart: $('#pAuto').checked,
    hide_on_blur: $('#pBlur').checked,
    pill_mode: $('#pPill').checked,
  });
  if (ok) {
    toast(`Enregistré · ${S.shortcut}${S.hide_shortcut ? ' / ' + S.hide_shortcut : ''}`);
    if (cityChanged) loadWeather(city);
    else $('#pCity').value = S.city;
  }
};
$('#pQuit').onclick = () => invoke('quit_app');

// ---------- mise à jour ----------
async function checkUpdate(manual = false) {
  try {
    const r = await invoke('check_update');
    if (r.available) {
      $('#updVer').textContent = 'v' + r.latest;
      $('#updNotes').textContent = r.notes || 'Nouvelle version disponible';
      $('#updCard').classList.remove('hidden');
      $('#setBadge').classList.remove('hidden');
      $('#pillDot').classList.add('upd');
      if (manual) { toast(`v${r.latest} disponible`); go('home'); goSub('ov'); }
    } else if (manual) toast(`REZ est à jour (v${r.current})`);
  } catch (e) { if (manual) toast(String(e), true); }
}
$('#pCheck').onclick = () => checkUpdate(true);
$('#updBtn').onclick = async () => {
  const b = $('#updBtn');
  b.disabled = true; b.textContent = '…';
  try { await invoke('install_update'); }
  catch (e) { toast(String(e), true); b.disabled = false; b.textContent = 'Réessayer'; }
};

// ---------- init ----------
(async () => {
  try { S = await invoke('get_settings'); fillSettings(); } catch {}
  $('#pVer').textContent = 'v' + (await invoke('app_version').catch(() => '?'));
  $('#focusMin').value = focusMin;
  $$('#focusPresets button').forEach((x) => x.classList.toggle('on', +x.dataset.min === focusMin));
  selectGame(curGame);
  onLive(await invoke('live_state').catch(() => null));

  setVars('pill');
  const st = await invoke('startup_state').catch(() => ({ visible: true, hidden: false }));
  winVisible = st.visible;
  const [pw, ph] = SIZES[sizeKey()].pill;
  winW = 0; winH = 0;
  await fit(pw, ph);
  schedule();
  if (st.visible && !st.hidden) setTimeout(expand, 250);
  loadWeather();
  checkUpdate();
})();
