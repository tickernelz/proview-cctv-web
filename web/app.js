const el = (id) => document.getElementById(id);
const enc = encodeURIComponent;

const PREFS_KEY = 'pvw_prefs_v2';
const LIVE_WINDOW_SECONDS = 12;
const LIVE_TARGET_LAG_SECONDS = 1.0;
const SECTIONS = ['playback', 'camera', 'device', 'connection', 'config', 'server'];
const POPOVER = typeof HTMLElement.prototype.showPopover === 'function';
const SIGNIN_LEDE = 'The same PROVIEW account your phone uses. The camera password stays on this machine.';
const SIGNIN_STAGE = 'The picture appears here as soon as a camera answers.';

document.documentElement.dataset.popover = POPOVER ? 'native' : 'fallback';

const state = {
  screen: 'boot',
  view: 'live',
  section: 'playback',
  signedIn: false,
  account: null,
  devices: [],
  cameras: [],
  selectedId: null,
  channel: 1,
  lens: null,
  lensSupport: 'unknown',
  lensView: 'combined',
  control: { rows: [], onvif: false, dvrip: false, error: null, checkedFor: null },
  players: new Map(),
  ticket: null,
  mode: null,
  recorder: null,
  recordChunks: [],
  recordStart: 0,
  muted: true,
  stage: { name: 'idle', requestedAt: 0, failedAt: 0, lastFrameAt: 0, error: null },
  padOpen: false,
  tour: null,
  bannerError: '',
  networkLostAt: null,
  expiredAt: null,
  resume: null,
  prefs: loadPrefs(),
};

function defaultPrefs() {
  return { theme: 'system', quality: 'main', lens: 'combined', osd: true, muted: true, flip: false };
}

function loadPrefs() {
  let stored = {};
  try {
    stored = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}') || {};
  } catch {
    stored = {};
  }
  let legacyTheme = null;
  try {
    legacyTheme = localStorage.getItem('pvw_theme');
  } catch {
    legacyTheme = null;
  }
  const prefs = { ...defaultPrefs(), ...stored };
  if (!stored.theme && (legacyTheme === 'dark' || legacyTheme === 'light')) {
    prefs.theme = legacyTheme;
  }
  return prefs;
}

function savePrefs() {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(state.prefs));
  } catch {}
}

function hms(time) {
  return new Date(time).toLocaleTimeString([], { hour12: false });
}

function hm(time) {
  return new Date(time).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit' });
}

function icon(name, extra) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'i' + (extra ? ' ' + extra : ''));
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttribute('href', '#i-' + name);
  svg.append(use);
  return svg;
}

function make(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function setBusy(button, busy, label) {
  button.setAttribute('aria-busy', busy ? 'true' : 'false');
  button.disabled = busy;
  if (label !== undefined) {
    if (busy) {
      button.dataset.label = button.textContent;
      button.style.minWidth = button.offsetWidth + 'px';
      button.textContent = label;
    } else if (button.dataset.label) {
      button.textContent = button.dataset.label;
      delete button.dataset.label;
    }
  }
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    ...options,
  });
  const text = await response.text();
  let body = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  if (!response.ok) {
    const message = body && body.message ? body.message : response.status + ' ' + response.statusText;
    const error = new Error(message);
    error.status = response.status;
    if (body && body.code) error.code = body.code;
    if (body) error.body = body;
    throw error;
  }
  return body;
}

function sessionLost(error) {
  if (!error || error.status !== 401 || !state.signedIn) {
    return false;
  }
  state.expiredAt = Date.now();
  state.resume = state.selectedId ? { id: state.selectedId, channel: state.channel } : null;
  showSignin({ expired: true });
  return true;
}

const floating = { open: null };

function menuItems(panel) {
  return [...panel.querySelectorAll('[role^="menuitem"]:not(:disabled)')];
}

function placeFloating(panel, anchor) {
  if (!anchor) {
    return;
  }
  const rect = anchor.getBoundingClientRect();
  const width = panel.offsetWidth;
  const height = panel.offsetHeight;
  let left = rect.left;
  if (left + width > window.innerWidth - 8) {
    left = rect.right - width;
  }
  left = Math.max(8, Math.min(left, window.innerWidth - width - 8));
  let top = rect.bottom + 6;
  if (top + height > window.innerHeight - 8) {
    top = Math.max(8, rect.top - height - 6);
  }
  panel.style.left = Math.round(left) + 'px';
  panel.style.top = Math.round(top) + 'px';
}

function settleFloating(restoreFocus) {
  const open = floating.open;
  if (!open) {
    return;
  }
  floating.open = null;
  if (open.anchor) {
    open.anchor.setAttribute('aria-expanded', 'false');
  }
  if (restoreFocus && open.restore) {
    open.restore.focus();
  }
}

function closeFloating(restoreFocus) {
  const open = floating.open;
  if (!open) {
    return false;
  }
  settleFloating(restoreFocus);
  if (!POPOVER) {
    open.panel.hidden = true;
  } else if (open.panel.matches(':popover-open')) {
    open.panel.hidePopover();
  }
  return true;
}

function openFloating(panel, anchor, focusTarget) {
  closeFloating(false);
  floating.open = { panel, anchor, restore: anchor || document.activeElement };
  if (POPOVER) {
    panel.showPopover();
  } else {
    panel.hidden = false;
  }
  if (anchor) {
    anchor.setAttribute('aria-expanded', 'true');
  }
  placeFloating(panel, anchor);
  const items = menuItems(panel);
  const target = focusTarget || items.find((item) => item.getAttribute('aria-checked') === 'true') || items[0];
  if (target) {
    target.focus();
  }
}

function toggleFloating(panel, anchor, prepare) {
  const pressedOpen = anchor.dataset.pressedOpen === 'true';
  delete anchor.dataset.pressedOpen;
  if (floating.open && floating.open.panel === panel) {
    closeFloating(true);
    return;
  }
  if (pressedOpen) {
    return;
  }
  if (prepare) {
    prepare();
  }
  openFloating(panel, anchor);
}

function onMenuKey(event) {
  const panel = event.currentTarget;
  const items = menuItems(panel);
  const index = items.indexOf(document.activeElement);
  let next = null;
  if (event.key === 'ArrowDown') {
    next = items[(index + 1) % items.length];
  } else if (event.key === 'ArrowUp') {
    next = items[(index - 1 + items.length) % items.length];
  } else if (event.key === 'Home') {
    next = items[0];
  } else if (event.key === 'End') {
    next = items[items.length - 1];
  } else if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    closeFloating(true);
    return;
  } else if (event.key === 'Tab') {
    closeFloating(true);
    return;
  } else {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  if (next) {
    next.focus();
  }
}

for (const panel of document.querySelectorAll('.menu')) {
  if (POPOVER) {
    panel.setAttribute('popover', 'auto');
    panel.hidden = false;
    panel.addEventListener('beforetoggle', (event) => {
      if (event.newState === 'closed' && floating.open && floating.open.panel === panel) {
        settleFloating(false);
      }
    });
  }
  panel.addEventListener('keydown', onMenuKey);
}

for (const anchor of document.querySelectorAll('[aria-haspopup="menu"]')) {
  anchor.addEventListener('pointerdown', () => {
    anchor.dataset.pressedOpen = String(Boolean(floating.open && floating.open.anchor === anchor));
  });
}

if (!POPOVER) {
  document.addEventListener(
    'pointerdown',
    (event) => {
      const open = floating.open;
      if (open && !open.panel.contains(event.target) && !(open.anchor && open.anchor.contains(event.target))) {
        closeFloating(false);
      }
    },
    true,
  );
}

window.addEventListener('resize', () => {
  if (floating.open) {
    placeFloating(floating.open.panel, floating.open.anchor);
  }
  layoutLensCrop();
});

function showScreen(name) {
  closeFloating(false);
  state.screen = name;
  for (const id of ['boot', 'signin', 'watch', 'settings']) {
    el(id).hidden = id !== name;
  }
  document.body.dataset.screen = name;
}

function resolveTheme() {
  const chosen = state.prefs.theme;
  if (chosen === 'light' || chosen === 'dark') {
    return chosen;
  }
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

function applyTheme() {
  const theme = resolveTheme();
  document.documentElement.dataset.theme = theme;
  for (const button of document.querySelectorAll('[data-theme-toggle]')) {
    button.setAttribute('aria-label', theme === 'dark' ? 'Switch to the light theme' : 'Switch to the dark theme');
    button.querySelector('use').setAttribute('href', theme === 'dark' ? '#i-sun' : '#i-moon');
  }
  for (const radio of document.querySelectorAll('input[name="theme"]')) {
    radio.checked = radio.value === state.prefs.theme;
  }
}

function setLoginError(message, fieldId) {
  const box = el(fieldId === 'local-pass' ? 'local-error' : 'login-error');
  const field = el(fieldId === 'local-pass' ? 'local-pass-field' : 'password-field');
  const input = el(fieldId || 'password');
  box.querySelector('span').textContent = message || '';
  box.hidden = !message;
  field.classList.toggle('bad', Boolean(message));
  if (message) {
    input.setAttribute('aria-invalid', 'true');
    input.setAttribute('aria-describedby', box.id);
    input.focus();
    input.select();
  } else {
    input.removeAttribute('aria-invalid');
    input.removeAttribute('aria-describedby');
  }
}

function showSignin(options = {}) {
  stopAll();
  state.signedIn = false;
  state.devices = [];
  state.cameras = [];
  state.selectedId = null;
  showScreen('signin');
  const expired = Boolean(options.expired);
  el('signin-notice').hidden = !expired;
  el('signin-lede').textContent = expired
    ? 'The server ended this session at ' + hm(state.expiredAt) + '. Signing in again picks the camera back up.'
    : SIGNIN_LEDE;
  el('signin-stage').textContent =
    expired && state.stage.lastFrameAt
      ? 'Last frame ' + hms(state.stage.lastFrameAt) + '. Signing in picks the stream up again.'
      : SIGNIN_STAGE;
  el('login-submit').textContent = expired ? 'Sign in to resume' : 'Sign in';
  setLoginError('');
  setLoginError('', 'local-pass');
  if (options.focus !== false) {
    el(expired && el('email').value ? 'password' : 'email').focus();
  }
}

function switchSignInMode(mode) {
  const cloud = mode !== 'local';
  el('tab-cloud').setAttribute('aria-selected', String(cloud));
  el('tab-local').setAttribute('aria-selected', String(!cloud));
  el('tab-cloud').tabIndex = cloud ? 0 : -1;
  el('tab-local').tabIndex = cloud ? -1 : 0;
  el('panel-cloud').hidden = !cloud;
  el('panel-local').hidden = cloud;
}

async function signedIn() {
  let me;
  try {
    me = await api('/api/me');
  } catch {
    showSignin({ focus: state.screen !== 'boot' });
    await loadMode();
    return;
  }
  state.signedIn = true;
  state.account = me.account || null;
  el('account').textContent = state.account || 'Signed in';
  if (state.screen !== 'settings') {
    showScreen('watch');
  }
  await loadMode();
  await loadDevices();
  if (state.screen === 'settings') {
    stopAll();
    renderSettingsChrome();
    return;
  }
  route(location.hash, false);
}

function route(hash, focus) {
  const raw = (hash || '').replace(/^#/, '');
  if (raw.startsWith('settings')) {
    openSettings(raw.split('/')[1] || state.section, { focus });
    return;
  }
  if (!state.signedIn) {
    return;
  }
  if (state.screen !== 'watch') {
    showScreen('watch');
  }
  setView(raw === 'grid' && state.cameras.length > 1 ? 'grid' : 'live');
}

function modeName(mode) {
  if (mode === 'local') {
    return 'this network';
  }
  if (mode === 'cloud') {
    return 'cloud account';
  }
  return 'unknown';
}

function sourceLabel() {
  if (state.ticket && state.ticket.source) {
    return state.ticket.source;
  }
  if (state.mode && state.mode.source) {
    return state.mode.source;
  }
  return 'source unknown';
}

async function loadMode() {
  try {
    state.mode = await api('/api/config/mode');
  } catch (error) {
    state.mode = null;
    console.error('mode report failed', error);
  }
  renderMode();
}

function renderMode() {
  const status = el('mode-status');
  const notice = el('mode-notice');
  const localMode = el('local-mode');
  for (const radio of document.querySelectorAll('input[name="mode"]')) {
    radio.disabled = !state.signedIn;
    radio.checked = Boolean(state.mode) && radio.value === state.mode.mode;
  }
  el('account-row').hidden = !state.signedIn;
  if (!state.mode) {
    status.textContent = 'The server did not report a provider mode.';
    notice.hidden = true;
    localMode.textContent = 'The server did not report which provider it uses.';
    setLocalFormEnabled(false);
    return;
  }
  status.textContent =
    'Mode: ' +
    modeName(state.mode.mode) +
    ' · answering: ' +
    state.mode.source +
    (state.signedIn ? '' : '. Sign in to switch the provider.');
  if (state.mode.notice) {
    notice.textContent =
      'Fallback: ' +
      (state.mode.notice.message || 'this network is not reachable, so the other provider answered.') +
      (state.mode.notice.to ? ' Answering provider: ' + state.mode.notice.to + '.' : '');
    notice.hidden = false;
  } else {
    notice.hidden = true;
  }
  localMode.textContent =
    state.mode.mode === 'local'
      ? 'The server is set to this network, so the camera login below is the one to use.'
      : 'The server is set to the ' + modeName(state.mode.mode) + '. Open Settings, Connection, choose this network, then come back to this tab.';
  setLocalFormEnabled(state.mode.mode === 'local');
  renderCameraInfo();
}

function setLocalFormEnabled(enabled) {
  for (const id of ['local-user', 'local-pass', 'local-submit']) {
    el(id).disabled = !enabled;
  }
}

async function setMode(mode) {
  try {
    state.mode = await api('/api/config/mode', { method: 'POST', body: JSON.stringify({ mode }) });
    state.control = { rows: [], onvif: false, dvrip: false, error: null, checkedFor: null };
    state.tour = null;
    renderMode();
    await loadDevices({ keepScreen: true });
  } catch (error) {
    if (sessionLost(error)) {
      return;
    }
    el('mode-status').textContent = 'The server did not change provider: ' + error.message;
    await loadMode();
  }
}

function buildCameras() {
  state.cameras = [];
  for (const device of state.devices) {
    const count = Math.max(1, device.channels || 1);
    for (let channel = 1; channel <= count; channel += 1) {
      state.cameras.push({
        device,
        channel,
        key: device.id + ':' + channel,
        name: count > 1 ? device.name + ' · channel ' + channel : device.name,
      });
    }
  }
}

function selectedDevice() {
  return state.devices.find((device) => device.id === state.selectedId) || null;
}

function currentCamera() {
  const key = state.selectedId + ':' + state.channel;
  return state.cameras.find((camera) => camera.key === key) || null;
}

function lensMeta() {
  const count = lensEntries().length;
  if (count > 1) {
    return 'dual lens';
  }
  return state.lens ? 'single lens' : '';
}

function deviceStatus(device) {
  if (!device) {
    return { known: false, online: false, detail: '' };
  }
  const status = String(device.online_status || '').toLowerCase();
  const known = status ? status !== 'unknown' : Boolean(device.online !== undefined);
  return { known, online: Boolean(device.online), detail: device.online_detail || '' };
}

function statusLine(device) {
  const status = deviceStatus(device);
  if (status.detail) {
    return status.detail;
  }
  return status.known ? (status.online ? 'cloud status online' : 'cloud status offline') : 'cloud status not read';
}

function renderSwitcher() {
  const camera = currentCamera();
  const switcher = el('switcher');
  const several = state.cameras.length > 1;
  switcher.classList.toggle('static', !several);
  switcher.disabled = !several;
  el('switcher-name').textContent = camera ? camera.name : 'No camera';
  const online = Boolean(camera && camera.device.online);
  const status = camera ? deviceStatus(camera.device) : { known: false, online: false };
  el('switcher-dot').classList.toggle('off', Boolean(camera) && status.known && !status.online);
  el('switcher-dot').setAttribute('aria-label', statusLine(camera ? camera.device : null));
  el('switcher-meta').textContent = camera
    ? ['channel ' + camera.channel, lensMeta()].filter(Boolean).join(' · ')
    : 'nothing selected';
  const grid = el('grid-toggle');
  grid.disabled = state.cameras.length === 0;
  const inGrid = state.view === 'grid';
  grid.setAttribute('aria-label', inGrid ? 'Back to the camera' : 'Show every camera');
  grid.title = (inGrid ? 'Back to the camera' : 'Show every camera') + ' (G)';
  grid.querySelector('use').setAttribute('href', inGrid ? '#i-frame' : '#i-grid');
}

function renderCameraMenu() {
  const menu = el('camera-menu');
  menu.replaceChildren();
  const count = state.cameras.length;
  menu.append(make('div', 'menu-h', 'Cameras · 1-' + Math.min(9, count) + ' on the keyboard'));
  state.cameras.forEach((camera, index) => {
    const row = make('button', 'menu-row');
    row.type = 'button';
    row.tabIndex = -1;
    row.setAttribute('role', 'menuitemradio');
    row.setAttribute('aria-checked', String(camera.key === state.selectedId + ':' + state.channel));
    const dot = make('span', 'dot' + (camera.device.online ? '' : ' off'));
    const col = make('span', 'col');
    col.append(
      make('span', 'name', camera.name),
      make('span', 'sub', camera.device.online ? ['channel ' + camera.channel, camera.device.model].filter(Boolean).join(' · ') : statusLine(camera.device)),
    );
    row.append(dot, col, make('span', 'meta', index < 9 ? String(index + 1) : ''));
    row.addEventListener('click', () => {
      closeFloating(true);
      openCamera(index);
    });
    menu.append(row);
  });
  menu.append(make('div', 'menu-sep'));
  const refresh = make('button', 'menu-row');
  refresh.type = 'button';
  refresh.tabIndex = -1;
  refresh.setAttribute('role', 'menuitem');
  refresh.append(icon('refresh', 'i-16'), make('span', 'name', 'Refresh the list'));
  refresh.addEventListener('click', () => {
    closeFloating(true);
    loadDevices();
  });
  const add = make('button', 'menu-row');
  add.type = 'button';
  add.tabIndex = -1;
  add.setAttribute('role', 'menuitem');
  add.append(icon('sliders', 'i-16'), make('span', 'name', 'Add a camera'), make('span', 'meta', 'Settings'));
  add.addEventListener('click', () => {
    closeFloating(false);
    openSettings('config', { focus: true });
  });
  menu.append(refresh, add);
}

function openCamera(index) {
  const camera = state.cameras[index];
  if (!camera) {
    return;
  }
  if (state.view !== 'live') {
    setView('live', { keepStream: true });
  }
  selectDevice(camera.device.id, { channel: camera.channel });
}

async function loadDevices(options = {}) {
  state.bannerError = '';
  renderBanner();
  try {
    state.devices = await api('/api/devices');
  } catch (error) {
    if (sessionLost(error)) {
      return;
    }
    state.bannerError = 'Could not load the cameras: ' + error.message;
    renderBanner();
    return;
  }
  buildCameras();
  renderCameraMenu();
  const keep = state.cameras.find((camera) => camera.key === state.selectedId + ':' + state.channel);
  const resume = state.resume && state.cameras.find((camera) => camera.key === state.resume.id + ':' + state.resume.channel);
  state.resume = null;
  el('empty').hidden = state.cameras.length > 0;
  el('live').hidden = state.cameras.length === 0 || state.view !== 'live';
  el('grid').hidden = state.cameras.length === 0 || state.view !== 'grid';
  if (!state.cameras.length) {
    state.selectedId = null;
    stopAll();
    renderSwitcher();
    renderCameraInfo();
    return;
  }
  const target = resume || keep || state.cameras[0];
  if (!keep || resume || options.keepScreen) {
    state.selectedId = null;
    await selectDevice(target.device.id, { channel: target.channel });
  } else {
    renderSwitcher();
    if (state.view === 'grid') {
      startGrid();
    }
  }
}

function renderBanner() {
  const camera = currentCamera();
  let title = '';
  let text = '';
  let action = null;
  if (state.networkLostAt) {
    title = 'This computer lost its network at ' + hms(state.networkLostAt) + '.';
    text = 'What you see below is the last frame the app received.';
    action = () => requestStream();
  } else if (state.view === 'live' && camera && deviceStatus(camera.device).known && !camera.device.online) {
    title = camera.name + ' is offline according to the cloud.';
    text = statusLine(camera.device) + '. Refresh the list once it answers again.';
    action = () => loadDevices();
  } else if (state.view === 'live' && camera && !deviceStatus(camera.device).known) {
    title = 'The cloud status of ' + camera.name + ' could not be read.';
    text = statusLine(camera.device) + '. The stream is still attempted, and its own error is shown below.';
    action = () => loadDevices();
  } else if (state.bannerError) {
    title = state.bannerError;
    action = () => loadDevices();
  }
  el('banner').hidden = !title;
  el('banner-title').textContent = title;
  el('banner-text').textContent = text;
  bannerAction = action;
}

let bannerAction = null;

function setStage(name, detail = {}) {
  const well = el('well');
  const camera = currentCamera();
  state.stage.name = name;
  if (name === 'loading') {
    state.stage.requestedAt = Date.now();
    state.stage.error = null;
  }
  if (name === 'error') {
    state.stage.failedAt = Date.now();
    state.stage.error = detail;
  }
  if (name === 'playing' && !performance.getEntriesByName('first-frame').length) {
    performance.mark('first-frame');
  }
  well.dataset.state = name;
  const showing = name === 'playing' || name === 'stalled';
  el('lens-crop').hidden = !showing;
  el('prog').hidden = name !== 'loading';
  const quality = state.prefs.quality === 'sub' ? 'SD' : 'HD';
  let title = '';
  let text = '';
  if (name === 'loading' && camera) {
    title = 'Requesting the stream';
    text = camera.name + ', channel ' + camera.channel + ', ' + quality + '. The server is asking the camera for a ticket.';
  } else if (name === 'error') {
    title = 'No picture from ' + (camera ? camera.name : 'this camera');
    text = detail.text || 'The stream stopped.';
  } else if (name === 'offline') {
    title = 'Camera offline';
    text = camera ? statusLine(camera.device) + '. The picture comes back after it answers and the list is refreshed.' : 'The cloud reports it offline.';
  }
  el('well-msg').hidden = !title;
  el('well-title').textContent = title;
  el('well-text').textContent = text;
  const code = name === 'error' && detail.code ? detail.code : '';
  el('well-code').hidden = !code;
  el('well-code').textContent = code;
  renderStage(name === 'error' ? detail.stage : null);
  el('well-actions').hidden = name !== 'error';
  el('check-password').hidden = !(name === 'error' && /401|unauthori|password|auth|login/i.test((detail.code || '') + ' ' + (detail.text || '')));
  el('record').disabled = name !== 'playing';
  el('snapshot').disabled = !showing;
  if (name !== 'playing' && name !== 'stalled') {
    stopRecording();
  }
  renderCaption();
  renderFsLine();
  layoutLensCrop();
  renderBanner();
}

function captionText() {
  const name = state.stage.name;
  if (state.networkLostAt && state.stage.lastFrameAt) {
    return 'not updating — last frame ' + hms(state.stage.lastFrameAt);
  }
  if (talk.active) {
    return 'live — the microphone is open';
  }
  if (name === 'loading') {
    return 'connecting — nothing has been received yet';
  }
  if (name === 'stalled') {
    return 'frozen — the frame below is from ' + hms(state.stage.lastFrameAt || Date.now());
  }
  if (name === 'error') {
    const after = Math.max(0, Math.round((state.stage.failedAt - state.stage.requestedAt) / 1000));
    return 'failed at ' + hms(state.stage.failedAt) + ' — ' + after + ' s after the request';
  }
  if (name === 'offline') {
    return 'offline';
  }
  if (name === 'playing') {
    const lens = lensEntries().length > 1 ? ', ' + lensViewLabel().toLowerCase() : '';
    return 'live' + lens + (state.padOpen ? ' — arrow keys move it' : '');
  }
  return '';
}

function renderCaption() {
  const camera = currentCamera();
  const name = camera ? camera.name : '';
  if (el('caption-name').textContent !== name) {
    el('caption-name').textContent = name;
  }
  const text = captionText();
  if (el('caption-state').textContent !== text) {
    el('caption-state').textContent = text;
  }
  renderFlag();
}

function renderFlag() {
  const flag = el('caption-flag');
  const name = state.stage.name;
  let text = '';
  let tone = '';
  if (state.recorder) {
    text = 'Recording ' + elapsed(state.recordStart);
    tone = 'rec';
  } else if (talk.active) {
    text = 'release to stop';
  } else if (state.networkLostAt) {
    text = 'stale';
    tone = 'warn';
  } else if (name === 'stalled') {
    text = 'frozen ' + Math.max(0, Math.round((Date.now() - (state.stage.lastFrameAt || Date.now())) / 1000)) + ' s';
    tone = 'warn';
  } else if (name === 'offline') {
    text = 'offline';
    tone = 'warn';
  } else if (name === 'loading' || name === 'error') {
    text = 'not live';
  } else if (name === 'playing') {
    const lag = liveLag();
    text = lag === null ? '' : lag.toFixed(1) + ' s behind';
  }
  flag.className = tone;
  flag.replaceChildren();
  if (tone === 'rec') {
    flag.append(make('span', 'dot rec'));
  }
  flag.append(document.createTextNode(text));
}

function elapsed(since) {
  const seconds = Math.max(0, Math.floor((Date.now() - since) / 1000));
  return String(Math.floor(seconds / 60)).padStart(2, '0') + ':' + String(seconds % 60).padStart(2, '0');
}

function liveLag() {
  const video = el('video');
  if (!video.buffered || video.buffered.length === 0) {
    return null;
  }
  const lag = video.buffered.end(video.buffered.length - 1) - video.currentTime;
  return Number.isFinite(lag) && lag >= 0 ? lag : null;
}

function renderFsLine() {
  const camera = currentCamera();
  const showing = state.stage.name === 'playing' || state.stage.name === 'stalled';
  el('fs-line').hidden = !(document.fullscreenElement === el('well') && state.prefs.osd && showing && camera);
  el('fs-name').textContent = camera ? camera.name + (lensEntries().length > 1 ? ' · ' + lensViewLabel().toLowerCase() : '') : '';
  el('fs-time').textContent = (state.stage.name === 'playing' ? 'live' : state.stage.name) + ' · ' + hms(Date.now());
}

function tick() {
  const now = hms(Date.now());
  el('clock').textContent = now;
  if (!el('fs-line').hidden) {
    renderFsLine();
  }
  if (state.recorder) {
    el('record').querySelector('span').textContent = elapsed(state.recordStart);
  }
  renderFlag();
  for (const tile of document.querySelectorAll('.tile[data-state="playing"] .tile-meta .right')) {
    tile.textContent = now;
  }
}

window.setInterval(tick, 1000);

const cloudPathWalks = new Set();

async function cloudPathDetail(deviceId) {
  if (cloudPathWalks.has(deviceId)) {
    return '';
  }
  cloudPathWalks.add(deviceId);
  try {
    const path = await api('/api/cloud-path?device=' + encodeURIComponent(deviceId));
    const parts = [];
    if (path.access) {
      parts.push('access server ' + path.access);
    }
    if (path.agent) {
      parts.push('agent ' + path.agent);
    }
    if (path.relayAccepted) {
      parts.push('relay accepted the session for ' + (path.destIp || 'the device'));
    } else if (path.relayError) {
      parts.push('relay ' + path.relayError);
    }
    if (!parts.length && path.detail) {
      parts.push(path.detail);
    }
    return parts.join(', ');
  } catch (error) {
    return '';
  } finally {
    cloudPathWalks.delete(deviceId);
  }
}

function stageLine(attempt) {
  return 'Failed at ' + attempt.stage + ' after ' + attempt.elapsed_ms + ' ms: ' + attempt.detail;
}

function stageTrailInline(attempt) {
  return (
    'Stages: ' +
    (attempt.stages || [])
      .map((entry) => entry.stage + ' ' + (entry.ok ? 'ok' : 'failed') + ' ' + entry.elapsed_ms + ' ms')
      .join(' · ')
  );
}

function stageTrail(attempt) {
  return (attempt.stages || [])
    .map((entry) => entry.stage + ' ' + (entry.ok ? 'ok' : 'failed') + ' in ' + entry.elapsed_ms + ' ms: ' + entry.detail)
    .join('\n');
}

function stagePanel() {
  let line = el('well-stage');
  if (!line) {
    line = document.createElement('p');
    line.className = 'mono';
    line.id = 'well-stage';
    const code = el('well-code');
    if (code && code.parentNode) {
      code.parentNode.insertBefore(line, code.nextSibling);
    }
  }
  if (!el('well-stage-copy')) {
    const button = document.createElement('button');
    button.className = 'btn btn-text inv';
    button.id = 'well-stage-copy';
    button.type = 'button';
    button.textContent = 'Copy the stage detail';
    button.addEventListener('click', copyStageDetail);
    const actions = el('well-actions');
    if (actions) {
      actions.appendChild(button);
    }
  }
  return line;
}

function renderStage(attempt) {
  const line = stagePanel();
  const button = el('well-stage-copy');
  state.stage.attempt = attempt && attempt.stages && attempt.stages.length ? attempt : null;
  line.textContent = state.stage.attempt ? stageTrailInline(state.stage.attempt) : '';
  line.hidden = !attempt;
  if (button) {
    button.hidden = !state.stage.attempt;
    button.textContent = 'Copy the stage detail';
  }
}

async function copyStageDetail() {
  const attempt = state.stage.attempt;
  if (!attempt) {
    return;
  }
  const head = attempt.stage
    ? stageLine(attempt)
    : 'No stage failed; the provider stopped: ' + (attempt.message || 'no message');
  const text = head + '\n' + stageTrail(attempt);
  const button = el('well-stage-copy');
  try {
    await navigator.clipboard.writeText(text);
    if (button) {
      button.textContent = 'Copied';
      window.setTimeout(() => {
        button.textContent = 'Copy the stage detail';
      }, 2000);
    }
  } catch (error) {
    setControlStatus('The browser refused the clipboard: ' + error.message);
  }
}

function explainStreamError(error, attempt) {
  if (attempt && attempt.stage) {
    return stageLine(attempt);
  }
  if (error.status === 501) {
    return 'Live video is not wired yet: the device transport is still being built, so there is no frame to show.';
  }
  if (error.status === 404) {
    return 'The server has no playable stream for that channel: ' + error.message;
  }
  if (error.status === 401) {
    return 'Session expired. Sign in again.';
  }
  if (error.status === 502) {
    return 'The server has no playable stream for this channel. The camera or the recording did not answer; the exact reason is in this line\u2019s tooltip.';
  }
  if (error.status === 400) {
    return 'The server rejected the request: ' + error.message;
  }
  return 'The stream could not be started: the server answered ' + error.status + '. The exact reason is in this line\u2019s tooltip.';
}

const MEDIA_SOURCE_TYPE = 'video/mp4; codecs="avc1.42E01E"';

class StreamPlayer {
  constructor(video, onState) {
    this.video = video;
    this.onState = onState || (() => {});
    this.socket = null;
    this.media = null;
    this.buffer = null;
    this.queue = [];
    this.url = null;
    this.hls = null;
    this.watchdog = null;
    this.lastTime = 0;
    this.lastAdvance = 0;
    this.stalled = false;
    this.metadataTimer = null;
    this.announced = false;
  }

  expectFirstFrame() {
    const video = this.video;
    this.announced = false;
    const announce = () => {
      if (this.announced) {
        return;
      }
      this.announced = true;
      if (this.metadataTimer) {
        window.clearTimeout(this.metadataTimer);
        this.metadataTimer = null;
      }
      this.startWatchdog();
      this.onState('playing', '');
    };
    if (video.videoWidth > 0) {
      announce();
      return;
    }
    video.addEventListener('loadedmetadata', announce, { once: true });
    this.metadataTimer = window.setTimeout(() => {
      if (!this.announced) {
        this.onState('error', 'The stream connected but sent no picture this browser could decode.');
      }
    }, 6000);
  }

  startWatchdog() {
    this.stopWatchdog();
    this.lastTime = this.video.currentTime;
    this.lastAdvance = Date.now();
    this.stalled = false;
    this.watchdog = window.setInterval(() => {
      const now = Date.now();
      const time = this.video.currentTime;
      if (time !== this.lastTime) {
        this.lastTime = time;
        this.lastAdvance = now;
        if (this.stalled) {
          this.stalled = false;
          this.onState('playing', '');
        }
        return;
      }
      if (this.video.paused) {
        return;
      }
      const idle = Math.round((now - this.lastAdvance) / 1000);
      if (idle >= 3) {
        this.stalled = true;
        this.onState(
          'stalled',
          'The picture has not advanced for ' +
            idle +
            ' s. The last decoded frame stays on screen, and the clock on it is not the time of what you see.',
        );
      }
    }, 1000);
  }

  stopWatchdog() {
    if (this.watchdog) {
      window.clearInterval(this.watchdog);
      this.watchdog = null;
    }
  }

  destroy() {
    this.stopWatchdog();
    if (this.metadataTimer) {
      window.clearTimeout(this.metadataTimer);
      this.metadataTimer = null;
    }
    if (this.socket) {
      this.socket.onmessage = null;
      this.socket.onclose = null;
      this.socket.onerror = null;
      this.socket.close();
      this.socket = null;
    }
    if (this.buffer && this.media && this.media.readyState === 'open') {
      try {
        this.media.removeSourceBuffer(this.buffer);
      } catch {}
    }
    if (this.hls) {
      this.hls.destroy();
      this.hls = null;
    }
    if (this.url) {
      URL.revokeObjectURL(this.url);
      this.url = null;
    }
    this.buffer = null;
    this.media = null;
    this.queue = [];
    this.video.pause();
    this.video.removeAttribute('src');
    this.video.load();
  }

  followEdge() {
    const video = this.video;
    const buffer = this.buffer;
    if (!video || !buffer || buffer.buffered.length === 0) {
      return;
    }
    const end = buffer.buffered.end(buffer.buffered.length - 1);
    const start = buffer.buffered.start(0);
    const inside = video.currentTime >= start && video.currentTime <= end;
    const behind = !inside || video.currentTime < end - LIVE_TARGET_LAG_SECONDS - 2;
    if (behind) {
      video.currentTime = Math.max(start, end - LIVE_TARGET_LAG_SECONDS);
      video.play().catch(() => {});
    }
    if (start < end - LIVE_WINDOW_SECONDS && !buffer.updating) {
      try {
        buffer.remove(0, end - LIVE_WINDOW_SECONDS);
      } catch {}
    }
  }

  pump() {
    const buffer = this.buffer;
    if (!buffer || buffer.updating) {
      return;
    }
    const next = this.queue.shift();
    if (!next) {
      this.followEdge();
      return;
    }
    try {
      buffer.appendBuffer(next);
    } catch (error) {
      console.error('append failed, dropping to the next keyframe', error);
      this.queue = [];
    }
  }

  playOverSocket(ticket) {
    const video = this.video;
    const media = new MediaSource();
    this.media = media;
    this.url = URL.createObjectURL(media);
    video.src = this.url;

    media.addEventListener('sourceopen', () => {
      let buffer;
      try {
        buffer = media.addSourceBuffer(MEDIA_SOURCE_TYPE);
      } catch (error) {
        this.onState('error', 'This browser cannot decode the stream codec: ' + error.message);
        return;
      }
      this.buffer = buffer;
      buffer.addEventListener('updateend', () => {
        this.pump();
        this.followEdge();
      });

      const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
      const target = ticket.url.startsWith('/')
        ? scheme + '://' + location.host + ticket.url
        : ticket.url.replace(/^http/, 'ws');
      const socket = new WebSocket(target);
      socket.binaryType = 'arraybuffer';
      this.socket = socket;
      socket.onmessage = (event) => {
        this.queue.push(new Uint8Array(event.data));
        this.pump();
      };
      socket.onerror = () => this.onState('error', 'The live socket failed to connect.');
      socket.onclose = () => {
        if (!video.paused) {
          this.onState('error', 'Live socket closed.');
        }
      };
      video.muted = state.muted;
      this.expectFirstFrame();
      video.play().catch(() => {});
    }, { once: true });
  }

  async play(ticket) {
    const video = this.video;
    if (ticket.kind === 'ws') {
      if (!window.MediaSource) {
        this.onState('error', 'This browser cannot play the live socket (no MediaSource).');
        return;
      }
      this.playOverSocket(ticket);
      return;
    }
    if (ticket.kind === 'mp4' || ticket.url.endsWith('.mp4')) {
      video.src = ticket.url;
      this.expectFirstFrame();
      await video.play().catch(() => {});
      return;
    }
    if (ticket.kind !== 'hls') {
      this.onState('error', 'Unsupported stream kind: ' + ticket.kind);
      return;
    }
    if (window.Hls && window.Hls.isSupported()) {
      const hls = new window.Hls({ lowLatencyMode: true });
      hls.on(window.Hls.Events.ERROR, (_event, data) => {
        if (data && data.fatal) {
          this.onState('error', 'Playlist could not be loaded: ' + data.type);
        }
      });
      hls.loadSource(ticket.url);
      hls.attachMedia(video);
      this.hls = hls;
      this.expectFirstFrame();
      await video.play().catch(() => {});
      return;
    }
    if (video.canPlayType('application/vnd.apple.mpegurl')) {
      video.src = ticket.url;
      this.expectFirstFrame();
      await video.play().catch(() => {});
      return;
    }
    this.onState('error', 'This browser has no HLS support and no hls.js is loaded.');
  }
}

function stagePlayer() {
  let player = state.players.get('stage');
  if (!player) {
    player = new StreamPlayer(el('video'), (name, message) => {
      if (name === 'stalled') {
        if (state.stage.name !== 'stalled') {
          setStage('stalled');
        }
        return;
      }
      if (name === 'error') {
        setStage('error', { text: message, code: '' });
        return;
      }
      setStage(name);
      renderCameraInfo();
    });
    state.players.set('stage', player);
  }
  return player;
}

function stopStagePlayer() {
  const player = state.players.get('stage');
  if (player) {
    player.destroy();
    state.players.delete('stage');
  }
  stopRecording();
}

function stopAll() {
  stopTalk();
  stopStagePlayer();
  stopGrid();
  closePad();
  state.ticket = null;
}

function normaliseCrop(crop, dims) {
  const safe = crop || {};
  const raw = {
    x: Number(safe.x) || 0,
    y: Number(safe.y) || 0,
    w: Number(safe.w) || 1,
    h: Number(safe.h) || 1,
  };
  const normalised = raw.w > 1 || raw.h > 1;
  if (!normalised) {
    return raw;
  }
  return {
    x: raw.x / dims.width,
    y: raw.y / dims.height,
    w: raw.w / dims.width,
    h: raw.h / dims.height,
  };
}

const LENS_WORDS = { full: 'Full frame', top: 'Top lens', middle: 'Middle lens', bottom: 'Bottom lens' };

function lensLabel(lens, index) {
  const raw = typeof lens.label === 'string' ? lens.label : '';
  const word = LENS_WORDS[raw.toLowerCase()];
  if (word) {
    return word;
  }
  return raw || 'Lens ' + (index + 1);
}

function lensRole(lens) {
  if (lens.ptz === true) {
    return 'moving';
  }
  if (lens.ptz === false) {
    return 'fixed';
  }
  return lens.role === 'moving' || lens.role === 'fixed' ? lens.role : 'unknown';
}

function normaliseLens(payload, source) {
  const streams = Array.isArray(payload && payload.streams) ? payload.streams : [];
  const dims = {
    width: streams[0] && streams[0].width ? streams[0].width : 1,
    height: streams[0] && streams[0].height ? streams[0].height : 1,
  };
  const lenses = Array.isArray(payload && payload.lenses) ? payload.lenses : [];
  return {
    deviceId: payload && payload.deviceId ? payload.deviceId : state.selectedId,
    channel: payload && payload.channel ? payload.channel : state.channel,
    streams,
    countSource: payload && payload.countSource ? payload.countSource : null,
    cropsVerified: payload && payload.cropsVerified === true,
    ptzSource: payload && payload.ptzSource ? payload.ptzSource : null,
    ptzEvidence: payload && payload.ptzEvidence ? payload.ptzEvidence : null,
    lenses: lenses.map((lens, index) => ({
      index: typeof lens.index === 'number' ? lens.index : index,
      label: lensLabel(lens, index),
      rawLabel: typeof lens.label === 'string' ? lens.label.toLowerCase() : '',
      role: lensRole(lens),
      crop: normaliseCrop(lens.crop, dims),
    })),
    source,
  };
}

function contractCaption(lens) {
  const countRule =
    lens.countSource === 'capability'
      ? 'reported by the device capabilities'
      : lens.countSource === 'geometry'
        ? 'inferred from the frame ratio'
        : 'reported by the device';
  const size = lens.streams[0] ? lens.streams[0].width + '×' + lens.streams[0].height : 'unknown size';
  const moving = lens.lenses.find((entry) => entry.role === 'moving');
  const fixed = lens.lenses.find((entry) => entry.role === 'fixed');
  const roles = [];
  if (moving) {
    roles.push(moving.label.toLowerCase() + ' can pan and tilt');
  }
  if (fixed) {
    roles.push(fixed.label.toLowerCase() + ' is aimed by hand');
  }
  const motor =
    lens.ptzSource === 'device'
      ? 'the device names the motor'
      : lens.ptzSource === 'measured_default'
        ? 'the motor lens is assumed, the device has not answered'
        : '';
  return (
    'Lens layout: ' +
    lens.lenses.length +
    ' lens' +
    (lens.lenses.length === 1 ? '' : 'es') +
    ', ' +
    countRule +
    ' (' +
    size +
    ')' +
    (roles.length ? ' · ' + roles.join(' · ') : '') +
    (motor ? ' · ' + motor : '') +
    '.'
  );
}

async function toggleTour() {
  const button = el('tour');
  try {
    if (state.tour !== null) {
      const stopped = await api('/api/device/controls/tour/stop?tour=' + enc(state.tour), { method: 'POST' });
      state.tour = null;
      button.setAttribute('aria-pressed', 'false');
      setControlStatus('Stopped tour ' + stopped.tour + '.');
      return;
    }
    const tour = await api('/api/device/controls/tour');
    const line = (tour.lines || []).find((entry) => (entry.points || []).length > 0);
    if (!line) {
      setControlStatus('No tour line with points on this camera: ' + (tour.note || 'add presets to a tour first'));
      return;
    }
    const started = await api('/api/device/controls/tour/start?tour=' + enc(line.id), { method: 'POST' });
    state.tour = line.id;
    button.setAttribute('aria-pressed', 'true');
    const stops = line.points.length === 1 ? '1 stop' : line.points.length + ' stops';
    setControlStatus('Tour ' + started.tour + ' running over ' + stops + '; the camera acks before it moves, so watch the picture.');
  } catch (error) {
    if (!sessionLost(error)) {
      setControlStatus('The camera refused the tour: ' + error.status + ' ' + error.message);
    }
  }
}

function deriveLensFromFrame() {
  const video = el('video');
  const width = video.videoWidth;
  const height = video.videoHeight;
  if (!width || !height) {
    return null;
  }
  const streams = [{ codec: '', width, height }];
  if (height / width <= 0.9) {
    return normaliseLens(
      { deviceId: state.selectedId, channel: state.channel, streams, lenses: [{ index: 0, label: 'Single lens', role: 'unknown', crop: { x: 0, y: 0, w: 1, h: 1 } }] },
      'geometry',
    );
  }
  return normaliseLens(
    {
      deviceId: state.selectedId,
      channel: state.channel,
      streams,
      lenses: [
        { index: 0, label: 'top', crop: { x: 0, y: 0, w: 1, h: 0.5 } },
        { index: 1, label: 'bottom', crop: { x: 0, y: 0.5, w: 1, h: 0.5 } },
      ],
    },
    'geometry',
  );
}

async function loadLens(deviceId, channel) {
  state.lens = null;
  renderLensPicker();
  if (state.lensSupport === 'absent') {
    return;
  }
  try {
    const payload = await api('/api/lens?device=' + enc(deviceId) + '&channel=' + enc(channel));
    state.lensSupport = 'present';
    state.lens = normaliseLens(payload, 'contract');
    renderLensPicker();
    applyLensView(state.lensView);
  } catch (error) {
    if (error.status === 404 || error.status === 501) {
      state.lensSupport = 'absent';
      renderLensPicker();
      return;
    }
    if (sessionLost(error)) {
      return;
    }
    console.error('lens contract failed', error);
    el('lens-source').textContent = 'The server answered ' + error.status + ' for GET /api/lens.';
  }
}

function lensEntries() {
  return state.lens && state.lens.lenses.length ? state.lens.lenses : [];
}

function lensViewLabel() {
  if (state.lensView === 'combined') {
    return 'Both lenses';
  }
  const entry = lensEntries().find((lens) => String(lens.index) === String(state.lensView));
  return entry ? entry.label : 'Both lenses';
}

function lensNote() {
  if (!state.selectedId) {
    return 'Pick a camera.';
  }
  if (!state.lens) {
    return 'Waiting for the first frame.';
  }
  if (state.lens.source === 'contract') {
    return contractCaption(state.lens).replace(/^Lens layout: /, '');
  }
  const dims = state.lens.streams[0].width + '×' + state.lens.streams[0].height;
  return state.lens.lenses.length > 1
    ? 'Split inferred from the frame shape (' + dims + '). The server has not reported the lens layout yet.'
    : 'One picture (' + dims + '), no stacked lens.';
}

function lensSub(entry) {
  if (entry.role === 'moving') {
    return 'pan and tilt from here';
  }
  if (entry.role === 'fixed') {
    return 'fixed, aimed by hand';
  }
  return entry.rawLabel === 'top' ? 'upper half of the frame' : entry.rawLabel === 'bottom' ? 'lower half of the frame' : 'one part of the frame';
}

function renderLensPicker() {
  const entries = lensEntries();
  const pair = entries.length > 1;
  if (!pair && state.lensView !== 'combined') {
    state.lensView = 'combined';
  }
  el('lens-pick').hidden = !pair;
  el('lens-pick-label').textContent = lensViewLabel();
  el('lens-source').textContent = lensNote();
  el('lens-source').title = state.lens && state.lens.ptzEvidence ? state.lens.ptzEvidence : '';
  el('lens-roles-note').hidden = entries.some((entry) => entry.role !== 'unknown');
  const menu = el('lens-menu');
  menu.replaceChildren(make('div', 'menu-h', 'Lens · L cycles'));
  const options = [{ value: 'combined', name: 'Both lenses', sub: 'the frame as the camera sends it' }].concat(
    entries.map((entry) => ({ value: String(entry.index), name: entry.label, sub: lensSub(entry) })),
  );
  for (const option of options) {
    const row = make('button', 'menu-row');
    row.type = 'button';
    row.tabIndex = -1;
    row.setAttribute('role', 'menuitemradio');
    row.setAttribute('aria-checked', String(option.value === String(state.lensView)));
    const col = make('span', 'col');
    col.append(make('span', 'name', option.name), make('span', 'sub', option.sub));
    row.append(icon('check', 'i-16'), col);
    row.addEventListener('click', () => {
      closeFloating(true);
      chooseLens(option.value);
    });
    menu.append(row);
  }
  menu.append(make('div', 'menu-sep'), make('p', 'menu-note', lensNote()));
  renderSwitcher();
  renderCaption();
}

function chooseLens(value) {
  applyLensView(value);
  renderLensPicker();
}

function cycleLens() {
  const entries = lensEntries();
  if (entries.length < 2) {
    return;
  }
  const order = ['combined'].concat(entries.map((entry) => String(entry.index)));
  const next = order[(order.indexOf(String(state.lensView)) + 1) % order.length];
  chooseLens(next);
}

function frameDims() {
  const video = el('video');
  const streams = state.lens && state.lens.streams.length ? state.lens.streams : null;
  return {
    width: streams && streams[0].width ? streams[0].width : video.videoWidth || 16,
    height: streams && streams[0].height ? streams[0].height : video.videoHeight || 9,
  };
}

function layoutLensCrop() {
  const well = el('well');
  const box = el('lens-crop');
  if (!well || !box || box.hidden) {
    return;
  }
  const dims = frameDims();
  const crop = currentCrop();
  const ratio = (crop.w * dims.width) / (crop.h * dims.height);
  const available = { width: well.clientWidth || 1, height: well.clientHeight || 1 };
  let width = available.width;
  let height = width / ratio;
  if (height > available.height) {
    height = available.height;
    width = height * ratio;
  }
  box.style.width = Math.max(1, Math.round(width)) + 'px';
  box.style.height = Math.max(1, Math.round(height)) + 'px';
  box.dataset.flipped = state.prefs.flip ? 'true' : 'false';
}

function applyLensView(value) {
  state.lensView = value;
  const video = el('video');
  const crop = currentCrop();
  video.style.left = (-crop.x / crop.w) * 100 + '%';
  video.style.top = (-crop.y / crop.h) * 100 + '%';
  video.style.width = 100 / crop.w + '%';
  video.style.height = 100 / crop.h + '%';
  el('well').dataset.lens = value;
  layoutLensCrop();
  renderFsLine();
}

function currentCrop() {
  if (state.lensView === 'combined') {
    return { x: 0, y: 0, w: 1, h: 1 };
  }
  const entry = lensEntries().find((lens) => String(lens.index) === String(state.lensView));
  return entry ? entry.crop : { x: 0, y: 0, w: 1, h: 1 };
}

function renderControlAvailability() {
  const rows = state.control.rows;
  const note = el('device-note');
  if (state.control.error) {
    note.textContent = 'Device controls could not be read: ' + state.control.error;
  } else if (!rows.length) {
    note.textContent = 'Device controls have not been checked yet.';
  } else {
    const supported = rows.filter((row) => row.state === 'supported').length;
    const backends = [state.control.onvif ? 'ONVIF' : null, state.control.dvrip ? 'DVRIP' : null].filter(Boolean);
    note.textContent =
      supported + ' of ' + rows.length + ' rows are usable' + (backends.length ? ' over ' + backends.join(' and ') : '') + '. Blocked rows keep the device\u2019s own reason.';
  }
  for (const button of document.querySelectorAll('[data-requires="device"]')) {
    const verdict = buttonVerdict(button);
    button.disabled = !verdict.ok;
    button.title = verdict.why;
  }
  if (state.padOpen && el('move').disabled) {
    closePad();
  }
  renderCapabilities();
}

const CONTROL_FEATURE = {
  up: 'ptz move',
  down: 'ptz move',
  left: 'ptz move',
  right: 'ptz move',
  home: 'ptz move',
  presets: 'ptz preset',
  strobe: 'white light',
  cruise: 'ptz patrol',
  intercom: 'two-way audio',
  move: 'ptz move',
};

const CONTROL_ABSENT = {
  tracking: 'No auto-tracking call exists on this server, and the capability report lists none for this device.',
  alarm: 'No audio-alarm call exists on this server; the device reports white light and two-way audio, not an alarm.',
  'cloud-storage': 'No server call for cloud storage in this build: the cloud API exposes the device list only.',
  'sd-album': 'No server call for the SD card album in this build: the cloud API exposes the device list only.',
};

function movingLensName() {
  const entry = lensEntries().find((lens) => lens.role === 'moving');
  const raw = entry ? entry.rawLabel : '';
  return raw === 'top' || raw === 'bottom' ? raw : 'bottom';
}

function capabilityRow(feature, lens) {
  const rows = state.control.rows.filter((row) => row.feature === feature);
  if (lens) {
    const exact = rows.find((row) => row.lens === lens);
    if (exact) {
      return exact;
    }
  }
  return rows.find((row) => row.state === 'supported') || rows[0] || null;
}

function buttonVerdict(button) {
  if (!state.control.rows.length) {
    return { ok: false, why: state.control.error || 'Device controls have not been checked yet.' };
  }
  const act = button.dataset.act || '';
  const feature = CONTROL_FEATURE[act];
  if (!feature) {
    return { ok: false, why: CONTROL_ABSENT[act] || 'No server call exists for this action in this build.' };
  }
  const lens = feature === 'ptz move' || feature === 'ptz preset' ? movingLensName() : null;
  const row = capabilityRow(feature, lens);
  if (!row) {
    return { ok: false, why: 'The capability report does not list "' + feature + '" for this device.' };
  }
  if (row.state !== 'supported') {
    return { ok: false, why: row.evidence || 'The device reports "' + feature + '" as blocked.' };
  }
  if (act === 'home' && !state.control.onvif) {
    return { ok: false, why: 'Stopping pan and tilt is served by the ONVIF channel, which this server does not expose.' };
  }
  return { ok: true, why: row.evidence || '' };
}

function renderCapabilities() {
  const body = el('capability-rows');
  body.replaceChildren();
  const rows = state.control.rows.map((row) => [row.feature, row.lens, row.backend, row.state, row.evidence]);
  const absent = [
    ['auto-tracking', 'tracking'],
    ['audio alarm', 'alarm'],
    ['cloud storage', 'cloud-storage'],
    ['sd card album', 'sd-album'],
  ];
  if (rows.length) {
    for (const [feature, act] of absent) {
      rows.push([feature, '—', 'none', 'unavailable', CONTROL_ABSENT[act]]);
    }
  }
  for (const row of rows) {
    const tr = document.createElement('tr');
    ['feature', 'lens', 'backend', 'state', 'evidence'].forEach((name, index) => {
      const cell = make('td', name);
      const text = row[index] == null ? '' : String(row[index]);
      if (name === 'state') {
        cell.append(make('span', 'pill ' + (text === 'supported' ? 'ok' : 'no'), text === 'supported' ? 'supported' : text || 'unavailable'));
      } else {
        cell.textContent = text;
      }
      tr.append(cell);
    });
    body.append(tr);
  }
}

async function probeControlChannel(device) {
  if (state.control.checkedFor === device.id) {
    renderControlAvailability();
    return;
  }
  try {
    const payload = await api('/api/device/controls');
    state.control = {
      rows: Array.isArray(payload.capabilities) ? payload.capabilities : [],
      onvif: payload.onvif === true,
      dvrip: payload.dvrip === true,
      error: null,
      checkedFor: device.id,
    };
  } catch (error) {
    if (sessionLost(error)) {
      return;
    }
    state.control = { rows: [], onvif: false, dvrip: false, error: error.status + ' ' + error.message, checkedFor: null };
  }
  renderControlAvailability();
}

function setControlStatus(message) {
  el('control-status').textContent = message || '';
}

let controlBusy = false;

async function loadPresets() {
  const list = el('presets');
  list.replaceChildren();
  el('preset-label').hidden = true;
  const row = capabilityRow('ptz preset', movingLensName());
  if (!row || row.state !== 'supported') {
    return;
  }
  try {
    const presets = await api('/api/device/controls/presets');
    el('preset-label').hidden = presets.length === 0;
    for (const preset of presets) {
      const button = make('button', '', preset.name || preset.token);
      button.type = 'button';
      button.title = 'GotoPreset ' + preset.token;
      button.addEventListener('click', async () => {
        try {
          await api('/api/device/controls/preset/goto?token=' + enc(preset.token), { method: 'POST' });
          setControlStatus('At ' + (preset.name || preset.token) + '.');
        } catch (error) {
          setControlStatus('The camera refused ' + (preset.name || preset.token) + ': ' + error.status + ' ' + error.message);
        }
      });
      list.append(button);
    }
  } catch (error) {
    if (!sessionLost(error)) {
      setControlStatus('Presets could not be read: ' + error.status + ' ' + error.message);
    }
  }
}

async function toggleWhiteLight() {
  const status = el('light-status');
  try {
    const current = await api('/api/device/controls/light');
    const next = current.mode === 'always' ? 'off' : 'always';
    const applied = await api('/api/device/controls/light?mode=' + enc(next), { method: 'POST' });
    status.textContent = 'White light: ' + (applied.mode || next) + ' (it was ' + (current.mode || 'not reported') + ').';
  } catch (error) {
    if (!sessionLost(error)) {
      status.textContent = 'The camera refused the white light: ' + error.status + ' ' + error.message;
    }
  }
}

async function sendControl(action) {
  if (controlBusy || !selectedDevice()) {
    return;
  }
  controlBusy = true;
  try {
    if (action === 'home') {
      const stopped = await api('/api/device/controls/ptz/stop', { method: 'POST' });
      setControlStatus(stopped.detail || 'Stopped.');
    } else if (action === 'cruise') {
      await toggleTour();
    } else if (action === 'up' || action === 'down' || action === 'left' || action === 'right') {
      const velocity = { up: [0, -0.5], down: [0, 0.5], left: [-0.5, 0], right: [0.5, 0] }[action];
      const query = 'lens=' + enc(movingLensName()) + '&pan=' + velocity[0] + '&tilt=' + velocity[1] + '&hold_ms=600';
      const moved = await api('/api/device/controls/ptz?' + query, { method: 'POST' });
      setControlStatus('Moved the ' + moved.lens + ' lens over ' + moved.backend + ' for ' + moved.hold_ms + ' ms.');
    } else {
      setControlStatus('No server call exists for "' + action + '" in this build.');
    }
  } catch (error) {
    if (!sessionLost(error)) {
      setControlStatus('The camera refused "' + action + '": ' + error.status + ' ' + error.message);
    }
  } finally {
    controlBusy = false;
  }
}

function openPad() {
  if (el('move').disabled || state.view !== 'live') {
    return;
  }
  state.padOpen = true;
  el('pad').hidden = false;
  el('move').setAttribute('aria-pressed', 'true');
  setControlStatus('');
  loadPresets();
  renderCaption();
}

function closePad() {
  if (!state.padOpen) {
    return;
  }
  state.padOpen = false;
  el('pad').hidden = true;
  el('move').setAttribute('aria-pressed', 'false');
  renderCaption();
}

function togglePad() {
  if (state.padOpen) {
    closePad();
  } else {
    openPad();
  }
}

async function selectDevice(deviceId, options = {}) {
  const device = state.devices.find((entry) => entry.id === deviceId);
  if (!device) {
    return;
  }
  const channel = options.channel || 1;
  const changed = state.selectedId !== deviceId || state.channel !== channel;
  if (changed) {
    stopTalk();
    stopRecording();
    closePad();
    state.lensView = state.prefs.lens;
  }
  state.selectedId = deviceId;
  state.channel = channel;
  state.ticket = null;
  state.stage.lastFrameAt = 0;
  state.tour = null;
  el('tour').setAttribute('aria-pressed', 'false');
  renderSwitcher();
  renderCameraMenu();
  setControlStatus('');
  el('light-status').textContent = '';
  if (state.view === 'grid') {
    renderTileSelection();
  }
  if (deviceStatus(device).known && !device.online) {
    stopStagePlayer();
    state.lens = null;
    renderLensPicker();
    setStage('offline');
    renderCameraInfo();
    await probeControlChannel(device);
    return;
  }
  setStage('loading');
  await loadLens(device.id, channel);
  if (state.view === 'live' && state.screen === 'watch') {
    await requestStream();
  }
  await probeControlChannel(device);
  renderCameraInfo();
}

async function requestStream() {
  const camera = currentCamera();
  if (!camera || (deviceStatus(camera.device).known && !camera.device.online) || !state.signedIn) {
    return;
  }
  const player = stagePlayer();
  player.destroy();
  state.ticket = null;
  setStage('loading');
  try {
    const ticket = await api(
      '/api/stream?device=' + enc(camera.device.id) + '&channel=' + enc(camera.channel) + '&quality=' + enc(state.prefs.quality),
    );
    if (currentCamera() !== camera) {
      return;
    }
    state.ticket = ticket;
    renderCameraInfo();
    await player.play(ticket);
  } catch (error) {
    if (sessionLost(error)) {
      return;
    }
    const attempt =
      error.body && Array.isArray(error.body.stages) && error.body.stages.length ? error.body : null;
    const base = explainStreamError(error, attempt);
    setStage('error', { text: base, code: error.message, stage: attempt });
    console.error('stream request failed', error);
    if (error.status === 501 && camera.device) {
      const deviceId = camera.device.id;
      cloudPathDetail(deviceId).then((detail) => {
        if (!detail || currentCamera() !== camera || state.stage.name !== 'error') {
          return;
        }
        setStage('error', {
          text: base + ' Cloud reach: ' + detail + '.',
          code: error.message,
          stage: attempt,
        });
      });
    }
  }
}

function renderCameraInfo() {
  const camera = currentCamera();
  el('camera-none').hidden = Boolean(camera);
  el('camera-rows').hidden = !camera;
  if (!camera) {
    return;
  }
  const device = camera.device;
  el('cam-name').textContent = camera.name;
  el('cam-id').textContent = device.id;
  el('cam-channel').textContent = 'channel ' + camera.channel;
  el('cam-model').textContent = device.model || 'model not reported';
  el('cam-connection').textContent =
    (state.mode ? modeName(state.mode.mode)[0].toUpperCase() + modeName(state.mode.mode).slice(1) + ' · answering ' + sourceLabel() : sourceLabel()) +
    (device.lan_host ? ' · ' + device.lan_host : '');
  const ticket = state.ticket;
  el('cam-stream').textContent = ticket
    ? 'Stream: ' + ticket.kind.toUpperCase() + ' · ' + (state.prefs.quality === 'sub' ? 'SD' : 'HD') + ' · ' + ticket.url + (ticket.expires_in ? ' · ticket expires in ' + ticket.expires_in + ' s' : '')
    : 'No stream ticket yet.';
}

function shortStreamError(error) {
  if (error.status === 404) {
    return 'This channel does not exist on the device.';
  }
  if (error.status === 401) {
    return 'Session expired. Sign in again.';
  }
  if (error.status === 502) {
    return 'No playable stream for this channel: the server answered 502.';
  }
  if (error.status === 501) {
    return 'Live video is not wired for this channel yet.';
  }
  return 'The stream could not be started: the server answered ' + error.status + '.';
}

function renderMute() {
  const video = el('video');
  video.muted = state.muted;
  const button = el('mute');
  button.setAttribute('aria-pressed', String(!state.muted));
  button.querySelector('use').setAttribute('href', state.muted ? '#i-speaker-off' : '#i-speaker');
  button.title = (state.muted ? 'Sound is off' : 'Sound is on') + ' (M)';
}

function toggleMute() {
  state.muted = !state.muted;
  renderMute();
}

function download(blob, name) {
  const link = document.createElement('a');
  link.href = URL.createObjectURL(blob);
  link.download = name;
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(link.href), 4000);
}

function stamp() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}

function captureSnapshot() {
  const video = el('video');
  const device = selectedDevice();
  if (!device || state.stage.name !== 'playing' || !video.videoWidth) {
    return;
  }
  const crop = currentCrop();
  const sx = Math.round(crop.x * video.videoWidth);
  const sy = Math.round(crop.y * video.videoHeight);
  const sw = Math.round(crop.w * video.videoWidth);
  const sh = Math.round(crop.h * video.videoHeight);
  const canvas = document.createElement('canvas');
  canvas.width = sw;
  canvas.height = sh;
  const context = canvas.getContext('2d');
  if (state.prefs.flip) {
    context.translate(sw, sh);
    context.rotate(Math.PI);
  }
  context.drawImage(video, sx, sy, sw, sh, 0, 0, sw, sh);
  canvas.toBlob((blob) => {
    if (blob) {
      download(blob, device.id + '_ch' + state.channel + '_lens-' + state.lensView + '_' + stamp() + '.png');
    }
  }, 'image/png');
}

function stopRecording() {
  if (state.recorder) {
    state.recorder.stop();
  }
}

function renderRecord() {
  const button = el('record');
  const on = Boolean(state.recorder);
  button.classList.toggle('rec-on', on);
  button.setAttribute('aria-pressed', String(on));
  button.setAttribute('aria-label', on ? 'Stop recording' : 'Record');
  button.querySelector('span').textContent = on ? elapsed(state.recordStart) : 'Record';
  button.querySelector('span').classList.toggle('mono', on);
  renderFlag();
}

function toggleRecord() {
  const video = el('video');
  if (state.recorder) {
    state.recorder.stop();
    return;
  }
  if (state.stage.name !== 'playing' || typeof video.captureStream !== 'function') {
    setControlStatus('Recording needs a playing picture and a browser with captureStream.');
    return;
  }
  const stream = video.captureStream();
  if (!stream || stream.getVideoTracks().length === 0) {
    return;
  }
  const candidates = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'];
  const mimeType = candidates.find((type) => window.MediaRecorder && MediaRecorder.isTypeSupported(type));
  if (!mimeType) {
    return;
  }
  state.recordChunks = [];
  const recorder = new MediaRecorder(stream, { mimeType });
  state.recorder = recorder;
  state.recordStart = Date.now();
  recorder.addEventListener('dataavailable', (event) => {
    if (event.data && event.data.size) {
      state.recordChunks.push(event.data);
    }
  });
  recorder.addEventListener('stop', () => {
    const device = selectedDevice();
    const blob = new Blob(state.recordChunks, { type: mimeType });
    state.recordChunks = [];
    state.recorder = null;
    renderRecord();
    if (blob.size) {
      download(blob, (device ? device.id : 'camera') + '_ch' + state.channel + '_' + stamp() + '.webm');
    }
    for (const track of stream.getTracks()) {
      track.stop();
    }
  });
  recorder.start();
  renderRecord();
}

const talk = { active: false, stream: null, context: null, frame: 0 };

async function startTalk() {
  const button = el('talk');
  if (talk.active || button.disabled || state.view !== 'live') {
    return;
  }
  talk.active = true;
  button.classList.add('talk-on');
  button.setAttribute('aria-pressed', 'true');
  el('level').hidden = false;
  renderCaption();
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    if (!talk.active) {
      stream.getTracks().forEach((track) => track.stop());
      return;
    }
    const context = new AudioContext();
    const analyser = context.createAnalyser();
    analyser.fftSize = 256;
    context.createMediaStreamSource(stream).connect(analyser);
    talk.stream = stream;
    talk.context = context;
    const samples = new Uint8Array(analyser.fftSize);
    const bars = el('level').children;
    const draw = () => {
      analyser.getByteTimeDomainData(samples);
      let sum = 0;
      for (const sample of samples) {
        const value = (sample - 128) / 128;
        sum += value * value;
      }
      const lit = Math.min(5, Math.round(Math.sqrt(sum / samples.length) * 20));
      for (let index = 0; index < bars.length; index += 1) {
        bars[index].classList.toggle('on', index < lit);
      }
      el('level').dataset.level = String(lit);
      talk.frame = requestAnimationFrame(draw);
    };
    draw();
  } catch (error) {
    stopTalk();
    setControlStatus('The microphone did not open: ' + error.message);
  }
}

function stopTalk() {
  if (!talk.active) {
    return;
  }
  talk.active = false;
  cancelAnimationFrame(talk.frame);
  if (talk.stream) {
    talk.stream.getTracks().forEach((track) => track.stop());
  }
  if (talk.context) {
    talk.context.close();
  }
  talk.stream = null;
  talk.context = null;
  const button = el('talk');
  button.classList.remove('talk-on');
  button.setAttribute('aria-pressed', 'false');
  el('level').hidden = true;
  el('level').dataset.level = '0';
  renderCaption();
}

async function toggleFullscreen() {
  const target = state.view === 'grid' ? el('tiles') : el('well');
  if (document.fullscreenElement) {
    await document.exitFullscreen().catch(() => {});
    return;
  }
  if (target.requestFullscreen) {
    await target.requestFullscreen().catch(() => setControlStatus('Full screen was refused by the browser.'));
  }
}

function startGrid() {
  ProviewGrid.open();
}

function stopGrid() {
  ProviewGrid.close();
}

function renderTileSelection() {
  ProviewGrid.syncSelection();
}

function moveTileFocus(key) {
  ProviewGrid.moveFocus(key);
}

function setView(name, options = {}) {
  const next = name === 'grid' && state.cameras.length > 0 ? 'grid' : 'live';
  const changed = state.view !== next;
  state.view = next;
  if (location.hash !== '#' + next) {
    history.replaceState(null, '', '#' + next);
  }
  const any = state.cameras.length > 0;
  el('live').hidden = !any || next !== 'live';
  el('grid').hidden = !any || next !== 'grid';
  el('empty').hidden = any;
  renderSwitcher();
  renderBanner();
  if (next === 'grid') {
    stopTalk();
    closePad();
    stopStagePlayer();
    startGrid();
    return;
  }
  stopGrid();
  if (any && (changed || !state.players.get('stage')) && !options.keepStream) {
    const camera = currentCamera();
    if (camera && (!deviceStatus(camera.device).known || camera.device.online)) {
      requestStream();
    } else if (camera) {
      setStage('offline');
    }
  }
}

const OPTION_LABELS = {
  proview: 'Vendor cloud',
  cloud: 'Cloud account',
  local: 'This network',
  fixture: 'Built-in sample data',
};

const MONO_KEY = /addr|host|base|listen|dir|rtsp|onvif|ffmpeg|url|port|profile/i;

let settingsView = null;
let settingsInitial = new Map();
let settingsRestartKeys = new Set();
let pendingRestart = 0;
let focusAfterSettings = null;

function settingsInputId(key) {
  return 'setting-' + key.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
}

function setSettingsStatus(message, bad = false) {
  const box = el('settings-status');
  box.textContent = message || '';
  box.classList.toggle('warn-text', bad);
}

function settingsCaret() {
  const active = document.activeElement;
  if (!active || !active.dataset || !active.dataset.key || !active.closest('#settings-fields')) {
    return null;
  }
  const caret =
    typeof active.selectionStart === 'number' ? { start: active.selectionStart, end: active.selectionEnd } : null;
  return { key: active.dataset.key, caret };
}

function restoreSettingsCaret(focus) {
  const input = document.querySelector('#settings-fields [data-key="' + focus.key + '"]');
  if (!input) {
    return;
  }
  input.focus();
  if (!focus.caret) {
    return;
  }
  try {
    input.setSelectionRange(focus.caret.start, focus.caret.end);
  } catch {}
}

function renderSettings(view, options = {}) {
  settingsView = view;
  const host = el('settings-fields');
  const typed = options.reset ? new Map() : new Map(Object.entries(settingsPayload()));
  const focus = options.reset ? null : settingsCaret();
  host.replaceChildren();
  settingsInitial = new Map();
  settingsRestartKeys = new Set(view.fields.filter((field) => field.restart).map((field) => field.key));
  const groups = new Map();
  for (const field of view.fields) {
    if (!groups.has(field.group)) groups.set(field.group, []);
    groups.get(field.group).push(field);
  }
  for (const [group, fields] of groups) {
    host.append(make('h3', 'set-group', group));
    for (const field of fields) {
      const id = settingsInputId(field.key);
      const row = make('div', 'row');
      const label = make('label', 'lab', field.label);
      label.setAttribute('for', id);
      if (field.restart) {
        label.append(make('span', 'hint', 'Needs a restart.'));
      }
      if (field.hint) {
        label.append(make('span', 'hint', field.hint));
      }
      for (const hint of label.querySelectorAll('.hint')) {
        hint.style.display = 'block';
      }
      const column = make('div', 'ctl-col');
      let input;
      if (field.kind === 'select') {
        input = make('select', 'text-input');
        for (const option of field.options) {
          const item = make('option', '', OPTION_LABELS[option] || option);
          item.value = option;
          input.append(item);
        }
        input.value = field.value || field.options[0] || '';
      } else {
        input = make('input', 'text-input' + (field.kind !== 'secret' && MONO_KEY.test(field.key) ? ' mono-input' : ''));
        input.type = field.kind === 'secret' ? 'password' : field.kind === 'number' ? 'number' : 'text';
        if (field.kind === 'secret') {
          input.placeholder = field.set ? 'stored, type to replace' : 'not set';
          input.autocomplete = 'new-password';
          input.value = '';
        } else {
          input.value = field.value || '';
        }
      }
      input.id = id;
      input.spellcheck = false;
      input.dataset.key = field.key;
      input.dataset.secret = field.secret ? 'true' : 'false';
      column.append(input);
      row.append(label, column);
      host.append(row);
      settingsInitial.set(field.key, { secret: field.secret, set: field.set, value: input.value });
    }
  }
  for (const [key, value] of typed) {
    const input = host.querySelector('[data-key="' + key + '"]');
    if (input) {
      input.value = value;
    }
  }
  if (focus) {
    restoreSettingsCaret(focus);
  }
  renderSaveBar();
}

function settingsPayload() {
  const payload = {};
  for (const input of document.querySelectorAll('#settings-fields input, #settings-fields select')) {
    const key = input.dataset.key;
    const secret = input.dataset.secret === 'true';
    const initial = settingsInitial.get(key) || { secret, set: false, value: '' };
    if (secret && input.value === '') continue;
    if (!secret && input.value === initial.value) continue;
    payload[key] = input.value;
  }
  return payload;
}

function desktopControl() {
  const control = window.__PROVIEW_DESKTOP__;
  return control && typeof control.token === 'string' && control.token ? control : null;
}

function renderSaveBar() {
  const changed = Object.keys(settingsPayload()).length;
  const restart = Boolean(desktopControl());
  el('settings-discard').hidden = changed === 0;
  el('settings-save').hidden = changed === 0;
  el('settings-restart').hidden = !(changed === 0 && pendingRestart > 0 && restart);
  let text = 'Everything saved';
  if (changed) {
    text = 'Unsaved changes · ' + changed + ' field' + (changed === 1 ? '' : 's');
  } else if (pendingRestart) {
    text =
      'Saved. ' +
      (restart ? 'Restart the app' : 'Restart the server process') +
      ' to apply ' +
      pendingRestart +
      ' field' +
      (pendingRestart === 1 ? '' : 's');
  }
  if (el('save-state').textContent !== text) {
    el('save-state').textContent = text;
  }
}

function settingsDirty() {
  return Object.keys(settingsPayload()).length > 0;
}

async function loadSettings() {
  setSettingsStatus('Loading…');
  try {
    const view = await api('/api/settings');
    renderSettings(view);
    if (view.exists) {
      setSettingsStatus('Saved values come from ' + view.path + '.');
    } else if (view.path) {
      setSettingsStatus('Nothing saved yet. Saving writes ' + view.path + '.');
    } else {
      setSettingsStatus('No data folder is set, so saved values apply to this run only.');
    }
    if (focusAfterSettings) {
      const target = document.querySelector('#settings-fields [data-key="' + focusAfterSettings + '"]');
      focusAfterSettings = null;
      if (target) {
        target.focus();
      }
    }
  } catch (error) {
    setSettingsStatus('Could not read the settings: ' + error.message, true);
  }
}

async function saveSettings() {
  const payload = settingsPayload();
  const keys = Object.keys(payload);
  if (!keys.length) {
    return;
  }
  const button = el('settings-save');
  setBusy(button, true, 'Saving…');
  try {
    const view = await api('/api/settings', { method: 'POST', body: JSON.stringify({ values: payload }) });
    pendingRestart += keys.filter((key) => settingsRestartKeys.has(key)).length;
    renderSettings(view, { reset: true });
    setSettingsStatus('Saved ' + keys.length + ' field' + (keys.length === 1 ? '' : 's') + ' to ' + (view.path || 'this run') + '.');
  } catch (error) {
    setSettingsStatus('Could not save: ' + error.message, true);
  } finally {
    setBusy(button, false, '');
    renderSaveBar();
  }
}

function showSection(name, focus) {
  const section = SECTIONS.includes(name) ? name : 'playback';
  state.section = section;
  for (const link of document.querySelectorAll('.set-nav a')) {
    if (link.dataset.section === section) {
      link.setAttribute('aria-current', 'page');
      link.tabIndex = 0;
    } else {
      link.removeAttribute('aria-current');
      link.tabIndex = -1;
    }
  }
  for (const node of document.querySelectorAll('#set-body > section')) {
    node.hidden = node.dataset.section !== section;
  }
  el('set-body').scrollTop = 0;
  if (location.hash !== '#settings/' + section) {
    history.replaceState(null, '', '#settings/' + section);
  }
  if (focus) {
    el('h-' + section).focus();
  }
}

function renderSettingsChrome() {
  el('settings-back-label').textContent = state.signedIn ? 'Live' : 'Sign in';
  renderMode();
  renderCameraInfo();
  renderControlAvailability();
}

function openSettings(section, options = {}) {
  stopAll();
  showScreen('settings');
  showSection(section || state.section, options.focus);
  renderPrefsIntoForm();
  renderSettingsChrome();
  loadSettings();
  if (!options.focus) {
    el('settings-back').focus();
  }
}

function leaveSettings() {
  if (settingsDirty()) {
    el('save-state').textContent = 'Save or discard the ' + Object.keys(settingsPayload()).length + ' changed field(s) first';
    el('settings-save').focus();
    return;
  }
  if (!state.signedIn) {
    showSignin();
    return;
  }
  showScreen('watch');
  const view = state.view;
  state.view = null;
  setView(view);
  el('open-settings').focus();
}

function renderPrefsIntoForm() {
  for (const name of ['theme', 'quality', 'lens']) {
    for (const radio of document.querySelectorAll('input[name="' + name + '"]')) {
      radio.checked = radio.value === state.prefs[name];
    }
  }
  el('set-osd').checked = state.prefs.osd;
  el('set-muted').checked = state.prefs.muted;
  el('set-flip').checked = state.prefs.flip;
  el('quality-pick-label').textContent = 'Quality ' + (state.prefs.quality === 'sub' ? 'SD' : 'HD');
  for (const row of el('quality-menu').querySelectorAll('[data-quality]')) {
    row.setAttribute('aria-checked', String(row.dataset.quality === state.prefs.quality));
    row.tabIndex = -1;
  }
}

function setQuality(quality) {
  if (state.prefs.quality === quality) {
    return;
  }
  state.prefs.quality = quality;
  savePrefs();
  renderPrefsIntoForm();
  if (state.screen !== 'watch') {
    return;
  }
  if (state.view === 'grid') {
    startGrid();
  } else {
    requestStream();
  }
}

async function runChecks() {
  const button = el('run-checks');
  setBusy(button, true);
  const write = (id, text, ok) => {
    const cell = el(id);
    cell.textContent = text;
    cell.dataset.ok = ok === undefined ? '' : String(ok);
  };
  for (const id of ['check-health', 'check-account', 'check-probe', 'check-ticket']) {
    write(id, 'Checking…');
  }
  try {
    const health = await api('/api/health');
    write('check-health', health.ok ? 'ok · ' + health.service : 'unexpected: ' + JSON.stringify(health), Boolean(health.ok));
  } catch (error) {
    write('check-health', error.status + ' ' + error.message, false);
  }
  try {
    const me = await api('/api/me');
    write('check-account', me.account, true);
  } catch (error) {
    write('check-account', error.status + ' ' + error.message, false);
  }
  try {
    const probe = await api('/api/device/rtsp', { method: 'POST' });
    write('check-probe', 'ok · ' + probe.host + ' · ' + (probe.video_codec || 'codec unknown'), true);
  } catch (error) {
    write('check-probe', error.status + ' ' + error.message, false);
  }
  const camera = currentCamera();
  if (!camera) {
    write('check-ticket', 'no camera open', undefined);
  } else {
    try {
      const ticket = await api('/api/stream?device=' + enc(camera.device.id) + '&channel=' + enc(camera.channel) + '&quality=' + enc(state.prefs.quality));
      write('check-ticket', ticket.kind + ' · ' + ticket.url, true);
    } catch (error) {
      write('check-ticket', error.status + ' ' + error.message, false);
    }
  }
  setBusy(button, false);
}

function typing(target) {
  return Boolean(target && target.closest && target.closest('input, textarea, select, [contenteditable="true"]'));
}

function toggleKeys() {
  if (floating.open && floating.open.panel === el('keys')) {
    closeFloating(true);
    return;
  }
  openFloating(el('keys'), null, el('keys-close'));
}

function onWatchKey(event) {
  const key = event.key;
  if (key === 'Escape') {
    if (closeFloating(true)) {
      event.preventDefault();
    } else if (state.padOpen) {
      event.preventDefault();
      closePad();
      el('move').focus();
    } else if (document.fullscreenElement) {
      event.preventDefault();
      document.exitFullscreen().catch(() => {});
    }
    return;
  }
  if (event.ctrlKey || event.metaKey || event.altKey || typing(event.target)) {
    return;
  }
  if (/^[1-9]$/.test(key)) {
    event.preventDefault();
    closeFloating(false);
    openCamera(Number(key) - 1);
    return;
  }
  if (key.startsWith('Arrow')) {
    if (state.view === 'live' && state.padOpen) {
      event.preventDefault();
      const act = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' }[key];
      if (!document.querySelector('.pk.' + act).disabled) {
        sendControl(act);
      }
    } else if (state.view === 'grid') {
      event.preventDefault();
      moveTileFocus(key);
    }
    return;
  }
  if (key === ' ') {
    if (state.padOpen && !event.target.closest('button')) {
      event.preventDefault();
      if (!el('pad').querySelector('.home').disabled) {
        sendControl('home');
      }
    }
    return;
  }
  const actions = {
    g: () => (el('grid-toggle').disabled ? null : setView(state.view === 'grid' ? 'live' : 'grid')),
    f: () => toggleFullscreen(),
    m: () => toggleMute(),
    r: () => (el('record').disabled ? null : toggleRecord()),
    s: () => (el('snapshot').disabled ? null : captureSnapshot()),
    l: () => cycleLens(),
    q: () => setQuality(state.prefs.quality === 'sub' ? 'main' : 'sub'),
    v: () => togglePad(),
    t: () => (event.repeat ? null : startTalk()),
    ',': () => openSettings(state.section),
    '?': () => toggleKeys(),
  };
  const action = actions[key.length === 1 ? key.toLowerCase() : key];
  if (action) {
    event.preventDefault();
    action();
  }
}

document.addEventListener('keydown', (event) => {
  if (event.defaultPrevented) {
    return;
  }
  if (state.screen === 'watch') {
    onWatchKey(event);
    return;
  }
  if (state.screen === 'settings') {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      saveSettings();
    } else if (event.key === 'Escape' && !typing(event.target) && !settingsDirty()) {
      event.preventDefault();
      leaveSettings();
    }
  }
});

document.addEventListener('keyup', (event) => {
  if (event.key.toLowerCase() === 't') {
    stopTalk();
  }
});

window.addEventListener('blur', () => stopTalk());

el('login-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  setLoginError('');
  const button = el('login-submit');
  setBusy(button, true, 'Signing in…');
  try {
    await api('/api/login', {
      method: 'POST',
      body: JSON.stringify({ email: el('email').value, password: el('password').value }),
    });
    el('password').value = '';
    setBusy(button, false, '');
    await signedIn();
  } catch (error) {
    setBusy(button, false, '');
    setLoginError(
      error.status === 401 ? 'That email and password were not accepted: ' + error.message : 'Sign in failed: ' + error.message,
    );
  }
});

el('tab-cloud').addEventListener('click', () => switchSignInMode('cloud'));
el('tab-local').addEventListener('click', () => switchSignInMode('local'));
for (const tab of [el('tab-cloud'), el('tab-local')]) {
  tab.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
      event.preventDefault();
      const next = tab.id === 'tab-cloud' ? 'local' : 'cloud';
      switchSignInMode(next);
      el('tab-' + next).focus();
    }
  });
}

el('local-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  setLoginError('', 'local-pass');
  const button = el('local-submit');
  setBusy(button, true, 'Signing in…');
  try {
    await api('/api/login', {
      method: 'POST',
      body: JSON.stringify({ email: el('local-user').value, password: el('local-pass').value }),
    });
    el('local-pass').value = '';
    setBusy(button, false, '');
    await signedIn();
  } catch (error) {
    setBusy(button, false, '');
    setLoginError(
      error.status === 401 ? 'The camera did not accept that username and password.' : 'The camera login failed: ' + error.message,
      'local-pass',
    );
  }
});

el('local-connect').addEventListener('click', async () => {
  const button = el('local-connect');
  setLoginError('', 'local-pass');
  el('local-report').hidden = true;
  el('local-next').hidden = true;
  setBusy(button, true, 'Probing…');
  try {
    const report = await api('/api/device/probe', { method: 'POST' });
    el('local-host').textContent = report.host;
    el('local-session').textContent = String(report.session);
    el('local-type').textContent = report.device_type || 'not reported';
    el('local-video').textContent = report.video_codec || 'not reported';
    el('local-audio').textContent = report.audio_codec || 'none';
    el('local-fps').textContent = report.framerate ? report.framerate + ' fps' : 'not reported';
    el('local-report').hidden = false;
    el('local-next').hidden = false;
  } catch (error) {
    el('local-mode').textContent =
      error.code === 'device_host_missing'
        ? 'This server has no camera address yet. Open Settings, Configuration, add the camera address and its login, restart the app, then probe again.'
        : 'The camera on this network did not answer: ' + error.message;
  } finally {
    setBusy(button, false, '');
  }
});

for (const radio of document.querySelectorAll('input[name="mode"]')) {
  radio.addEventListener('change', () => {
    if (radio.checked) {
      setMode(radio.value);
    }
  });
}

el('signout').addEventListener('click', async () => {
  try {
    await api('/api/logout', { method: 'POST' });
  } catch {}
  state.stage.lastFrameAt = 0;
  if (settingsView) {
    renderSettings(settingsView, { reset: true });
  }
  showSignin();
  await loadMode();
});

el('login-settings').addEventListener('click', () => openSettings('config', { focus: true }));
el('empty-settings').addEventListener('click', () => openSettings('config', { focus: true }));
el('open-settings').addEventListener('click', () => openSettings(state.section));
el('settings-back').addEventListener('click', () => leaveSettings());
el('settings-save').addEventListener('click', () => saveSettings());
el('settings-discard').addEventListener('click', () => {
  if (settingsView) {
    renderSettings(settingsView, { reset: true });
  }
});
el('settings-fields').addEventListener('input', () => renderSaveBar());
el('settings-fields').addEventListener('change', () => renderSaveBar());
el('settings-restart').addEventListener('click', async () => {
  const button = el('settings-restart');
  setBusy(button, true, 'Restarting…');
  try {
    await api('/api/desktop/restart', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-proview-desktop-token': desktopControl().token,
      },
    });
  } catch (error) {
    setSettingsStatus('Could not restart: ' + error.message, true);
    setBusy(button, false, '');
  }
});

const navLinks = [...document.querySelectorAll('.set-nav a')];
for (const link of navLinks) {
  link.addEventListener('click', (event) => {
    event.preventDefault();
    showSection(link.dataset.section, true);
  });
  link.addEventListener('keydown', (event) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
    if (!step) {
      return;
    }
    event.preventDefault();
    const next = navLinks[(navLinks.indexOf(link) + step + navLinks.length) % navLinks.length];
    showSection(next.dataset.section, false);
    next.focus();
  });
}

el('refresh').addEventListener('click', () => loadDevices());
el('banner-action').addEventListener('click', () => {
  if (bannerAction) {
    bannerAction();
  }
});
el('retry').addEventListener('click', () => requestStream());
el('check-password').addEventListener('click', () => {
  focusAfterSettings = (settingsView && settingsView.fields.find((field) => /DEVICE_PASS/i.test(field.key)) || { key: 'PROVIEW_DEVICE_PASS' }).key;
  openSettings('config');
});
el('switcher').addEventListener('click', () => toggleFloating(el('camera-menu'), el('switcher'), renderCameraMenu));
el('grid-toggle').addEventListener('click', () => setView(state.view === 'grid' ? 'live' : 'grid'));
el('lens-pick').addEventListener('click', () => toggleFloating(el('lens-menu'), el('lens-pick'), renderLensPicker));
el('quality-pick').addEventListener('click', () => toggleFloating(el('quality-menu'), el('quality-pick'), renderPrefsIntoForm));
for (const row of el('quality-menu').querySelectorAll('[data-quality]')) {
  row.addEventListener('click', () => {
    closeFloating(true);
    setQuality(row.dataset.quality);
  });
}
el('keys-close').addEventListener('click', () => closeFloating(false));
el('mute').addEventListener('click', toggleMute);
el('record').addEventListener('click', toggleRecord);
el('snapshot').addEventListener('click', captureSnapshot);
el('move').addEventListener('click', togglePad);
el('fullscreen').addEventListener('click', toggleFullscreen);
el('grid-fullscreen').addEventListener('click', toggleFullscreen);
el('light').addEventListener('click', toggleWhiteLight);
el('run-checks').addEventListener('click', runChecks);
for (const key of el('pad').querySelectorAll('.pk')) {
  key.addEventListener('click', () => sendControl(key.dataset.act));
}
el('tour').addEventListener('click', () => sendControl('cruise'));
document.querySelector('.skip').addEventListener('click', (event) => {
  event.preventDefault();
  const first = el('controls').querySelector('button:not(:disabled):not([hidden])');
  (first || el('controls')).focus();
});

const talkButton = el('talk');
talkButton.addEventListener('pointerdown', (event) => {
  if (event.button !== 0) {
    return;
  }
  talkButton.setPointerCapture(event.pointerId);
  startTalk();
});
for (const type of ['pointerup', 'pointercancel', 'lostpointercapture']) {
  talkButton.addEventListener(type, () => stopTalk());
}
talkButton.addEventListener('keydown', (event) => {
  if (event.key === ' ' || event.key === 'Enter') {
    event.preventDefault();
    if (!event.repeat) {
      startTalk();
    }
  }
});
talkButton.addEventListener('keyup', (event) => {
  if (event.key === ' ' || event.key === 'Enter') {
    event.preventDefault();
    stopTalk();
  }
});

for (const button of document.querySelectorAll('[data-theme-toggle]')) {
  button.addEventListener('click', () => {
    state.prefs.theme = resolveTheme() === 'dark' ? 'light' : 'dark';
    savePrefs();
    applyTheme();
  });
}

for (const name of ['theme', 'quality', 'lens']) {
  for (const radio of document.querySelectorAll('input[name="' + name + '"]')) {
    radio.addEventListener('change', () => {
      if (!radio.checked) {
        return;
      }
      if (name === 'quality') {
        setQuality(radio.value);
        return;
      }
      state.prefs[name] = radio.value;
      savePrefs();
      if (name === 'theme') {
        applyTheme();
      }
    });
  }
}

el('set-osd').addEventListener('change', () => {
  state.prefs.osd = el('set-osd').checked;
  savePrefs();
  renderFsLine();
});

el('set-muted').addEventListener('change', () => {
  state.prefs.muted = el('set-muted').checked;
  savePrefs();
  state.muted = state.prefs.muted;
  renderMute();
});

el('set-flip').addEventListener('change', () => {
  state.prefs.flip = el('set-flip').checked;
  savePrefs();
  layoutLensCrop();
});

window.addEventListener('hashchange', () => route(location.hash, true));

window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => {
  if (state.prefs.theme === 'system') {
    applyTheme();
  }
});

window.addEventListener('offline', () => {
  state.networkLostAt = Date.now();
  renderBanner();
  renderCaption();
});

window.addEventListener('online', () => {
  state.networkLostAt = null;
  renderBanner();
  renderCaption();
  if (state.screen === 'watch' && state.view === 'live' && state.stage.name !== 'playing') {
    requestStream();
  }
});

const video = el('video');
video.addEventListener('loadedmetadata', () => {
  if (!state.lens && state.selectedId) {
    const derived = deriveLensFromFrame();
    if (derived) {
      state.lens = derived;
      renderLensPicker();
      applyLensView(state.lensView);
    }
  }
  layoutLensCrop();
});

let lastVideoTime = -1;
video.addEventListener('timeupdate', () => {
  if (video.currentTime !== lastVideoTime) {
    lastVideoTime = video.currentTime;
    state.stage.lastFrameAt = Date.now();
  }
});

if (typeof ResizeObserver === 'function') {
  new ResizeObserver(() => layoutLensCrop()).observe(el('well'));
}

document.addEventListener('fullscreenchange', () => {
  layoutLensCrop();
  renderFsLine();
});

state.muted = state.prefs.muted;
state.lensView = state.prefs.lens;
renderMute();
renderPrefsIntoForm();
applyTheme();
renderControlAvailability();
tick();
signedIn();
