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
  compact: { pill: [200, 34], open: [340, 336] },
  large:   { pill: [236, 40], open: [400, 396] },
};
const ANIM = 440;

let S = {};                 // paramètres (clés masquées)
let mode = 'pill';          // 'pill' | 'open'
let winVisible = false;
let winW = 200, winH = 34;  // taille actuelle de la fenêtre
let animId = 0, lastCollapse = 0;
let view = 'home';
let clockTimer = null, mediaTimer = null;
let lastWeather = 0, weatherTxt = '';
let media = null;
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
  // fenêtre assez grande pour l'animation, puis ajustée à la taille finale
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
listen('rez://blur', () => { if (S.hide_on_blur && mode === 'open') collapse(); });
listen('rez://visible', (e) => { winVisible = !!e.payload; schedule(); });

$('#pillFace').onclick = expand;
$('#btnCollapse').onclick = collapse;
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && document.activeElement?.id !== 'pShortcut') collapse();
});

// ---------- navigation ----------
function go(v) {
  view = v;
  $$('.view').forEach((el) => el.classList.toggle('active', el.dataset.view === v));
  $$('.dock button').forEach((b) => b.classList.toggle('active', b.dataset.go === v));
  onView();
  schedule();
}
document.addEventListener('click', (e) => {
  const b = e.target.closest('[data-go]');
  if (b) go(b.dataset.go);
});
function onView() {
  if (mode !== 'open') return;
  if (view === 'home' && Date.now() - lastWeather > 10 * 60e3) loadWeather();
  if (view === 'discord') refreshDiscord();
  if (view === 'ai') setTimeout(() => $('#aIn').focus(), 60);
}

// ---------- timers (tout est en pause quand REZ est masqué) ----------
function schedule() {
  clearInterval(clockTimer); clockTimer = null;
  clearInterval(mediaTimer); mediaTimer = null;
  if (!winVisible) return;
  tick();
  clockTimer = setInterval(tick, mode === 'open' ? 1000 : 5000);
  const fast = mode === 'open' && (view === 'home' || view === 'media');
  refreshMedia();
  mediaTimer = setInterval(refreshMedia, fast ? 1500 : 5000);
}

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
}

// ---------- météo ----------
const WMO = [
  [[0], '☀️', 'Dégagé'], [[1], '🌤️', 'Peu nuageux'], [[2], '⛅', 'Nuageux'], [[3], '☁️', 'Couvert'],
  [[45, 48], '🌫️', 'Brouillard'], [[51, 53, 55, 56, 57], '🌦️', 'Bruine'], [[61, 63, 65, 66, 67], '🌧️', 'Pluie'],
  [[71, 73, 75, 77], '❄️', 'Neige'], [[80, 81, 82], '🌧️', 'Averses'], [[85, 86], '🌨️', 'Neige'], [[95, 96, 99], '⛈️', 'Orage'],
];
async function loadWeather() {
  try {
    const w = await invoke('weather');
    lastWeather = Date.now();
    const m = WMO.find(([c]) => c.includes(w.code)) || [[], '🌡️', '—'];
    const ico = (w.code <= 1 && !w.is_day) ? '🌙' : m[1];
    $('#wIco').textContent = ico;
    $('#wTemp').textContent = `${Math.round(w.temp)}°`;
    $('#wDesc').textContent = `${m[2]} · ${w.city}`;
    $('#wSide').textContent = `↑ ${Math.round(w.max)}°  ↓ ${Math.round(w.min)}°  ·  Ressenti ${Math.round(w.feels)}°  ·  Vent ${Math.round(w.wind)} km/h`;
    weatherTxt = `${ico} ${Math.round(w.temp)}°`;
    renderPill();
  } catch {
    $('#wDesc').textContent = 'Météo indisponible';
  }
}

// ---------- multimédia ----------
const ICON_PLAY = '<path d="M7 4l13 8-13 8z"/>';
const ICON_PAUSE = '<path d="M7 4h3v16H7zM14 4h3v16h-3z"/>';
const ICON_VOL = '<path d="M11 5L6 9H2v6h4l5 4V5zM15.5 8.5a5 5 0 010 7M19 5a10 10 0 010 14"/>';
const ICON_MUTE = '<path d="M11 5L6 9H2v6h4l5 4V5zM22 9l-6 6M16 9l6 6"/>';
let volDrag = false;

function appName(id) {
  if (!id) return '';
  const s = id.toLowerCase();
  const known = { spotify: 'Spotify', chrome: 'Chrome', msedge: 'Edge', firefox: 'Firefox', opera: 'Opera', brave: 'Brave', vlc: 'VLC', deezer: 'Deezer', zune: 'Lecteur Windows', applemusic: 'Apple Music', discord: 'Discord' };
  for (const k in known) if (s.includes(k)) return known[k];
  return id.split(/[!\\.]/)[0];
}
function setRange(el, v) { el.value = v; el.style.setProperty('--p', v + '%'); }
function renderPill() {
  const playing = media?.playing && media?.has_session;
  $('#pillBars').classList.toggle('hidden', !playing);
  $('#pillInfo').textContent = playing ? (media.title || appName(media.app)) : weatherTxt;
}
async function refreshMedia() {
  let m;
  try { m = await invoke('media_info'); } catch { return; }
  media = m;
  renderPill();
  if (mode !== 'open') return;
  const title = m.has_session ? (m.title || 'Titre inconnu') : 'Aucune lecture';
  const artist = m.has_session ? (m.artist || appName(m.app) || '—') : 'Lance Spotify, YouTube…';
  const letter = (m.title || '♪').trim().charAt(0).toUpperCase() || '♪';
  const ico = m.playing ? ICON_PAUSE : ICON_PLAY;
  $('#mTitle').textContent = $('#npTitle').textContent = title;
  $('#mArtist').textContent = $('#npArtist').textContent = artist;
  $('#mArt').textContent = $('#npArt').textContent = letter;
  $('#mArt').classList.toggle('playing', m.playing);
  $('#mApp').textContent = appName(m.app);
  $('#mPlayIco').innerHTML = $('#npPlayIco').innerHTML = ico;
  $('#mMuteIco').innerHTML = m.muted ? ICON_MUTE : ICON_VOL;
  if (!volDrag) {
    const v = Math.round(m.volume * 100);
    setRange($('#mVol'), v);
    $('#mVolVal').textContent = v;
  }
}
async function mediaAct(a) { try { await call('media_action', { action: a }); setTimeout(refreshMedia, 250); } catch {} }
$('#mPrev').onclick = () => mediaAct('prev');
$('#mPlay').onclick = () => mediaAct('toggle');
$('#npPlay').onclick = () => mediaAct('toggle');
$('#mNext').onclick = () => mediaAct('next');
$('#mMute').onclick = async () => { try { await call('media_mute', { muted: !media?.muted }); refreshMedia(); } catch {} };
const vol = $('#mVol');
let volT;
vol.addEventListener('input', () => {
  volDrag = true;
  setRange(vol, vol.value);
  $('#mVolVal').textContent = vol.value;
  clearTimeout(volT);
  volT = setTimeout(() => invoke('media_volume', { value: vol.value / 100 }).catch(() => {}), 40);
});
vol.addEventListener('change', () => { volDrag = false; });

// ---------- discord ----------
async function refreshDiscord() {
  const on = await invoke('discord_status').catch(() => false);
  const st = $('#dStatus');
  st.textContent = on ? 'Présence active' : 'Déconnecté';
  st.classList.toggle('on', on);
}
$('#dSet').onclick = async () => {
  const b = $('#dSet'); b.disabled = true;
  try {
    await call('discord_set', { details: $('#dDetails').value.trim() || 'Sur REZ', state: $('#dState').value.trim() || 'Dynamic Island' });
    toast('Rich Presence activée');
  } catch {} finally { b.disabled = false; refreshDiscord(); }
};
$('#dClear').onclick = async () => { try { await call('discord_clear'); toast('Présence effacée'); } catch {} refreshDiscord(); };
$('#dOpen').onclick = () => call('open_uri', { uri: 'discord://-/channels/@me' }).catch(() => {});

// ---------- stats ----------
const TRACKERS = {
  r6: (n) => `https://r6.tracker.network/r6siege/profile/ubi/${encodeURIComponent(n)}/overview`,
  valorant: (n) => `https://tracker.gg/valorant/profile/riot/${encodeURIComponent(n)}/overview`,
  apex: (n) => `https://apex.tracker.gg/apex/profile/origin/${encodeURIComponent(n)}/overview`,
  rl: (n) => `https://rocketleague.tracker.network/rocket-league/profile/epic/${encodeURIComponent(n)}/overview`,
};
const num = (v, d = 0) => (v == null ? '—' : Number(v).toLocaleString('fr-FR', { maximumFractionDigits: d }));
$('#sGame').onchange = () => {
  const fn = $('#sGame').value === 'fortnite';
  $('#sPlat').classList.toggle('hidden', !fn);
  $('#sWin').classList.toggle('hidden', !fn);
};
$('#sName').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('#sGo').click(); });
$('#sGo').onclick = async () => {
  const game = $('#sGame').value, name = $('#sName').value.trim(), out = $('#sOut');
  if (!name) return toast('Entre un pseudo', true);
  store.set('rez.stats', { game, name });
  if (game !== 'fortnite') {
    await call('open_uri', { uri: TRACKERS[game](name) }).catch(() => {});
    out.innerHTML = `<p class="hint">Profil ouvert dans le navigateur (tracker public).</p>`;
    return;
  }
  const b = $('#sGo'); b.disabled = true;
  out.innerHTML = `<p class="hint">Chargement…</p>`;
  try {
    const r = await invoke('fortnite_stats', { name, platform: $('#sPlat').value, window: $('#sWin').value });
    const o = r.overall || {};
    const box = (l, v) => `<div class="sbox"><small>${l}</small><div>${v}</div></div>`;
    out.innerHTML = `<div class="card">
      <div class="stat-head"><b>${esc(r.name)}</b><span>Passe niv. ${num(r.level)}</span></div>
      <div class="sgrid">
        ${box('Victoires', num(o.wins))}${box('Win %', num(o.winRate, 1))}${box('K/D', num(o.kd, 2))}
        ${box('Kills', num(o.kills))}${box('Parties', num(o.matches))}${box('Top 10', num(o.top10))}
      </div></div>`;
  } catch (e) {
    out.innerHTML = `<div class="card err">${esc(e)}</div>`;
  } finally { b.disabled = false; }
};

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

// ---------- fichiers ----------
const FOLDER = '<svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/></svg>';
function renderPins() {
  $('#fGrid').innerHTML = (S.pins || []).map((p, i) => `
    <button class="fitem" data-i="${i}" title="${esc(p.path)}">
      <span class="fi">${FOLDER}</span>
      <span class="fn"><b>${esc(p.name)}</b><small>${esc(p.path.startsWith('shell:') ? 'Système' : p.path)}</small></span>
      <span class="rm" data-rm="${i}" title="Retirer">✕</span>
    </button>`).join('');
}
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
function applySize() {
  const large = sizeKey() === 'large';
  document.documentElement.classList.toggle('large', large);
  $('#pSize').classList.toggle('large', large);
  $$('#pSize button').forEach((b) => b.classList.toggle('on', b.dataset.size === sizeKey()));
}
function fillSettings() {
  $('#pShortcut').value = S.shortcut;
  $('#pCity').value = S.city;
  $('#pDiscord').value = S.discord_client_id;
  $('#pFn').value = S.fortnite_key;
  $('#pAi').value = S.ai_key;
  $('#pModel').value = S.ai_model;
  $('#pAuto').checked = S.autostart;
  $('#pBlur').checked = S.hide_on_blur;
  $('#pPill').checked = S.pill_mode;
  $('#dDetails').value = S.discord_details;
  $('#dState').value = S.discord_state;
  applySize();
  renderPins();
}
async function persist(extra = {}) {
  try {
    S = await call('save_settings', { s: { ...S, ...extra } });
    fillSettings();
    return true;
  } catch { return false; }
}
$('#pSize').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-size]');
  if (!b || b.dataset.size === sizeKey()) return;
  S.size = b.dataset.size;
  applySize();
  render(mode);           // transition fluide vers la nouvelle taille
  await persist();
});
$('#pShortcut').addEventListener('keydown', (e) => {
  e.preventDefault();
  if (/^(Control|Shift|Alt|Meta|OS)/.test(e.code)) return;
  const mods = [];
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');
  const key = e.code.replace(/^Key/, '').replace(/^Digit/, '');
  if (!mods.length && !/^F\d{1,2}$/.test(key)) return toast('Ajoute Ctrl, Alt ou Shift', true);
  $('#pShortcut').value = [...mods, key].join('+');
});
$('#pSave').onclick = async () => {
  const cityChanged = $('#pCity').value.trim() !== S.city;
  const ok = await persist({
    shortcut: $('#pShortcut').value.trim(),
    city: $('#pCity').value.trim() || 'Paris',
    discord_client_id: $('#pDiscord').value.trim(),
    fortnite_key: $('#pFn').value.trim(),
    ai_key: $('#pAi').value.trim(),
    ai_model: $('#pModel').value.trim() || 'claude-opus-5-5',
    autostart: $('#pAuto').checked,
    hide_on_blur: $('#pBlur').checked,
    pill_mode: $('#pPill').checked,
  });
  if (ok) {
    toast(`Enregistré · ${S.shortcut}`);
    if (cityChanged) { lastWeather = 0; loadWeather(); }
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
      if (manual) { toast(`v${r.latest} disponible`); go('home'); }
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
  const last = store.get('rez.stats', null);
  if (last) { $('#sGame').value = last.game; $('#sName').value = last.name; $('#sGame').onchange(); }

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
