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
  compact: { pill: [210, 34], open: [372, 452] },
  large:   { pill: [244, 40], open: [432, 520] },
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
const size = (b) => { b = +b || 0; const u = ['o', 'Ko', 'Mo', 'Go', 'To']; let i = 0; while (b >= 1000 && i < 4) { b /= 1000; i++; } return `${num(b, b < 10 && i ? 1 : 0)} ${u[i]}`; };
const rate = (b) => size(b) + '/s';
const dur = (s) => { s = Math.round(s); if (s < 60) return `${s} s`; if (s < 3600) return `${Math.round(s / 60)} min`; const h = Math.floor(s / 3600); return `${h} h ${String(Math.round((s % 3600) / 60)).padStart(2, '0')}`; };
let voice = { status: 'off', members: [] };

// ---------- île : pilule <-> ouverte ----------
async function fit(w, h, focus = false) {
  winW = w; winH = h;
  try { applyAlign(await invoke('island_fit', { width: w, height: h, focus })); } catch {}
}
// ancrage de l'île dans sa fenêtre : elle grandit vers l'intérieur de l'écran
function applyAlign(a) {
  if (!a?.align) return;
  const b = document.body;
  b.classList.remove('al-l', 'al-c', 'al-r', 'al-t', 'al-m', 'al-b', 'dock-top', 'dock-left', 'dock-right', 'dock-free');
  b.classList.add('al-' + a.align[0], 'al-' + a.align[1], 'dock-' + (a.dock || 'top'));
  if (a.dock && a.dock !== S.dock) { S.dock = a.dock; renderDock(); }
}
listen('rez://align', (e) => applyAlign(e.payload));
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
  if (editing) setEditing(false);
  setTimeout(maybeAutoInstall, 1500);
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
$('#btnLock').onclick = () => setLocked(S.locked !== false ? false : true);
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape' || document.activeElement?.classList.contains('sc')) return;
  if (view === 'calc' && cMode === 'calc' && (cExpr || cJust)) return; // Échap efface la calculatrice
  if (editing) return setEditing(false);
  collapse();
});

// ---------- navigation ----------
const PAGES = ['settings', 'calc', 'weather', 'dl'];   // pages dédiées (hors dock) : bouton retour
let backTo = 'home';
function go(v) {
  if (v !== view) $('#views').scrollTop = 0;
  if (editing && v !== 'home') setEditing(false);
  if (PAGES.includes(v) && !PAGES.includes(view)) backTo = view;
  view = v;
  $$('.view').forEach((el) => el.classList.toggle('active', el.dataset.view === v));
  $$('.dock button').forEach((b) => b.classList.toggle('active', b.dataset.go === v));
  onView();
  schedule();
}
document.addEventListener('click', (e) => {
  const b = e.target.closest('[data-go]');
  if (b) {
    if (b.dataset.go === 'stats' && live.game && GAMES[live.game.id]) selectGame(live.game.id);
    go(b.dataset.go);
  }
  if (b?.hasAttribute('data-mix')) $('#mTabs [data-msub="mix"]').click();
  if (e.target.closest('[data-back]')) go(PAGES.includes(view) ? backTo : 'home');
});
$('#btnSettings').onclick = () => go(view === 'settings' ? backTo : 'settings');
function onView() {
  if (mode !== 'open') return;
  if ((view === 'home' || view === 'weather') && Date.now() - lastWeather > 10 * 60e3) loadWeather();
  if (view === 'stats') loadStats(false);
  if (view === 'ai') setTimeout(() => $('#aIn').focus(), 60);
  $('#btnSettings').classList.toggle('on', view === 'settings');
}

// ---------- timers (tout est en pause quand REZ est masqué) ----------
function focusLeft() { return live.focus_until ? live.focus_until - nowS() : 0; }
let mixTimer = null, msub = 'player', perfTimer = null, devTimer = null;
function schedule() {
  clearInterval(clockTimer); clearInterval(mediaTimer); clearInterval(sysTimer); clearInterval(mixTimer);
  clearInterval(perfTimer); clearInterval(devTimer);
  clockTimer = mediaTimer = sysTimer = mixTimer = perfTimer = devTimer = null;
  if (!winVisible) return;
  tick();
  clockTimer = setInterval(tick, mode === 'open' || focusLeft() > 0 || tm.end ? 1000 : 5000);
  const fast = mode === 'open' && ((view === 'home' && wOn('media')) || view === 'media');
  refreshMedia();
  mediaTimer = setInterval(refreshMedia, fast ? 2000 : 5000);
  if (mode === 'open' && view === 'home' && wOn('sys')) { refreshSys(); sysTimer = setInterval(refreshSys, 2000); }
  if (mode === 'open' && view === 'media' && msub === 'mix') { refreshMix(); mixTimer = setInterval(refreshMix, 900); }
  if (mode === 'open' && view === 'perf') { refreshPerf(); perfTimer = setInterval(refreshPerf, 1000); }
  // batteries : la liste est en cache côté Rust (20 s), la détection reste très légère
  const devView = mode === 'open' && view === 'devices';
  if (devView || !devOff()) {
    refreshDevices(devView);
    devTimer = setInterval(() => refreshDevices(false), devView ? 15e3 : 60e3);
  }
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
  $('#wDate').textContent = fmtDate.format(d).replace(/\./g, '');
  renderTimer();
  renderFocus();
  renderProgress();
  if (focusLeft() > 0 || tm.end || mode === 'pill') renderPill();
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
// priorité : Focus > vocal Discord > téléchargement > musique > jeu > météo
function renderPill() {
  const left = focusLeft();
  const playing = media?.playing && media?.has_session;
  const ch = voice?.channel;
  const dl = topDownload();
  const dlPct = dl && dl.pct != null && dl.state !== 'En pause';
  const what = left > 0 ? 'focus' : tm.end ? 'timer' : ch ? 'voice' : dlPct ? 'dl' : playing ? 'media' : live.game ? 'game' : dl ? 'dl' : 'wx';
  const info = $('#pillInfo');
  const dot = $('#pillDot');
  dot.classList.toggle('focus', what === 'focus' || what === 'timer');
  dot.classList.toggle('voice', what === 'voice');
  dot.classList.toggle('dl', what === 'dl');
  dot.classList.toggle('game', what === 'game' || (what === 'media' && !!live.game));
  info.classList.toggle('focus', what === 'focus' || what === 'timer');
  $('#pillArt').classList.toggle('hidden', !(what === 'media' && $('#pillArt').dataset.ok === '1'));
  $('#pillBars').classList.toggle('hidden', what !== 'media');
  // avatars du salon (ceux qui parlent d'abord)
  const pv = $('#pillVoice');
  if (what === 'voice') {
    const ms = [...(voice.members || [])].sort((a, b) => b.speaking - a.speaking).slice(0, 3);
    const html = ms.map((m) => safeUrl(m.avatar) ? `<img src="${esc(m.avatar)}" class="${m.speaking ? 'sp' : ''}" alt="">` : '').join('');
    if (pv.innerHTML !== html) pv.innerHTML = html;
  }
  pv.classList.toggle('hidden', what !== 'voice');
  // fine barre de progression du téléchargement, en bas de la pilule
  const pp = $('#pillProg');
  pp.classList.toggle('hidden', !(dl && dl.pct != null));
  if (dl && dl.pct != null) pp.firstElementChild.style.width = dl.pct.toFixed(1) + '%';
  if (what === 'focus') info.textContent = `Focus ${mmss(left)}`;
  else if (what === 'timer') info.textContent = `⏱ ${mmss(Math.ceil(tmLeft()))}`;
  else if (what === 'voice') {
    const sp = (voice.members || []).filter((m) => m.speaking && !m.me).map((m) => m.name);
    info.textContent = sp.length ? sp.join(', ') : ch.name;
  } else if (what === 'dl') info.textContent = dlPct ? `↓ ${Math.floor(dl.pct)}%${dl.eta != null ? ' · ' + dur(dl.eta) : ''}` : `↓ ${rate(dl.speed)}`;
  else if (what === 'media') info.textContent = media.title || appInfo(media.app)?.name || '';
  else if (what === 'game') info.textContent = `🎮 ${live.game.name}`;
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
  $('#npTime').textContent = has ? `${mmss(pos)} / ${mmss(m.duration)}` : '';
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
function gauge(id, pct, sub, temp) {
  const g = $(id);
  const ok = pct != null && isFinite(pct);
  g.querySelector('.arc').style.strokeDashoffset = ok ? 106.8 * (1 - Math.min(100, Math.max(0, pct)) / 100) : 106.8;
  g.querySelector('b').textContent = ok ? Math.round(pct) + '%' : '—';
  const em = g.querySelector('em');
  if (temp !== undefined) {
    em.textContent = temp != null ? Math.round(temp) + '°C' : '—';
    em.className = 'temp' + (temp == null ? '' : temp >= 85 ? ' t-hot' : temp >= 70 ? ' t-warm' : ' t-ok');
  } else em.textContent = sub;
}
async function refreshSys() {
  try {
    const s = await invoke('sys_info');
    gauge('#gCpu', s.cpu, '', s.cpu_temp);
    gauge('#gGpu', s.gpu, '', s.gpu_temp);
    gauge('#gRam', s.ram, `${num(s.used, 1)}/${num(s.total, 0)} Go`);
    const vp = s.vram_total ? (s.vram_used / s.vram_total) * 100 : null;
    gauge('#gVram', vp, s.vram_total ? `${num(s.vram_used, 1)}/${num(s.vram_total, 0)} Go` : '—');
    $('#gCpu em').title = s.cpu_temp == null ? 'Windows ne fournit pas la température CPU sur ce PC (il faudrait un pilote noyau, bloqué par les anti-cheats)' : 'Zone thermique ACPI';
    $('#gpuName').textContent = s.gpu_name || '';
  } catch {}
}

// ---------- téléchargements ----------
const topDownload = () => {
  const d = live.downloads || [];
  return d.find((x) => x.pct != null && x.state !== 'En pause') || d.find((x) => x.pct != null) || d[0] || null;
};
function renderDownloads() {
  const d = live.downloads || [];
  const top = topDownload();
  $('#netVal').textContent = rate(live.net || 0);
  $('#wgDl').classList.toggle('on', !!d.length);
  $('#wgDlVal').textContent = top ? (top.pct != null ? `${Math.floor(top.pct)}%` : rate(top.speed)) : 'Aucun';
  $('#wgDlSub').textContent = top ? `${top.name}` : `↓ ${rate(live.net || 0)}`;
  $('#wgDlBar').style.width = (top?.pct ?? 0) + '%';
  $('#dlList').innerHTML = d.length ? d.map((x) => {
    const known = x.pct != null;
    const parts = [
      known ? `${size(x.done)} / ${size(x.total)}` : x.done ? size(x.done) : '',
      x.speed > 0 ? rate(x.speed) : '',
      x.eta != null ? `reste ${dur(x.eta)}` : '',
      x.state && x.state !== 'Téléchargement' ? x.state : '',
    ].filter(Boolean).join(' · ');
    return `<div class="card dl"><div class="dl-h"><span class="src">${esc(x.source)}</span><b title="${esc(x.name)}">${esc(x.name)}</b>${known ? `<span class="pct">${num(x.pct, 1)}%</span>` : ''}</div>
      <div class="bar${known ? '' : ' ind'}"><i style="width:${known ? x.pct : 35}%"></i></div><small>${esc(parts)}</small></div>`;
  }).join('') : '<p class="hint">Aucun téléchargement en cours.</p>';
}

// ---------- mélangeur audio ----------
const EXES = {
  '#system': ['Sons système', '#64748b', '🔔'], 'fortniteclient': ['Fortnite', '#9d4dff', 'F'], 'valorant': ['Valorant', '#ff4655', 'V'],
  'r5apex': ['Apex Legends', '#da292a', 'A'], 'rainbowsix': ['Rainbow Six', '#3d8bfd', 'R'], 'rocketleague': ['Rocket League', '#1fa2ff', 'R'],
  'cs2': ['Counter-Strike 2', '#de9b35', 'C'], 'league of legends': ['League of Legends', '#c89b3c', 'L'], 'medal': ['Medal', '#f5c518', 'M'],
  'obs64': ['OBS', '#302e31', 'O'], 'steamwebhelper': ['Steam', '#1b2838', 'S'], 'teams': ['Teams', '#5b5fc7', 'T'], 'whatsapp': ['WhatsApp', '#25d366', 'W'],
  'rtkuwp': ['Realtek Audio', '#4b5563', 'R'], 'nvcontainer': ['NVIDIA', '#76b900', 'N'], 'radeonsoftware': ['AMD Software', '#ed1c24', 'A'],
};
function exeInfo(exe) {
  const k = Object.keys(EXES).find((x) => exe === x || exe.startsWith(x));
  if (k) { const [name, color, glyph] = EXES[k]; return { name, color, glyph }; }
  const a = APPS.find(([x]) => exe.includes(x));
  if (a) return { name: a[1], color: a[2], glyph: a[3] };
  const stem = exe.replace(/\.exe$/, '');
  return { name: stem.charAt(0).toUpperCase() + stem.slice(1), color: '#6d28d9', glyph: esc(stem.charAt(0).toUpperCase() || '♪') };
}
let mixDrag = null, mixT;
async function refreshMix() {
  let list;
  try { list = await invoke('mixer_list'); } catch (e) { $('#mixList').innerHTML = `<div class="card err">${esc(e)}</div>`; return; }
  const box = $('#mixList');
  if (!list.length) { box.innerHTML = '<p class="hint">Aucune application ne joue de son.</p>'; return; }
  const keys = list.map((x) => x.key).join('|');
  if (box.dataset.keys !== keys) {
    box.dataset.keys = keys;
    box.innerHTML = list.map((x) => {
      const a = exeInfo(x.key);
      return `<div class="mix" data-key="${esc(x.key)}"><span class="ico" style="background:${a.color}">${a.glyph}</span>
        <div class="mix-m"><div class="mix-t"><b>${esc(a.name)}</b><span class="v"></span></div>
        <input type="range" min="0" max="100"><div class="peak"><i></i></div></div>
        <button class="icon-btn" title="Couper le son"><svg viewBox="0 0 24 24"></svg></button></div>`;
    }).join('');
  }
  for (const x of list) {
    const row = box.querySelector(`.mix[data-key="${CSS.escape(x.key)}"]`);
    if (!row) continue;
    row.classList.toggle('off', !x.active);
    const v = Math.round(x.volume * 100);
    if (mixDrag !== x.key) { setRange(row.querySelector('input'), v); row.querySelector('.v').textContent = x.muted ? 'Muet' : v + ' %'; }
    row.querySelector('.peak i').style.width = (x.muted ? 0 : Math.min(100, x.peak * 140)) + '%';
    const btn = row.querySelector('button');
    btn.classList.toggle('on', x.muted);
    btn.dataset.muted = x.muted ? '1' : '';
    btn.querySelector('svg').innerHTML = x.muted ? ICON_MUTE : ICON_VOL;
  }
}
$('#mixList').addEventListener('input', (e) => {
  const r = e.target.closest('input[type=range]');
  if (!r) return;
  const key = r.closest('.mix').dataset.key;
  mixDrag = key;
  setRange(r, r.value);
  r.closest('.mix').querySelector('.v').textContent = r.value + ' %';
  clearTimeout(mixT);
  mixT = setTimeout(() => invoke('mixer_set', { key, volume: r.value / 100, muted: null }).catch(() => {}), 30);
});
$('#mixList').addEventListener('change', () => { setTimeout(() => { mixDrag = null; }, 400); });
$('#mixList').addEventListener('click', async (e) => {
  const b = e.target.closest('button');
  if (!b) return;
  try { await call('mixer_set', { key: b.closest('.mix').dataset.key, volume: null, muted: !b.dataset.muted }); refreshMix(); } catch {}
});
$('#mTabs').addEventListener('click', (e) => {
  const b = e.target.closest('[data-msub]');
  if (!b) return;
  msub = b.dataset.msub;
  $$('#mTabs button').forEach((x) => x.classList.toggle('on', x === b));
  $$('.msub').forEach((el) => el.classList.toggle('on', el.dataset.msub === msub));
  schedule();
});

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
  renderDownloads();
  renderFocus();
  renderPill();
  if (!!before !== !!l.focus_until) schedule();
  if (pendingUpd) maybeAutoInstall();
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
  const state = d.state || (d.running ? 'waiting' : 'off');
  const [txt, cls] = d.connected ? ['Connecté · Rich Presence prête', 'on'] : DSTATE[state] || DSTATE.waiting;
  st.textContent = txt;
  st.className = 'status ' + cls;
  renderWizard(state);
  $('#dName').textContent = u?.name || 'Discord';
  $('#dUser').textContent = u ? '@' + (u.username || '') : (d.running ? 'Discord lancé' : 'Non connecté');
  $('#dBadge').classList.toggle('hidden', !d.connected);
  const a = d.activity;
  $('#dAct').textContent = a ? a.details : 'Aucune';
  $('#dActSub').textContent = a ? a.state : (S.discord_auto ? 'Lance un jeu : présence automatique' : 'Présence auto désactivée');
  $('#wgDName').textContent = u?.name || (d.running ? 'Discord' : 'Hors ligne');
  $('#wgDSub').textContent = a ? a.details : (u ? '@' + u.username : (DSTATE[state] || DSTATE.waiting)[0]);
  if (voice.channel) renderVoice();
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
  $('#fcVal').textContent = on ? mmss(left) : `${focusMin}:00`;
  $('#fcTxt').textContent = on ? 'Notifs Discord coupées' : 'Coupe les notifs Discord';
  wEl('focus').classList.toggle('on', on);
  $('#fcBtn svg').innerHTML = on ? '<rect x="6.5" y="6.5" width="11" height="11" rx="2"/>' : ICON_PLAY;
}
function fcAdj(d) {
  if (focusLeft() > 0) return;
  focusMin = Math.min(600, Math.max(5, focusMin + d * ((d > 0 ? focusMin >= 30 : focusMin > 30) ? 15 : 5)));
  store.set('rez.focusMin', focusMin);
  renderFocus();
}
$('#fcMinus').onclick = () => fcAdj(-1);
$('#fcPlus').onclick = () => fcAdj(1);
$('#fcBtn').onclick = async () => {
  const b = $('#fcBtn'); b.disabled = true;
  try {
    if (focusLeft() > 0) { await call('focus_stop'); live.focus_until = 0; toast('Mode Focus terminé'); }
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
  const l = await invoke('discord_set', { details: $('#dDetails').value.trim() || 'Sur REZ', state: $('#dState').value.trim() || 'Dynamic Island' }).catch(() => null);
  if (l) onLive(l);
  toast(!live.discord?.connected ? 'Enregistrée : affichée dès que Discord sera connecté'
    : live.game && S.discord_auto ? 'Enregistrée (le jeu détecté reste prioritaire)' : 'Rich Presence activée');
};
$('#dClear').onclick = async () => { const l = await invoke('discord_clear').catch(() => null); if (l) onLive(l); toast('Présence effacée'); };
$('#dOpen').onclick = () => call('open_uri', { uri: 'discord://-/channels/@me' }).catch(() => {});

// ---------- discord : vocal en direct ----------
const MIC_X = '<svg viewBox="0 0 24 24"><rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0014 0M4 4l16 16"/></svg>';
const HEAD_X = '<svg viewBox="0 0 24 24"><path d="M4 15v-3a8 8 0 0116 0v3M4 4l16 16"/></svg>';
const DSTATE = {
  off: ['Discord fermé · connexion auto à son lancement', ''], no_id: ['À connecter (1 minute)', 'wait'],
  invalid_id: ['Application ID à vérifier', 'wait'], waiting: ['En attente de Discord…', 'wait'], ready: ['Connecté · Rich Presence prête', 'on'],
};
const VOICE_TXT = { authorizing: 'Clique « Autoriser » dans la fenêtre Discord…', connecting: 'Connexion au vocal…' };

// ---------- assistant de connexion Discord ----------
const WZ_ERR = {
  invalid: 'Discord ne reconnaît pas cet ID : copie bien « Application ID » (General Information), pas ton ID utilisateur.',
  closed: 'Lance Discord : REZ réessaie tout seul dès qu\'il est prêt.',
  noreply: 'Discord démarre encore… nouvel essai automatique.',
};
let wzTimer = 0, wzBusy = false;
function wzMsg(t, cls = '') { const m = $('#wzMsg'); m.textContent = t; m.className = 'hint wz-msg ' + cls; }
const pickId = (t) => (String(t).match(/\d{17,20}/) || [''])[0];
function renderWizard(state) {
  const show = state === 'no_id' || state === 'invalid_id' || (state === 'off' && !S.discord_client_id);
  const w = $('#dWiz');
  const was = !w.classList.contains('hidden');
  w.classList.toggle('hidden', !show);
  if (show && !was) {
    if (!$('#wzId').value && S.discord_client_id) $('#wzId').value = S.discord_client_id;
    if (state === 'invalid_id') wzMsg(WZ_ERR.invalid, 'warn');
  }
  if (!show) clearTimeout(wzTimer);
}
async function wizTest() {
  clearTimeout(wzTimer);
  const id = pickId($('#wzId').value);
  if (!id) return wzMsg('Colle l\'Application ID : un nombre de 17 à 20 chiffres.', 'warn');
  if (wzBusy) return;
  $('#wzId').value = id;
  wzBusy = true; $('#wzGo').disabled = true;
  wzMsg('Vérification auprès de Discord…', 'busy');
  try {
    await invoke('discord_test', { cid: id });
    if (await persist({ discord_client_id: id })) { wzMsg('✓ Discord accepte cet ID : connexion en cours…', 'ok'); toast('Discord connecté à REZ'); }
  } catch (e) {
    const k = String(e);
    wzMsg(WZ_ERR[k] || k, k === 'invalid' ? 'warn' : 'busy');
    // Discord pas prêt : nouvel essai discret tant que l'assistant est affiché
    if (k === 'closed' || k === 'noreply') wzTimer = setTimeout(() => { if (!$('#dWiz').classList.contains('hidden')) wizTest(); }, 6000);
  } finally { wzBusy = false; $('#wzGo').disabled = false; }
}
$('#wzGo').onclick = wizTest;
$('#wzId').addEventListener('paste', () => setTimeout(wizTest, 30));
$('#wzId').addEventListener('keydown', (e) => { if (e.key === 'Enter') wizTest(); });
$('#wzPortal').onclick = () => invoke('open_uri', { uri: 'https://discord.com/developers/applications' }).catch(() => {});

listen('rez://voice', (e) => onVoice(e.payload));
function onVoice(v) {
  if (!v) return;
  voice = v;
  renderVoice();
  renderPill();
}
function renderVoice() {
  const v = voice, ch = v.channel;
  const ready = v.status === 'ready';
  const setup = ['need_secret', 'need_auth', 'authorizing'].includes(v.status);
  $('#vCard').classList.toggle('hidden', !ready);
  $('#vSetup').classList.toggle('hidden', !setup);
  const hasSecret = v.status !== 'need_secret';
  $('#vSecret').classList.toggle('hidden', hasSecret);
  $$('#vSetup .steps li')[0]?.classList.toggle('done', hasSecret);
  $('#vStep').textContent = hasSecret ? 'Secret enregistré ✓' : '';
  const t = $('#vSetupTxt');
  if (VOICE_TXT[v.status]) { t.textContent = VOICE_TXT[v.status]; t.className = 'hint wz-msg busy'; }
  else if (v.error) { t.textContent = v.error; t.className = 'hint wz-msg warn'; }
  else { t.textContent = ''; t.className = 'hint wz-msg'; }
  $('#vAuth').textContent = v.error ? 'Réessayer' : 'Activer le vocal';
  $('#vAuth').disabled = v.status === 'authorizing';
  if (ready) {
    $('#vGuild').textContent = ch ? (ch.guild || 'Vocal') : 'Vocal';
    $('#vChan').textContent = ch ? ch.name : 'Pas en vocal';
    $('#vCount').textContent = ch ? `${v.members.length} 👤` : '';
    // reconstruit seulement si la liste change ; sinon juste l'anneau « parle » (pas de clignotement)
    const box = $('#vMembers');
    const key = ch ? ch.id + v.members.map((m) => `|${m.id}${m.mute ? 'm' : ''}${m.deaf ? 'd' : ''}${m.name}${m.avatar}`).join('') : '';
    if (box.dataset.key === key) {
      v.members.forEach((m, i) => box.children[i]?.classList.toggle('sp', !!m.speaking));
    } else box.dataset.key = key, box.innerHTML = ch ? v.members.map((m) => `<div class="vm${m.speaking ? ' sp' : ''}${m.me ? ' me' : ''}" title="${esc(m.name)}">
      <span class="av">${safeUrl(m.avatar) ? `<img src="${esc(m.avatar)}" alt="">` : `<span class="ph">${esc(m.name.charAt(0).toUpperCase())}</span>`}
      <span class="st">${m.mute ? `<i>${MIC_X}</i>` : ''}${m.deaf ? `<i>${HEAD_X}</i>` : ''}</span></span><small>${esc(m.name)}</small></div>`).join('') : '';
    $('#vMute').classList.toggle('on', !!v.mute);
    $('#vDeaf').classList.toggle('on', !!v.deaf);
    $('#vLeave').disabled = !ch;
  }
  // widget Discord de l'accueil
  if (ready && ch) {
    const sp = v.members.filter((m) => m.speaking).map((m) => m.name);
    $('#wgDName').textContent = '🔊 ' + ch.name;
    $('#wgDSub').textContent = sp.length ? `${sp.join(', ')} parle` : `${v.members.length} dans le salon`;
  }
}
async function voiceCmd(action, on = false) {
  try { await call('voice_cmd', { action, on }); } catch {}
}
$('#vMute').onclick = () => voiceCmd('mute', !voice.mute);
$('#vDeaf').onclick = () => voiceCmd('deaf', !voice.deaf);
$('#vLeave').onclick = () => voiceCmd('leave');
$('#vPortal').onclick = () => call('open_uri', { uri: 'https://discord.com/developers/applications' }).catch(() => {});
$('#vCopy').onclick = async () => {
  try { await navigator.clipboard.writeText('http://localhost'); toast('http://localhost copié'); }
  catch { getSelection().selectAllChildren($('#vSetup .cp')); toast('Sélectionné : Ctrl+C pour copier'); }
};
$('#vAuth').onclick = async () => {
  const sec = $('#vSecret').value.trim();
  if (voice.status === 'need_secret') {
    if (!sec) { $('#vSetupTxt').textContent = 'Colle d\'abord le Client Secret (étape 1).'; $('#vSetupTxt').className = 'hint wz-msg warn'; return; }
    if (!(await persist({ discord_secret: sec }))) return;
    $('#vSecret').value = '';
  }
  const b = $('#vAuth'); b.disabled = true;
  try { await invoke('voice_auth'); toast('Vocal Discord activé'); }
  catch (e) { const t = $('#vSetupTxt'); t.textContent = String(e); t.className = 'hint wz-msg warn'; }
  finally { b.disabled = false; }
};

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

// ---------- chat IA : Claude / ChatGPT / Gemini (clé optionnelle) ----------
const PROVIDERS = {
  claude: { name: 'Claude', vendor: 'Anthropic', field: 'ai_key', model: 'claude-opus-5-5', url: 'https://console.anthropic.com/settings/keys', prefix: 'claude' },
  openai: { name: 'ChatGPT', vendor: 'OpenAI', field: 'openai_key', model: 'gpt-5-mini', url: 'https://platform.openai.com/api-keys', prefix: 'gpt' },
  gemini: { name: 'Gemini', vendor: 'Google AI Studio', field: 'gemini_key', model: 'gemini-2.5-flash', url: 'https://aistudio.google.com/app/apikey', prefix: 'gemini' },
};
let aiProv = 'claude';
const hasKey = (p) => !!S[PROVIDERS[p].field];
function renderAi() {
  aiProv = PROVIDERS[S.ai_provider] ? S.ai_provider : 'claude';
  const P = PROVIDERS[aiProv];
  $$('#aProv button').forEach((b) => b.classList.toggle('on', b.dataset.prov === aiProv));
  $('#aKeyName').textContent = P.vendor;
  $('#aKey').value = S[P.field] || '';
  $('#aKey').placeholder = `Clé ${P.vendor}`;
  $('#aKeyLink').title = P.url;
  const m = S.ai_model || '';
  $('#aModel').value = m.toLowerCase().startsWith(P.prefix) || (aiProv === 'openai' && /^(o\d|chatgpt)/.test(m)) ? m : P.model;
  $('#aModel').placeholder = P.model;
  $('#aCfgBtn').classList.toggle('need', !hasKey(aiProv));
  $('#aIn').placeholder = hasKey(aiProv) ? `Demande à ${P.name}…` : `Ajoute une clé ${P.vendor} pour discuter`;
}
$('#aProv').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-prov]');
  if (!b || b.dataset.prov === aiProv) return;
  const P = PROVIDERS[b.dataset.prov];
  await persist({ ai_provider: b.dataset.prov, ai_model: P.model });
  if (!hasKey(b.dataset.prov)) $('#aCfg').classList.remove('hidden');
});
$('#aCfgBtn').onclick = () => $('#aCfg').classList.toggle('hidden');
$('#aKeyLink').onclick = () => call('open_uri', { uri: PROVIDERS[aiProv].url }).catch(() => {});
$('#aSave').onclick = async () => {
  const P = PROVIDERS[aiProv];
  if (await persist({ [P.field]: $('#aKey').value.trim(), ai_model: $('#aModel').value.trim() || P.model })) {
    toast(hasKey(aiProv) ? `${P.name} prêt` : 'Clé retirée');
    if (hasKey(aiProv)) $('#aCfg').classList.add('hidden');
  }
};

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
  if (!hasKey(aiProv)) { $('#aCfg').classList.remove('hidden'); return toast(`Ajoute d'abord une clé ${PROVIDERS[aiProv].vendor}`, true); }
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

// segmentés à deux choix (indicateur glissant)
function seg2(el, val, attr) {
  const bs = [...el.querySelectorAll('button')];
  bs.forEach((b) => b.classList.toggle('on', b.dataset[attr] === val));
  el.classList.toggle('large', bs[1]?.dataset[attr] === val);
}

// ---------- calculatrice intégrée ----------
// Analyseur maison (aucun eval) : + − × ÷ ^ % √ π, parenthèses, multiplication implicite.
function calc(src) {
  const s = String(src).replace(/\s+/g, '').replace(/,/g, '.').replace(/[x×]/gi, '*').replace(/÷/g, '/').replace(/−/g, '-').replace(/π/g, 'p');
  let i = 0;
  const peek = () => s[i];
  const fail = () => { throw new Error(); };
  function prim() {
    const c = peek();
    if (c === '√') { i++; return Math.sqrt(post()); }
    if (c === 'p') { i++; return Math.PI; }
    if (c === '(') { i++; const v = expr(); if (i < s.length) { if (s[i] !== ')') fail(); i++; } return v; }
    const m = /^(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?/i.exec(s.slice(i));
    if (!m) fail();
    i += m[0].length;
    return parseFloat(m[0]);
  }
  function post() { let v = prim(); while (peek() === '%') { i++; v /= 100; } return v; }
  function pow() { const b = post(); if (peek() === '^') { i++; return b ** unary(); } return b; }
  function unary() { if (peek() === '-') { i++; return -unary(); } if (peek() === '+') { i++; return unary(); } return pow(); }
  function term() {
    let v = unary();
    for (;;) {
      const c = peek();
      if (c === '*' || c === '/') { i++; const r = unary(); v = c === '*' ? v * r : v / r; }
      else if (c && /[(\dp√.]/.test(c)) v *= unary();      // 2π, 3(4+1)
      else return v;
    }
  }
  function expr() { let v = term(); while (peek() === '+' || peek() === '-') { const o = s[i++]; const r = term(); v = o === '+' ? v + r : v - r; } return v; }
  if (!s) fail();
  const v = expr();
  if (i !== s.length || !isFinite(v)) fail();
  return v;
}
const fmtNum = (v) => {
  if (!isFinite(v)) return 'Erreur';
  const a = Math.abs(v);
  if (a !== 0 && (a >= 1e15 || a < 1e-9)) return v.toExponential(6).replace('.', ',');
  return Number(v.toPrecision(12)).toLocaleString('fr-FR', { maximumFractionDigits: 10 });
};
const rawNum = (v) => { const a = Math.abs(v); return a !== 0 && (a >= 1e15 || a < 1e-9) ? v.toExponential(10) : String(Number(v.toPrecision(14))).replace('.', ','); };
let cExpr = '', cJust = false, cMode = 'calc';
let cHist = store.get('rez.calcHist', []);
function calcRender() {
  const big = $('#cRes'), small = $('#cExpr');
  if (cJust) {
    small.textContent = cHist[0] ? cHist[0].e + ' =' : ' ';
    big.textContent = cHist[0]?.r ?? '0';
  } else {
    big.textContent = cExpr || '0';
    let prev = ' ';
    if (/[+−×÷^%√π(]/.test(cExpr)) { try { prev = '= ' + fmtNum(calc(cExpr)); } catch {} }
    small.textContent = prev;
  }
  const n = big.textContent.length;
  big.className = 'calc-res' + (n > 18 ? ' xs' : n > 11 ? ' sm' : '');
  big.classList.remove('err');
  big.scrollLeft = big.scrollWidth; small.scrollLeft = small.scrollWidth;   // affiche la fin de l'expression
}
function calcHistRender() {
  $('#cHistList').innerHTML = cHist.length
    ? cHist.map((h, i) => `<button class="h-row" data-h="${i}"><small>${esc(h.e)} =</small><b>${esc(h.r)}</b></button>`).join('')
    : '<p class="hint">Aucun calcul pour l\'instant.</p>';
}
function cPress(k) {
  if (k === 'clear') { cExpr = ''; cJust = false; }
  else if (k === 'back') { if (cJust) { cJust = false; cExpr = ''; } else cExpr = cExpr.slice(0, -1); }
  else if (k === '=') {
    if (!cExpr) return;
    try {
      const v = calc(cExpr);
      cHist.unshift({ e: cExpr, r: fmtNum(v), raw: rawNum(v) });
      cHist = cHist.slice(0, 40);
      store.set('rez.calcHist', cHist);
      cExpr = rawNum(v); cJust = true;
      calcHistRender();
    } catch { $('#cRes').classList.add('err'); setTimeout(() => $('#cRes').classList.remove('err'), 450); return; }
  } else {
    const op = /^[+−×÷^%]/.test(k);
    if (cJust) { if (!op) cExpr = ''; cJust = false; }
    if (/^[+×÷^]/.test(k) && /[+−×÷^]$/.test(cExpr)) cExpr = cExpr.slice(0, -1);   // remplace l'opérateur
    if (/^[+×÷^%]/.test(k) && !cExpr) return;
    if (k === ',') { const last = cExpr.match(/[\d,]*$/)[0]; if (last.includes(',')) return; if (!last) k = '0,'; }
    if (cExpr.length < 80) cExpr += k;
  }
  calcRender();
}
$('#cKeys').parentElement.addEventListener('click', (e) => { const b = e.target.closest('[data-k]'); if (b) cPress(b.dataset.k); });
$('#cHistBtn').onclick = () => { calcHistRender(); $('#cHist').classList.toggle('hidden'); };
$('#cHistClr').onclick = () => { cHist = []; store.set('rez.calcHist', cHist); calcHistRender(); };
$('#cHistList').addEventListener('click', (e) => {
  const r = e.target.closest('[data-h]');
  if (!r) return;
  cExpr = cHist[+r.dataset.h].raw || ''; cJust = false;
  $('#cHist').classList.add('hidden');
  calcRender();
});
const C_KEYS = { Enter: '=', '=': '=', Backspace: 'back', Delete: 'clear', '*': '×', x: '×', X: '×', '/': '÷', '-': '−', '+': '+', '.': ',', ',': ',', '(': '(', ')': ')', '^': '^', '%': '%', p: 'π', r: '√' };
document.addEventListener('keydown', (e) => {
  if (mode !== 'open' || view !== 'calc' || cMode !== 'calc' || e.ctrlKey || e.altKey) return;
  if (e.target.closest?.('input, textarea, select')) return;
  const k = /^\d$/.test(e.key) ? e.key : e.key === 'Escape' && (cExpr || cJust) ? 'clear' : C_KEYS[e.key];
  if (!k) return;
  e.preventDefault();
  cPress(k);
  const b = $(`#views [data-view="calc"] [data-k="${CSS.escape(k)}"]`);
  if (b) { b.classList.add('hit'); setTimeout(() => b.classList.remove('hit'), 120); }
});

// conversion d'unités
const UNITS = {
  len: ['Longueur', [['mm', 'Millimètre', 1e-3], ['cm', 'Centimètre', 1e-2], ['m', 'Mètre', 1], ['km', 'Kilomètre', 1e3], ['po', 'Pouce', 0.0254], ['pi', 'Pied', 0.3048], ['yd', 'Yard', 0.9144], ['mi', 'Mile', 1609.344]]],
  mass: ['Masse', [['mg', 'Milligramme', 1e-6], ['g', 'Gramme', 1e-3], ['kg', 'Kilogramme', 1], ['t', 'Tonne', 1e3], ['oz', 'Once', 0.028349523125], ['lb', 'Livre (lb)', 0.45359237], ['st', 'Stone', 6.35029318]]],
  temp: ['Température', [['°C', 'Celsius'], ['°F', 'Fahrenheit'], ['K', 'Kelvin']]],
  vol: ['Volume', [['mL', 'Millilitre', 1e-3], ['cL', 'Centilitre', 1e-2], ['L', 'Litre', 1], ['m³', 'Mètre cube', 1e3], ['tasse', 'Tasse US', 0.2365882365], ['fl oz', 'Once liquide US', 0.0295735295625], ['gal', 'Gallon US', 3.785411784]]],
  speed: ['Vitesse', [['m/s', 'Mètre/seconde', 1], ['km/h', 'Kilomètre/heure', 1 / 3.6], ['mph', 'Mile/heure', 0.44704], ['nd', 'Nœud', 1852 / 3600], ['Mach', 'Mach', 340.29]]],
  area: ['Surface', [['cm²', 'Centimètre carré', 1e-4], ['m²', 'Mètre carré', 1], ['a', 'Are', 100], ['ha', 'Hectare', 1e4], ['km²', 'Kilomètre carré', 1e6], ['pi²', 'Pied carré', 0.09290304], ['acre', 'Acre', 4046.8564224]]],
  data: ['Données', [['bit', 'Bit', 0.125], ['o', 'Octet', 1], ['Ko', 'Kilooctet', 1e3], ['Mo', 'Mégaoctet', 1e6], ['Go', 'Gigaoctet', 1e9], ['To', 'Téraoctet', 1e12], ['Kio', 'Kibioctet', 1024], ['Mio', 'Mébioctet', 1048576], ['Gio', 'Gibioctet', 1073741824]]],
  time: ['Temps', [['ms', 'Milliseconde', 1e-3], ['s', 'Seconde', 1], ['min', 'Minute', 60], ['h', 'Heure', 3600], ['j', 'Jour', 86400], ['sem', 'Semaine', 604800], ['an', 'Année', 31557600]]],
  energy: ['Énergie', [['J', 'Joule', 1], ['kJ', 'Kilojoule', 1e3], ['cal', 'Calorie', 4.184], ['kcal', 'Kilocalorie', 4184], ['Wh', 'Wattheure', 3600], ['kWh', 'Kilowattheure', 3.6e6]]],
};
let uState = store.get('rez.units', { cat: 'len', from: 'km', to: 'mi' });
const toBase = (cat, u, v) => cat === 'temp' ? (u === '°C' ? v : u === '°F' ? (v - 32) * 5 / 9 : v - 273.15) : v * UNITS[cat][1].find((x) => x[0] === u)[2];
const fromBase = (cat, u, v) => cat === 'temp' ? (u === '°C' ? v : u === '°F' ? v * 9 / 5 + 32 : v + 273.15) : v / UNITS[cat][1].find((x) => x[0] === u)[2];
function convFill() {
  const list = UNITS[uState.cat][1];
  if (!list.some((x) => x[0] === uState.from)) uState.from = list[0][0];
  if (!list.some((x) => x[0] === uState.to)) uState.to = list[Math.min(1, list.length - 1)][0];
  $('#uCats').innerHTML = Object.entries(UNITS).map(([k, [l]]) => `<button data-cat="${k}" class="${k === uState.cat ? 'on' : ''}">${l}</button>`).join('');
  const opts = list.map(([k, l]) => `<option value="${esc(k)}">${esc(l)} (${esc(k)})</option>`).join('');
  $('#uFrom').innerHTML = opts; $('#uTo').innerHTML = opts;
  $('#uFrom').value = uState.from; $('#uTo').value = uState.to;
  convRun();
}
function convRun() {
  store.set('rez.units', uState);
  const list = UNITS[uState.cat][1];
  let v;
  try { v = calc($('#uIn').value || '0'); } catch { $('#uOut').textContent = '—'; $('#uAll').innerHTML = ''; return; }
  const base = toBase(uState.cat, uState.from, v);
  $('#uOut').textContent = fmtNum(fromBase(uState.cat, uState.to, base));
  $('#uAll').innerHTML = list.filter((x) => x[0] !== uState.from).map(([k, l]) =>
    `<button class="u-row${k === uState.to ? ' on' : ''}" data-u="${esc(k)}"><span>${esc(l)}</span><b>${fmtNum(fromBase(uState.cat, k, base))} <small>${esc(k)}</small></b></button>`).join('');
}
$('#uCats').addEventListener('click', (e) => { const b = e.target.closest('[data-cat]'); if (b) { uState.cat = b.dataset.cat; uState.from = uState.to = ''; convFill(); } });
$('#uFrom').onchange = () => { uState.from = $('#uFrom').value; convRun(); };
$('#uTo').onchange = () => { uState.to = $('#uTo').value; convRun(); };
$('#uIn').addEventListener('input', convRun);
$('#uSwap').onclick = () => { [uState.from, uState.to] = [uState.to, uState.from]; $('#uFrom').value = uState.from; $('#uTo').value = uState.to; convRun(); };
$('#uAll').addEventListener('click', (e) => { const b = e.target.closest('[data-u]'); if (b) { uState.to = b.dataset.u; $('#uTo').value = uState.to; convRun(); } });
$('#cMode').addEventListener('click', (e) => {
  const b = e.target.closest('[data-cm]');
  if (!b) return;
  cMode = b.dataset.cm;
  seg2($('#cMode'), cMode, 'cm');
  $$('.cmode').forEach((m) => m.classList.toggle('on', m.dataset.cm === cMode));
  if (cMode === 'conv') { convFill(); setTimeout(() => $('#uIn').select(), 50); }
});
calcRender();

// ---------- Performance PC ----------
const HN = 60, HIST = {};
function hpush(k, v) { const a = (HIST[k] ||= []); a.push(v == null || !isFinite(v) ? null : v); if (a.length > HN) a.shift(); }
function spark(id, data, max) {
  const el = document.getElementById(id);
  if (!el || !data?.length) return;
  const h = el.viewBox.baseVal.height;
  const off = HN - data.length;
  let line = '', area = '', run = [];
  const flush = () => {
    if (run.length > 1) {
      const d = run.map(([x, y], j) => `${j ? 'L' : 'M'}${x.toFixed(2)} ${y.toFixed(2)}`).join('');
      line += d;
      area += `${d}L${run[run.length - 1][0].toFixed(2)} ${h}L${run[0][0].toFixed(2)} ${h}Z`;
    }
    run = [];
  };
  data.forEach((v, i) => {
    if (v == null) return flush();
    run.push([((i + off) / (HN - 1)) * 100, h - Math.min(1, Math.max(0, v / max)) * (h - 2) - 1]);
  });
  flush();
  el.innerHTML = `<path class="area" d="${area}"/><path class="line" d="${line}"/>`;
}
const autoMax = (k, min) => Math.max(min, ...(HIST[k] || []).filter((x) => x != null)) * 1.15;
const pct = (v) => (v == null ? '—' : Math.round(v) + ' %');
const gb = (v, d = 1) => (v == null ? '—' : num(v, d) + ' Go');
const upFmt = (s) => { const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60); return d ? `${d} j ${h} h` : h ? `${h} h ${String(m).padStart(2, '0')}` : `${m} min`; };
function tempEm(el, t) {
  el.textContent = t == null ? '' : Math.round(t) + '°';
  el.className = t == null ? '' : t >= 85 ? 't-hot' : t >= 70 ? 't-warm' : 't-ok';
}
let perfBusy = false, pfTab = 'ov', pfSort = 'cpu', perfLast = null;
async function refreshPerf() {
  if (perfBusy) return;
  perfBusy = true;
  try {
    const p = await invoke('perf_info');
    perfLast = p;
    hpush('cpu', p.cpu); hpush('gpu', p.gpu);
    hpush('ram', p.ram_total ? (p.ram_used / p.ram_total) * 100 : null);
    hpush('vram', p.vram_total ? (p.vram_used / p.vram_total) * 100 : null);
    hpush('disk', p.disk_act); hpush('net', (p.net_down || 0) + (p.net_up || 0));
    renderPerf(p);
  } catch {} finally { perfBusy = false; }
}
function appRows(list, key) {
  const max = Math.max(1e-9, ...list.map((a) => a[key]));
  return list.map((a) => {
    const val = key === 'cpu' ? `${num(a.cpu, 1)} %` : size(a.ram);
    const w = key === 'cpu' ? Math.min(100, a.cpu) : (a.ram / max) * 100;
    return `<div class="app-row"><span class="a-n"><b>${esc(a.name)}</b>${a.n > 1 ? `<small>×${a.n}</small>` : ''}</span><span class="a-v">${val}</span><span class="a-bar"><i style="width:${w.toFixed(1)}%"></i></span></div>`;
  }).join('') || '<p class="hint">Mesure en cours…</p>';
}
function renderPerf(p) {
  const vramPct = p.vram_total ? (p.vram_used / p.vram_total) * 100 : null;
  $('#pfCpu').textContent = pct(p.cpu);
  $('#pfCpuS').textContent = p.freq ? `${num(p.freq, 2)} GHz` : `${p.logical} threads`;
  tempEm($('#pfCpuT'), p.cpu_temp);
  $('#pfGpu').textContent = pct(p.gpu);
  $('#pfGpuS').textContent = (p.gpu_name || 'GPU').replace(/^(NVIDIA|AMD|Intel\(R\))\s*/i, '').replace(/GeForce\s*/i, '');
  tempEm($('#pfGpuT'), p.gpu_temp);
  $('#pfRam').textContent = gb(p.ram_used);
  $('#pfRamS').textContent = `sur ${num(p.ram_total, 0)} Go · ${pct((p.ram_used / p.ram_total) * 100)}`;
  $('#pfVram').textContent = p.vram_total ? gb(p.vram_used) : '—';
  $('#pfVramS').textContent = p.vram_total ? `sur ${num(p.vram_total, 0)} Go · ${pct(vramPct)}` : 'Non fournie';
  $('#pfDisk').textContent = pct(p.disk_act);
  $('#pfDiskS').textContent = `L ${rate(p.disk_r)} · É ${rate(p.disk_w)}`;
  $('#pfNet').textContent = '↓ ' + rate(p.net_down);
  $('#pfNetS').textContent = '↑ ' + rate(p.net_up);
  if (pfTab === 'ov') {
    spark('skCpu', HIST.cpu, 100); spark('skGpu', HIST.gpu, 100); spark('skRam', HIST.ram, 100);
    spark('skVram', HIST.vram, 100); spark('skDisk', HIST.disk, 100); spark('skNet', HIST.net, autoMax('net', 50e3));
    $('#pfTop3').innerHTML = appRows(p.top_cpu.slice(0, 3), 'cpu');
  } else if (pfTab === 'cpu') {
    $('#pfCpuName').textContent = p.cpu_name || 'Processeur';
    $('#pfCpu2').textContent = pct(p.cpu);
    spark('skCpu2', HIST.cpu, 100);
    $('#pfFreq').textContent = p.freq ? `${num(p.freq, 2)} GHz` : p.base ? `${num(p.base, 2)} GHz` : '—';
    $('#pfTemp').textContent = p.cpu_temp != null ? Math.round(p.cpu_temp) + ' °C' : 'Non fournie';
    $('#pfLog').textContent = p.logical;
    $('#pfCores').innerHTML = (p.cores || []).map((c, i) => `<span title="Cœur ${i} : ${Math.round(c)} %"><i style="height:${Math.max(3, c).toFixed(0)}%;opacity:${(0.45 + c / 180).toFixed(2)}"></i></span>`).join('');
    $('#pfProcs').textContent = num(p.procs);
    $('#pfThreads').textContent = num(p.threads);
    $('#pfHandles').textContent = num(p.handles);
    $('#pfUp').textContent = upFmt(p.uptime);
    $('#pfCache').textContent = gb(p.ram_cache);
    $('#pfCommit').textContent = p.commit != null ? `${num(p.commit, 1)} / ${num(p.commit_limit, 0)} Go` : '—';
  } else {
    $('#pfApps').innerHTML = appRows(pfSort === 'cpu' ? p.top_cpu : p.top_ram, pfSort);
  }
}
$('#pfTabs').addEventListener('click', (e) => {
  const b = e.target.closest('[data-pf]');
  if (!b) return;
  pfTab = b.dataset.pf;
  $$('#pfTabs button').forEach((x) => x.classList.toggle('on', x === b));
  $$('.pfsub').forEach((s) => s.classList.toggle('on', s.dataset.pf === pfTab));
  if (perfLast) renderPerf(perfLast);
});
document.addEventListener('click', (e) => { const b = e.target.closest('[data-pf-go]'); if (b) $(`#pfTabs [data-pf="${b.dataset.pfGo}"]`).click(); });
$('#pfSort').addEventListener('click', (e) => {
  const b = e.target.closest('[data-sort]');
  if (!b) return;
  pfSort = b.dataset.sort;
  seg2($('#pfSort'), pfSort, 'sort');
  if (perfLast) renderPerf(perfLast);
});

// ---------- périphériques & batteries ----------
const DEV_ICO = {
  mouse: '<svg viewBox="0 0 24 24"><rect x="6" y="3" width="12" height="18" rx="6"/><path d="M12 7v3"/></svg>',
  keyboard: '<svg viewBox="0 0 24 24"><rect x="2" y="6" width="20" height="12" rx="2.5"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/></svg>',
  headset: '<svg viewBox="0 0 24 24"><path d="M4 15v-3a8 8 0 0116 0v3"/><rect x="3" y="14" width="4" height="6" rx="1.5"/><rect x="17" y="14" width="4" height="6" rx="1.5"/></svg>',
  gamepad: '<svg viewBox="0 0 24 24"><path d="M6.5 8h11a4.5 4.5 0 014.4 5.5l-.7 3.2a2.5 2.5 0 01-4.3 1.1L15 16H9l-1.9 1.8a2.5 2.5 0 01-4.3-1.1l-.7-3.2A4.5 4.5 0 016.5 8z"/><path d="M8 11v3M6.5 12.5h3M15.5 12h.01M17.5 13.5h.01"/></svg>',
  laptop: '<svg viewBox="0 0 24 24"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>',
  speaker: '<svg viewBox="0 0 24 24"><rect x="6" y="3" width="12" height="18" rx="3"/><circle cx="12" cy="14" r="3"/><path d="M12 7h.01"/></svg>',
  pen: '<svg viewBox="0 0 24 24"><path d="M15 4l5 5L9 20H4v-5z"/></svg>',
  other: '<svg viewBox="0 0 24 24"><path d="M7 7l10 10-5 5V2l5 5L7 17"/></svg>',
};
const BOLT = '<svg class="bolt" viewBox="0 0 24 24"><path d="M13 2L4 14h7l-1 8 9-12h-7z"/></svg>';
let devices = [], devAt = 0, devBusy = false;
const lowWarned = new Set();
const batCls = (d) => (!d.online || d.pct == null ? 'na' : d.full ? 'ok' : d.charging ? 'chg' : d.pct <= 15 ? 'low' : d.pct <= 30 ? 'mid' : 'ok');
function batState(d) {
  if (!d.online) return 'Éteint ou hors de portée';
  if (d.pct == null) return 'Batterie non publiée par le fabricant';
  if (d.full) return 'Chargé';
  if (d.charging) return 'En charge';
  if (d.pct <= 15) return 'Batterie faible';
  return d.approx ? 'Estimation (tension)' : 'Sur batterie';
}
async function refreshDevices(force) {
  if (devBusy || (!force && Date.now() - devAt < 10e3)) return;
  devBusy = true;
  if (force) $('#devRefresh').classList.add('spin');
  try { devices = await invoke('devices_list', { force: !!force }); devAt = Date.now(); renderDevices(); }
  catch {} finally { devBusy = false; $('#devRefresh').classList.remove('spin'); }
}
function renderDevices() {
  $('#devList').innerHTML = devices.length ? devices.map((d) => {
    const c = batCls(d);
    const p = d.pct ?? 0;
    const bat = d.pct != null && d.online
      ? `<span class="bat"><span class="bat-body"><i style="width:${p}%"></i></span>${d.charging || d.full ? BOLT : ''}<b>${d.approx ? '≈' : ''}${p}<small>%</small></b></span>`
      : `<span class="bat na"><span class="bat-body"><i></i></span><b>—</b></span>`;
    return `<div class="dev ${c}"><span class="dev-ico">${DEV_ICO[d.kind] || DEV_ICO.other}</span>
      <span class="dev-t"><b>${esc(d.name)}</b><small><span class="src">${esc(d.src)}</span>${batState(d)}</small></span>${bat}
      ${d.pct != null && d.online ? `<span class="dev-bar"><i style="width:${p}%"></i></span>` : ''}</div>`;
  }).join('') : `<div class="card empty"><b>Aucun périphérique sans fil détecté</b><small>Allume ton casque, ta manette ou ta souris Bluetooth puis actualise.</small></div>`;
  const withBat = devices.filter((d) => d.pct != null && d.online);
  $('#homeDev').innerHTML = withBat.length ? withBat.slice(0, 5).map((d) =>
    `<span class="dchip ${batCls(d)}" title="${esc(d.name)} · ${batState(d)}">${DEV_ICO[d.kind] || DEV_ICO.other}<b>${d.pct}%</b>${d.charging ? BOLT : ''}</span>`).join('')
    : `<span class="w-empty">${devices.length ? devices.length + ' périphérique(s) · batterie non publiée' : 'Aucune batterie détectée'}</span>`;
  let low = false;
  for (const d of withBat) {
    const isLow = d.pct <= 15 && !d.charging;
    if (isLow) {
      low = true;
      if (!lowWarned.has(d.id)) { lowWarned.add(d.id); toast(`Batterie faible : ${d.name} (${d.pct} %)`, true); }
    } else if (d.charging || d.pct > 25) lowWarned.delete(d.id);
  }
  $('#devBadge').classList.toggle('hidden', !low);
}
$('#devRefresh').onclick = () => refreshDevices(true);

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
const LOCK_SHUT = '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 018 0v3"/>';
const LOCK_OPEN = '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 017.6-1.8"/>';
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
  const th = S.theme || 'violet';
  document.documentElement.dataset.theme = th;
  $$('#pTheme button').forEach((b) => b.classList.toggle('on', b.dataset.theme === th));
  const free = S.locked === false;
  document.body.classList.toggle('free', free);
  $('#btnLock').classList.toggle('on', free);
  $('#btnLock').title = free ? 'Verrouiller l\'île à cet endroit' : 'Déverrouiller pour déplacer l\'île';
  $('#btnLock svg').innerHTML = free ? LOCK_OPEN : LOCK_SHUT;
  $('#pUnlock').checked = free;
  renderDock();
}
const DOCKS = { top: 'en haut', left: 'sur le côté gauche', right: 'sur le côté droit' };
function renderDock() {
  const d = S.dock || 'top';
  const seg = $('#pDock');
  const i = ['top', 'left', 'right'].indexOf(d);
  seg.classList.toggle('p1', i === 1);
  seg.classList.toggle('p2', i === 2);
  seg.classList.toggle('none', i < 0);
  $$('#pDock button').forEach((b) => b.classList.toggle('on', b.dataset.dock === d));
  const where = DOCKS[d] || 'à l\'endroit où tu l\'as posée';
  $('#pUnlockHint').textContent = S.locked === false
    ? 'Glisse la pilule ou l\'en-tête où tu veux (multi-écrans). 🔒 la fige à cet endroit exact.'
    : `Île verrouillée ${where}. Les panneaux s'ouvrent vers l'intérieur de l'écran.`;
}
$('#pDock').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-dock]');
  if (!b || b.dataset.dock === S.dock) return;
  if (await persist({ dock: b.dataset.dock })) toast(`Île ${DOCKS[b.dataset.dock]}`);
});
function fillSettings() {
  $('#pShortcut').value = S.shortcut;
  $('#pHide').value = S.hide_shortcut || '';
  $('#pCity').value = S.city;
  $('#pDiscord').value = S.discord_client_id;
  $('#pDAuto').checked = S.discord_auto;
  $('#pFn').value = S.fortnite_key;
  $('#pHenrik').value = S.henrik_key;
  $('#pApex').value = S.apex_key;
  $('#pDSecret').value = S.discord_secret;
  $('#pAuto').checked = S.autostart;
  $('#pBlur').checked = S.hide_on_blur;
  $('#pPill').checked = S.pill_mode;
  $('#dDetails').value = S.discord_details;
  $('#dState').value = S.discord_state;
  $('#pAutoUpd').checked = S.auto_update !== false;
  renderCoffee();
  applyWidgets();
  applyModules();
  applyLook();
  renderPins();
  renderAi();
  renderVoice();
}
async function persist(extra = {}) {
  try {
    S = await call('save_settings', { s: { ...S, ...extra } });
    fillSettings();
    return true;
  } catch { return false; }
}
async function setLocked(locked) {
  if (await persist({ locked })) toast(locked ? 'Position verrouillée ici' : 'Déverrouillée : glisse l\'île où tu veux');
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
$('#pTheme').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-theme]');
  if (!b || b.dataset.theme === S.theme) return;
  S.theme = b.dataset.theme;
  applyLook();
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
    discord_secret: $('#pDSecret').value.trim(),
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
    $('#pUpdState').textContent = r.available ? `v${r.latest} disponible` : 'À jour · ' + new Date().toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
    if (r.available) {
      $('#updVer').textContent = 'v' + r.latest;
      $('#updNotes').textContent = r.notes || 'Nouvelle version disponible';
      $('#updCard').classList.remove('hidden');
      $('#setBadge').classList.remove('hidden');
      $('#pillDot').classList.add('upd');
      pendingUpd = r;
      if (manual) { toast(`v${r.latest} disponible`); go('home'); }
      maybeAutoInstall();
    } else if (manual) toast(`REZ est à jour (v${r.current})`);
  } catch (e) { $('#pUpdState').textContent = 'Vérification impossible'; if (manual) toast(String(e), true); }
}
$('#pCheck').onclick = () => checkUpdate(true);
$('#updBtn').onclick = async () => {
  const b = $('#updBtn');
  b.disabled = true; b.textContent = '…';
  try { await invoke('install_update'); }
  catch (e) { toast(String(e), true); b.disabled = false; b.textContent = 'Réessayer'; }
};

// ---------- accueil : grille de widgets modulable ----------
const WIDGETS = {
  timer: 'Minuteur', media: 'Lecteur', clip: 'Presse-papiers', notes: 'Notes', coffee: 'Anti-veille',
  weather: 'Météo', clock: 'Horloge', focus: 'Mode Focus', sys: 'Système', dev: 'Batteries',
  dl: 'Téléchargements', discord: 'Discord', game: 'En jeu', pins: 'Dossiers épinglés',
  q_calc: 'Calculatrice', q_shot: 'Capture', q_perf: 'Performance PC', q_dev: 'Périphériques', q_files: 'Fichiers',
  q_ai: 'Chat IA', q_mix: 'Mélangeur audio', q_stats: 'Stats de jeu', q_set: 'Réglages',
};
const W_DEFAULT = ['timer', 'media', 'clip', 'q_calc', 'q_shot', 'q_perf', 'q_set', 'notes', 'coffee', 'weather', 'focus', 'sys', 'dev', 'dl', 'discord', 'game'];
const W_SIZE = { s11: '1×1', s21: '2×1', s22: '2×2', s41: '4×1' };
const wEl = (id) => document.querySelector(`.w[data-w="${id}"]`);
const wOrder = () => (Array.isArray(S.widgets) ? S.widgets : W_DEFAULT).filter((id, i, a) => WIDGETS[id] && wEl(id) && a.indexOf(id) === i);
const wOn = (id) => wOrder().includes(id);
const wGrid = $('#wgrid');
for (const el of $$('.w')) {
  const x = document.createElement('button');
  x.className = 'w-x'; x.title = 'Retirer'; x.textContent = '×';
  el.appendChild(x);
}
function applyWidgets() {
  const on = wOrder();
  for (const id of on) wGrid.appendChild(wEl(id));
  for (const el of [...wGrid.children]) if (!on.includes(el.dataset.w)) $('#wpool').appendChild(el);
  $('#wAddList').innerHTML = Object.keys(WIDGETS).filter((id) => !on.includes(id))
    .map((id) => `<button data-add="${id}">+ ${esc(WIDGETS[id])}</button>`).join('') || '<small class="hint">Tous les widgets sont affichés.</small>';
  const rows = [...on, ...Object.keys(WIDGETS).filter((id) => !on.includes(id))];
  $('#pWidgets').innerHTML = rows.map((id) => {
    const act = on.includes(id);
    const sz = W_SIZE[[...wEl(id).classList].find((c) => W_SIZE[c])] || '';
    return `<div class="wrow${act ? '' : ' off'}" data-wid="${id}"><span class="wr-n">${esc(WIDGETS[id])}</span><small>${sz}</small>
      <button class="icon-btn" data-mv="-1" title="Monter"${act ? '' : ' disabled'}><svg viewBox="0 0 24 24"><path d="M6 15l6-6 6 6"/></svg></button>
      <button class="icon-btn" data-mv="1" title="Descendre"${act ? '' : ' disabled'}><svg viewBox="0 0 24 24"><path d="M6 9l6 6 6-6"/></svg></button>
      <label class="tog"><input type="checkbox" data-wtog="${id}"${act ? ' checked' : ''}><span></span></label></div>`;
  }).join('');
  schedule();
}
async function saveWidgets(list) {
  S.widgets = list;
  applyWidgets();
  await persist({ widgets: list });
}
let editing = false;
function setEditing(on) {
  editing = on;
  document.body.classList.toggle('editing', on);
  $('#btnEditDone').classList.toggle('hidden', !on);
  $('#btnEdit').classList.toggle('hidden', on);
  $('#wAdd').classList.toggle('hidden', !on);
  $('#wHint').textContent = on ? 'Glisse pour déplacer · × pour retirer' : '';
  if (on && view !== 'home') go('home');
}
$('#btnEdit').onclick = () => setEditing(true);
$('#btnEditDone').onclick = () => { setEditing(false); toast('Disposition enregistrée'); };
$('#pWEdit').onclick = () => { setEditing(true); };
$('#pWReset').onclick = () => saveWidgets([...W_DEFAULT]).then(() => toast('Disposition par défaut'));
$('#wAddList').addEventListener('click', (e) => {
  const b = e.target.closest('[data-add]');
  if (b) saveWidgets([...wOrder(), b.dataset.add]);
});
$('#pWidgets').addEventListener('change', (e) => {
  const t = e.target.closest('[data-wtog]');
  if (!t) return;
  const id = t.dataset.wtog;
  saveWidgets(t.checked ? [...wOrder(), id] : wOrder().filter((x) => x !== id));
});
$('#pWidgets').addEventListener('click', (e) => {
  const b = e.target.closest('[data-mv]');
  if (!b) return;
  const id = b.closest('[data-wid]').dataset.wid;
  const l = wOrder();
  const i = l.indexOf(id), j = i + +b.dataset.mv;
  if (i < 0 || j < 0 || j >= l.length) return;
  [l[i], l[j]] = [l[j], l[i]];
  saveWidgets(l);
});
// en mode édition, aucun clic n'ouvre de page
wGrid.addEventListener('click', (e) => { if (editing) { e.stopPropagation(); e.preventDefault(); } }, true);
wGrid.addEventListener('pointerover', (e) => {
  if (editing) return;
  const w = e.target.closest('.w');
  $('#wHint').textContent = w ? WIDGETS[w.dataset.w] || '' : '';
});
wGrid.addEventListener('pointerleave', () => { if (!editing) $('#wHint').textContent = ''; });

// glisser-déposer des tuiles (pointeur) + animation FLIP des voisines
let wd = null;
function flip(change) {
  const els = [...wGrid.children].filter((x) => x !== wd?.el);
  const before = new Map(els.map((x) => [x, x.getBoundingClientRect()]));
  change();
  for (const x of els) {
    const a = before.get(x), b = x.getBoundingClientRect();
    const dx = a.left - b.left, dy = a.top - b.top;
    if (!dx && !dy) continue;
    x.style.transition = 'none';
    x.style.translate = `${dx}px ${dy}px`;
    x.getBoundingClientRect();
    x.style.transition = '';
    x.style.translate = '';
  }
}
function wdPlace(cx, cy) {
  const el = wd.el;
  el.style.translate = '';
  const r = el.getBoundingClientRect();
  el.style.translate = `${cx - wd.ox - r.left}px ${cy - wd.oy - r.top}px`;
}
wGrid.addEventListener('pointerdown', (e) => {
  if (!editing || e.button !== 0) return;
  const x = e.target.closest('.w-x');
  if (x) { e.preventDefault(); saveWidgets(wOrder().filter((id) => id !== x.closest('.w').dataset.w)); return; }
  const el = e.target.closest('.w');
  if (!el) return;
  e.preventDefault();
  const r = el.getBoundingClientRect();
  wd = { el, ox: e.clientX - r.left, oy: e.clientY - r.top, sx: e.clientX, sy: e.clientY, moved: false, cx: e.clientX, cy: e.clientY };
  el.setPointerCapture(e.pointerId);
});
document.addEventListener('pointermove', (e) => {
  if (!wd) return;
  wd.cx = e.clientX; wd.cy = e.clientY;
  if (!wd.moved) {
    if (Math.hypot(e.clientX - wd.sx, e.clientY - wd.sy) < 5) return;
    wd.moved = true;
    wd.el.classList.add('drag');
    wGrid.classList.add('dragging');
    requestAnimationFrame(wdScroll);
  }
  const t = document.elementFromPoint(e.clientX, e.clientY)?.closest('#wgrid > .w');
  if (t && t !== wd.el) {
    const items = [...wGrid.children];
    const from = items.indexOf(wd.el), to = items.indexOf(t);
    flip(() => wGrid.insertBefore(wd.el, from < to ? t.nextSibling : t));
  }
  wdPlace(e.clientX, e.clientY);
});
// défilement automatique quand on approche du haut / bas
function wdScroll() {
  if (!wd?.moved) return;
  const m = $('#views'), r = m.getBoundingClientRect();
  const d = wd.cy < r.top + 36 ? -7 : wd.cy > r.bottom - 36 ? 7 : 0;
  if (d) { m.scrollTop += d; wdPlace(wd.cx, wd.cy); }
  requestAnimationFrame(wdScroll);
}
function wdEnd() {
  if (!wd) return;
  const { el, moved } = wd;
  wd = null;
  if (!moved) return;
  el.classList.remove('drag');
  wGrid.classList.remove('dragging');
  el.style.translate = '';
  const list = [...wGrid.children].map((x) => x.dataset.w);
  if (list.join() !== wOrder().join()) { S.widgets = list; persist({ widgets: list }); }
}
document.addEventListener('pointerup', wdEnd);
document.addEventListener('pointercancel', wdEnd);

// ---------- modules du dock ----------
const MODULES = { media: 'Multimédia', discord: 'Discord', stats: 'Stats de jeu', perf: 'Performance PC', devices: 'Batteries', ai: 'Chat IA', files: 'Fichiers' };
const modOff = (m) => (S.modules_off || []).includes(m);
const devOff = () => modOff('devices') && !wOn('dev');
function applyModules() {
  for (const b of $$('.dock [data-go]')) b.classList.toggle('hidden', modOff(b.dataset.go));
  $('#pModules').innerHTML = Object.entries(MODULES).map(([k, n]) =>
    `<label class="tog"><input type="checkbox" data-mod="${k}"${modOff(k) ? '' : ' checked'}><span></span>${n}</label>`).join('');
  if (modOff(view)) go('home');
}
$('#pModules').addEventListener('change', (e) => {
  const t = e.target.closest('[data-mod]');
  if (!t) return;
  const off = new Set(S.modules_off || []);
  t.checked ? off.delete(t.dataset.mod) : off.add(t.dataset.mod);
  persist({ modules_off: [...off] }).then(() => schedule());
});

// ---------- minuteur ----------
let tm = store.get('rez.timer', null);
if (!tm || !(tm.total > 0)) tm = { total: 1500, left: 1500, end: 0 };
let tmT = null, audio = null;
const tmLeft = () => (tm.end ? Math.max(0, (tm.end - Date.now()) / 1000) : tm.left);
function renderTimer() {
  const left = tmLeft();
  $('#tmVal').textContent = mmss(Math.ceil(left));
  $('#tmArc').style.strokeDashoffset = 106.8 * (1 - Math.min(1, left / Math.max(1, tm.total)));
  $('#tmPlay svg').innerHTML = tm.end ? ICON_PAUSE : ICON_PLAY;
  wEl('timer').classList.toggle('run', !!tm.end);
}
function tmSave() { store.set('rez.timer', tm); clearTimeout(tmT); if (tm.end) tmT = setTimeout(tmDone, Math.max(0, tm.end - Date.now()) + 30); renderTimer(); renderPill(); }
function chime() {
  try {
    audio ||= new AudioContext();
    audio.resume();
    const t0 = audio.currentTime;
    [880, 1175, 1568, 1175, 1568].forEach((f, i) => {
      const o = audio.createOscillator(), g = audio.createGain();
      const t = t0 + i * 0.16;
      o.type = 'sine'; o.frequency.value = f;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.22, t + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, t + 0.45);
      o.connect(g).connect(audio.destination);
      o.start(t); o.stop(t + 0.5);
    });
  } catch {}
}
function tmDone() {
  if (!tm.end) return;
  if (tm.end - Date.now() > 300) return tmSave();
  tm.end = 0; tm.left = tm.total;
  tmSave(); schedule();
  chime();
  toast('⏱ Minuteur terminé');
  if (mode !== 'open') expand();
}
$('#tmPlay').onclick = () => {
  try { audio ||= new AudioContext(); } catch {}
  if (tm.end) { tm.left = tmLeft(); tm.end = 0; }
  else { if (tm.left < 1) tm.left = tm.total; tm.end = Date.now() + tm.left * 1000; }
  tmSave(); schedule();
};
function tmAdj(sign) {
  const cur = tmLeft();
  const step = (sign > 0 ? cur >= 600 : cur > 600) ? 300 : 60;
  if (tm.end) {
    tm.end = Math.max(Date.now() + 5000, tm.end + sign * step * 1000);
    tm.total = Math.max(tm.total, tmLeft());
  } else {
    tm.left = tm.total = Math.min(99 * 60, Math.max(60, Math.round(cur / 60) * 60 + sign * step));
  }
  tmSave();
}
$('#tmMinus').onclick = () => tmAdj(-1);
$('#tmPlus').onclick = () => tmAdj(1);
$('#tmReset').onclick = () => { tm.end = 0; tm.left = tm.total; tmSave(); schedule(); };

// ---------- presse-papiers (historique en mémoire, côté Rust) ----------
let clips = [];
function renderClips() {
  $('#clipList').innerHTML = clips.length ? clips.slice(0, 15).map((c, i) =>
    `<div class="clip" data-ci="${i}" title="${esc(c.text.slice(0, 400))}"><span>${esc(c.text.replace(/\s+/g, ' ').trim().slice(0, 140))}</span><i data-cx="${i}" title="Supprimer">×</i></div>`).join('')
    : '<p class="w-empty">Copie quelque chose</p>';
}
listen('rez://clip', (e) => { clips = e.payload || []; renderClips(); });
$('#clipList').addEventListener('click', async (e) => {
  const x = e.target.closest('[data-cx]');
  if (x) { e.stopPropagation(); const i = +x.dataset.cx; await invoke('clip_clear', { index: i }).catch(() => {}); clips.splice(i, 1); renderClips(); return; }
  const c = e.target.closest('[data-ci]');
  if (!c) return;
  try { await call('clip_copy', { text: clips[+c.dataset.ci].text }); c.classList.add('ok'); toast('Copié dans le presse-papiers'); } catch {}
});
$('#clipClr').onclick = async () => { await invoke('clip_clear', { index: null }).catch(() => {}); clips = []; renderClips(); };

// ---------- anti-veille ----------
function renderCoffee() {
  const on = !!S.awake;
  $('#cfOn').checked = on;
  $('#cfTxt').textContent = on ? 'Toujours allumé' : 'Désactivé';
  wEl('coffee').classList.toggle('on', on);
}
$('#cfOn').onchange = async () => {
  S.awake = $('#cfOn').checked;
  renderCoffee();
  await invoke('keep_awake', { on: S.awake }).catch(() => {});
  toast(S.awake ? 'Anti-veille : l\'écran reste allumé' : 'Anti-veille désactivé');
};

// ---------- mises à jour automatiques ----------
let pendingUpd = null, autoBusy = false;
function maybeAutoInstall() {
  if (!pendingUpd || !S.auto_update || autoBusy || mode === 'open' || live.game) return;
  autoBusy = true;
  invoke('install_update').catch(() => { autoBusy = false; });
}
$('#pAutoUpd').onchange = () => persist({ auto_update: $('#pAutoUpd').checked }).then(maybeAutoInstall);
setInterval(() => checkUpdate(), 6 * 3600e3);

// ---------- init ----------
(async () => {
  try { S = await invoke('get_settings'); fillSettings(); } catch {}
  $('#pVer').textContent = $('#pVer2').textContent = 'v' + (await invoke('app_version').catch(() => '?'));
  clips = (await invoke('clip_list').catch(() => null)) || [];
  renderClips();
  if (tm.end && tm.end < Date.now()) { tm.end = 0; tm.left = tm.total; }
  tmSave();
  selectGame(curGame);
  onLive(await invoke('live_state').catch(() => null));
  onVoice(await invoke('voice_state').catch(() => null));

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
