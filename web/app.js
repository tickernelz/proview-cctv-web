
const el = (id) => document.getElementById(id);
const enc = encodeURIComponent;

const VIEWS = ['live', 'grid', 'settings'];
const PREFS_KEY = 'pvw_prefs_v2';
const LIVE_WINDOW_SECONDS = 12;
const LIVE_TARGET_LAG_SECONDS = 1.0;

const state = {
  view: null,
  devices: [],
  selectedId: null,
  channel: 1,
  lens: null,
  lensSupport: 'unknown',
  capabilitySupport: 'unknown',
  control: { available: false, reason: 'Device controls have not been checked yet.' },
  players: new Map(),
  ticket: null,
  mode: null,
  recorder: null,
  recordChunks: [],
  muted: true,
  flipped: false,
  clock: null,
  prefs: loadPrefs(),
};

function defaultPrefs() {
  return { theme: 'system', quality: 'main', lens: 'combined', osd: true, muted: true };
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

function setBanner(message) {
  const banner = el('app-error');
  banner.textContent = message || '';
  banner.hidden = !message;
}

function setLoginError(message) {
  const box = el('login-error');
  box.textContent = message || '';
  box.hidden = !message;
}

function setLocalError(message) {
  const box = el('local-error');
  box.textContent = message || '';
  box.hidden = !message;
}

function setBusy(button, busy) {
  button.setAttribute('aria-busy', busy ? 'true' : 'false');
  button.disabled = busy;
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
    throw error;
  }
  return body;
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
  const button = el('theme-toggle');
  button.textContent = theme === 'dark' ? 'Dark' : 'Light';
  button.setAttribute('aria-label', theme === 'dark' ? 'Dark theme is on. Switch to light' : 'Light theme is on. Switch to dark');
  for (const radio of document.querySelectorAll('input[name="theme"]')) {
    radio.checked = radio.value === state.prefs.theme;
  }
}

function showLogin() {
  stopPlayback();
  stopGrid();
  stopClock();
  el('login-view').hidden = false;
  el('app-view').hidden = true;
  el('view-nav').hidden = true;
  el('account').hidden = true;
  el('signout').hidden = true;
  state.selectedId = null;
  setBanner('');
}

function showApp(account) {
  el('login-view').hidden = true;
  el('app-view').hidden = false;
  el('view-nav').hidden = false;
  el('account').textContent = account || '';
  el('account').title = account || '';
  el('account').hidden = !account;
  el('signout').hidden = false;
}

function setView(name, options = {}) {
  const next = VIEWS.includes(name) ? name : 'live';
  const changed = state.view !== next;
  state.view = next;
  for (const view of VIEWS) {
    el('view-' + view).hidden = view !== next;
  }
  for (const link of document.querySelectorAll('#view-nav a')) {
    if (link.dataset.view === next) {
      link.setAttribute('aria-current', 'page');
    } else {
      link.removeAttribute('aria-current');
    }
  }
  if (options.record !== false && location.hash !== '#' + next) {
    history.replaceState(null, '', '#' + next);
  }
  if (next === 'grid') {
    startGrid();
  } else {
    stopGrid();
  }
  if (changed && options.focus !== false && state.view) {
    el(next + '-title').focus({ preventScroll: true });
  }
}

function selectedDevice() {
  return state.devices.find((device) => device.id === state.selectedId) || null;
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

function modeName(mode) {
  if (mode === 'local') {
    return 'this network';
  }
  if (mode === 'cloud') {
    return 'cloud account';
  }
  return 'unknown';
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
  if (!state.mode) {
    status.textContent = 'The server did not report a provider mode.';
    notice.hidden = true;
    localMode.textContent = 'The server did not report which provider it uses.';
    setLocalFormEnabled(false);
    return;
  }
  status.textContent = 'Mode: ' + modeName(state.mode.mode) + ' · answering: ' + state.mode.source;
  if (state.mode.notice) {
    notice.textContent = 'Fallback: ' + (state.mode.notice.message || 'this network is not reachable, so the other provider answered.');
    notice.hidden = false;
  } else {
    notice.hidden = true;
  }
  for (const radio of document.querySelectorAll('input[name="mode"]')) {
    radio.checked = radio.value === state.mode.mode;
  }
  localMode.textContent =
    state.mode.mode === 'local'
      ? 'The server is set to this network, so the camera login below is the one to use.'
      : 'The server is set to the ' + modeName(state.mode.mode) + '. Switch it under Settings → Connection once you are signed in, then come back to this tab.';
  setLocalFormEnabled(state.mode.mode === 'local');
  renderSourceBadge();
}

function setLocalFormEnabled(enabled) {
  for (const id of ['local-user', 'local-pass', 'local-submit']) {
    el(id).disabled = !enabled;
  }
}

async function setMode(mode) {
  try {
    state.mode = await api('/api/config/mode', { method: 'POST', body: JSON.stringify({ mode }) });
    renderMode();
    await loadDevices();
    if (state.selectedId) {
      await loadLens(state.selectedId, state.channel);
      await requestStream();
    }
  } catch (error) {
    setBanner('The server did not change provider: ' + error.message);
    await loadMode();
  }
}

function renderDevices() {
  const list = el('devices');
  list.replaceChildren();
  for (const device of state.devices) {
    const selected = device.id === state.selectedId;
    const item = document.createElement('li');
    item.className = 'device';
    item.dataset.selected = selected ? 'true' : 'false';

    const dot = document.createElement('span');
    dot.className = 'status-dot';
    dot.dataset.online = device.online ? 'true' : 'false';
    dot.setAttribute('role', 'img');
    dot.setAttribute('aria-label', device.online ? 'Online' : 'Offline');

    const body = document.createElement('div');
    body.className = 'device-body';
    const name = document.createElement('span');
    name.className = 'device-name';
    name.textContent = device.name;
    name.title = device.name;
    const meta = document.createElement('span');
    meta.className = 'device-meta';
    meta.textContent = [device.channels + ' ch', sourceLabel()].filter(Boolean).join(' · ');
    const id = document.createElement('span');
    id.className = 'device-id';
    id.textContent = device.id;
    if (device.model) {
      item.title = device.model;
    }
    body.append(name, meta, id);

    const action = document.createElement('button');
    action.type = 'button';
    action.className = 'ghost compact';
    action.textContent = 'Watch';
    action.setAttribute('aria-pressed', String(selected));
    action.setAttribute('aria-label', selected ? 'Watching ' + device.name : 'Watch ' + device.name);
    action.addEventListener('click', () => {
      selectDevice(device.id);
      if (state.view !== 'live') {
        setView('live');
      }
    });

    item.append(dot, body, action);
    list.append(item);
  }
  el('devices-empty').hidden = state.devices.length > 0;
}

function fillChannels(device) {
  const select = el('channel');
  select.replaceChildren();
  const count = Math.max(1, device ? device.channels || 1 : 1);
  for (let index = 1; index <= count; index += 1) {
    const option = document.createElement('option');
    option.value = String(index);
    option.textContent = count === 1 ? '1 (only channel)' : 'Channel ' + index;
    select.append(option);
  }
  select.value = String(Math.min(state.channel, count));
  select.closest('.channel-picker').hidden = count < 2;
}

function setStage(stateName, message) {
  const stage = el('stage');
  stage.dataset.state = stateName;
  el('stage-status').textContent = message || '';
  el('stage-status').hidden = stateName === 'playing';
  el('lens-crop').hidden = stateName !== 'playing';
  const playing = stateName === 'playing';
  el('record').disabled = !playing;
  el('snapshot').disabled = !playing;
  el('flip').disabled = !playing;
  renderOsd();
  layoutLensCrop();
}

function explainStreamError(error) {
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
  }

  destroy() {
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
      video.play().catch(() => {});
      this.onState('playing', '');
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
      await video.play().catch(() => {});
      this.onState('playing', '');
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
      await video.play().catch(() => {});
      this.onState('playing', '');
      return;
    }
    if (video.canPlayType('application/vnd.apple.mpegurl')) {
      video.src = ticket.url;
      await video.play().catch(() => {});
      this.onState('playing', '');
      return;
    }
    this.onState('error', 'This browser has no HLS support and no hls.js is loaded.');
  }
}

function stagePlayer() {
  let player = state.players.get('stage');
  if (!player) {
    player = new StreamPlayer(el('video'), (name, message) => {
      setStage(name, message);
      el('source-badge').dataset.source = name === 'playing' ? 'live' : name;
      if (name === 'error') {
        el('source-label').textContent = 'Stream failed';
      }
      if (name === 'playing') {
        renderSourceBadge();
      }
    });
    state.players.set('stage', player);
  }
  return player;
}

function stopPlayback() {
  const player = state.players.get('stage');
  if (player) {
    player.destroy();
    state.players.delete('stage');
  }
  stopRecording();
  state.lens = null;
  state.ticket = null;
  el('osd-name').hidden = true;
  el('osd-time').hidden = true;
  el('source-badge').dataset.source = 'idle';
  el('source-label').textContent = 'No stream';
}

function stopClock() {
  if (state.clock) {
    window.clearInterval(state.clock);
    state.clock = null;
  }
}

function startClock() {
  stopClock();
  const tick = () => {
    const now = new Date();
    el('osd-time').textContent = now.toLocaleTimeString([], { hour12: false });
  };
  tick();
  state.clock = window.setInterval(tick, 1000);
}

function renderSourceBadge() {
  const device = selectedDevice();
  const badge = el('source-badge');
  const ticket = state.ticket;
  if (!device || !state.selectedId) {
    badge.dataset.source = 'idle';
    el('source-label').textContent = 'No stream';
    return;
  }
  const transport = ticket ? ticket.kind.toUpperCase() : 'ticket pending';
  const quality = state.prefs.quality === 'sub' ? 'SD' : 'HD';
  badge.dataset.source = ticket ? 'live' : 'idle';
  el('source-label').textContent = [transport, quality, 'ch ' + state.channel, sourceLabel()].join(' · ');
  badge.setAttribute('aria-label', 'Stream source: ' + el('source-label').textContent);
  badge.title = [
    'device ' + device.id,
    ticket ? 'endpoint ' + ticket.url : 'no ticket yet',
    ticket ? 'expires in ' + ticket.expires_in + 's' : '',
    state.mode ? 'mode ' + modeName(state.mode.mode) + ' · answering ' + state.mode.source : 'mode unknown',
    state.mode && state.mode.notice ? 'fallback: ' + state.mode.notice.message : '',
    device.lan_host ? 'lan ' + device.lan_host : '',
  ]
    .filter(Boolean)
    .join(' | ');
}

function renderOsd() {
  const device = selectedDevice();
  const show = state.prefs.osd && Boolean(device) && el('stage').dataset.state === 'playing';
  el('osd-name').textContent = device ? device.name : '';
  el('osd-name').hidden = !show;
  el('osd-time').hidden = !show;
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
    lenses: lenses.map((lens, index) => ({
      index: typeof lens.index === 'number' ? lens.index : index,
      label: lensLabel(lens, index),
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
    '.'
  );
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
    console.error('lens contract failed', error);
    el('lens-source').textContent = 'Lens layout: the server answered ' + error.status + ' for GET /api/lens.';
  }
}

function lensEntries() {
  return state.lens && state.lens.lenses.length ? state.lens.lenses : [];
}

function renderLensPicker() {
  const entries = lensEntries();
  const upper = el('seg-upper');
  const lower = el('seg-lower');
  const upperLabel = upper.querySelector('span');
  const lowerLabel = lower.querySelector('span');
  const hasPair = entries.length > 1;
  upper.hidden = !hasPair;
  lower.hidden = !hasPair;
  upper.querySelector('input').disabled = !hasPair;
  lower.querySelector('input').disabled = !hasPair;

  if (hasPair) {
    const [first, second] = entries;
    const suffix = (entry) => (entry.role === 'moving' ? ' (moving)' : entry.role === 'fixed' ? ' (fixed)' : '');
    upperLabel.textContent = first.label + suffix(first);
    lowerLabel.textContent = second.label + suffix(second);
    upper.querySelector('input').value = String(first.index);
    lower.querySelector('input').value = String(second.index);
  } else if (entries.length === 1) {
    upper.hidden = entries[0].crop.h >= 0.99;
    lower.hidden = true;
  }

  const note = el('lens-source');
  if (!state.selectedId) {
    note.textContent = 'Lens layout: pick a device.';
  } else if (!state.lens) {
    note.textContent = 'Lens layout: waiting for the first frame.';
  } else if (state.lens.source === 'contract') {
    note.textContent = contractCaption(state.lens);
  } else {
    const dims = state.lens.streams[0].width + '×' + state.lens.streams[0].height;
    note.textContent =
      state.lens.lenses.length > 1
        ? 'Lens layout: inferred from the frame (' + dims + '), two lenses stacked. The server does not report lens roles yet.'
        : 'Lens layout: single frame (' + dims + '), no stacked lens.';
  }

  const rolesKnown = entries.some((entry) => entry.role !== 'unknown');
  el('lens-roles-note').hidden = rolesKnown;

  const radios = document.querySelectorAll('input[name="lensview"]');
  const current = state.lensView;
  let matched = false;
  for (const radio of radios) {
    if (radio.value === current && !radio.closest('label').hidden) {
      radio.checked = true;
      matched = true;
    }
  }
  if (!matched) {
    radios[0].checked = true;
    applyLensView('combined');
  }
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
  const stage = el('stage');
  const cropEl = el('lens-crop');
  const dims = frameDims();
  const crop = currentCrop();
  const ratio = (crop.w * dims.width) / (crop.h * dims.height);
  const fullscreen = document.fullscreenElement === stage;
  const shell = el('stage-shell');
  const availableWidth = (fullscreen ? window.innerWidth : shell.parentElement.clientWidth) || 1;
  const availableHeight = Math.max(220, Math.round(fullscreen ? window.innerHeight : window.innerHeight * 0.66));
  const playing = stage.dataset.state === 'playing';
  const height = playing ? Math.max(1, Math.round(Math.min(availableHeight, availableWidth / ratio))) : 260;
  const width = Math.max(1, Math.round(Math.min(availableWidth, height * ratio)));
  cropEl.style.width = fullscreen ? '100%' : width + 'px';
  cropEl.style.height = fullscreen ? '100%' : height + 'px';
  shell.style.width = '100%';
  stage.style.width = '100%';
  stage.style.height = fullscreen ? '100%' : height + 'px';
}

function applyLensView(value) {
  state.lensView = value;
  const video = el('video');
  const crop = currentCrop();
  video.style.left = (-crop.x / crop.w) * 100 + '%';
  video.style.top = (-crop.y / crop.h) * 100 + '%';
  video.style.width = 100 / crop.w + '%';
  video.style.height = 100 / crop.h + '%';
  el('stage').dataset.lens = value;
  layoutLensCrop();
}

function currentCrop() {
  if (state.lensView === 'combined') {
    return { x: 0, y: 0, w: 1, h: 1 };
  }
  const entry = lensEntries().find((lens) => String(lens.index) === String(state.lensView));
  return entry ? entry.crop : { x: 0, y: 0, w: 1, h: 1 };
}

function renderControlAvailability() {
  const available = state.control.available;
  const note = el('device-note');
  note.textContent = available
    ? state.control.summary || 'Device controls are reported by the server.'
    : state.control.reason;
  note.title = available ? '' : state.control.detail || state.control.reason;
  for (const button of document.querySelectorAll('[data-requires="device"]')) {
    button.disabled = !available;
    if (!available) {
      button.setAttribute('aria-describedby', 'device-note');
    }
  }
}

async function probeControlChannel(device, channel) {
  if (state.capabilitySupport === 'absent') {
    renderControlAvailability();
    return;
  }
  try {
    const payload = await api('/api/device/capabilities?device=' + enc(device.id) + '&channel=' + enc(channel));
    state.capabilitySupport = 'present';
    const controls = Array.isArray(payload && payload.controls) ? payload.controls : [];
    state.control = {
      available: true,
      controls,
      summary: 'Device controls: ' + (controls.length ? controls.map((entry) => entry.label || entry.id).join(', ') : 'the server lists none for this channel') + '.',
    };
  } catch (error) {
    if (error.status === 404 || error.status === 501) {
      state.capabilitySupport = 'absent';
      state.control = {
        available: false,
        reason:
          'Device controls need the control channel; the server answered ' +
          (error.message || error.status) +
          ', so these stay disabled.',
        detail:
          'GET /api/device/capabilities returned ' +
          (error.message || error.status) +
          '. Pan, presets, cruise, strobe, alarm, intercom, cloud storage and the SD album are disabled until the server exposes that channel.',
      };
    } else {
      state.control = { available: false, reason: 'Device controls could not be checked: ' + error.status + ' ' + error.message };
    }
  }
  renderControlAvailability();
}

async function sendControl(action, params) {
  if (!state.control.available) {
    return;
  }
  const device = selectedDevice();
  if (!device) {
    return;
  }
  try {
    await api('/api/device/control', {
      method: 'POST',
      body: JSON.stringify({ device: device.id, channel: state.channel, action, params: params || {} }),
    });
    setBanner('');
  } catch (error) {
    setBanner('The device did not confirm "' + action + '": ' + error.message);
  }
}

async function selectDevice(deviceId, options = {}) {
  const device = state.devices.find((entry) => entry.id === deviceId);
  if (!device) {
    return;
  }
  const deviceChanged = state.selectedId !== deviceId;
  state.selectedId = deviceId;
  state.channel = options.channel || (deviceChanged ? 1 : state.channel);
  if (deviceChanged) {
    state.lensView = state.prefs.lens;
  }
  renderDevices();
  fillChannels(device);
  setStage('loading', 'Requesting stream…');
  el('osd-name').textContent = device.name;
  startClock();
  await loadLens(device.id, state.channel);
  await requestStream();
  await probeControlChannel(device, state.channel);
}

async function requestStream() {
  const device = selectedDevice();
  if (!device) {
    return;
  }
  const player = stagePlayer();
  player.destroy();
  state.ticket = null;
  setStage('loading', 'Requesting stream…');
  el('source-badge').dataset.source = 'loading';
  el('source-label').textContent = 'Requesting…';
  try {
    const ticket = await api(
      '/api/stream?device=' +
        enc(device.id) +
        '&channel=' +
        enc(state.channel) +
        '&quality=' +
        enc(state.prefs.quality),
    );
    state.ticket = ticket;
    renderSourceBadge();
    await player.play(ticket);
  } catch (error) {
    setStage('error', explainStreamError(error));
    el('stage-status').title = error.message;
    el('source-badge').dataset.source = 'error';
    el('source-label').textContent = 'Stream failed';
    console.error('stream request failed', error);
  }
}

async function loadDevices() {
  setBanner('');
  el('devices-skeleton').hidden = false;
  el('devices').replaceChildren();
  el('devices-empty').hidden = true;
  try {
    state.devices = await api('/api/devices');
    renderDevices();
    if (!state.selectedId && state.devices.length) {
      await selectDevice(state.devices[0].id);
    }
  } catch (error) {
    if (error.status === 401) {
      showLogin();
      setLoginError('Session expired. Sign in again.');
      return;
    }
    setBanner('Could not load devices: ' + error.message);
  } finally {
    el('devices-skeleton').hidden = true;
  }
}

async function signedIn() {
  try {
    const me = await api('/api/me');
    showApp(me.account);
    await loadMode();
    await loadDevices();
    setView(location.hash.replace('#', '') || 'live', { focus: false, record: false });
  } catch {
    showLogin();
    await loadMode();
  }
}

function renderMute() {
  el('video').muted = state.muted;
  el('mute').setAttribute('aria-pressed', String(state.muted));
  el('mute').querySelector('use').setAttribute('href', state.muted ? '#i-speaker-off' : '#i-speaker');
}

function toggleMute() {
  state.muted = !state.muted;
  renderMute();
}

function toggleFlip() {
  state.flipped = !state.flipped;
  el('lens-crop').dataset.flipped = state.flipped ? 'true' : 'false';
  el('flip').setAttribute('aria-pressed', String(state.flipped));
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
  if (!device || el('stage').dataset.state !== 'playing' || !video.videoWidth) {
    setStage('error', 'There is no frame to capture yet.');
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
  if (state.flipped) {
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

function toggleRecord() {
  const video = el('video');
  if (state.recorder) {
    state.recorder.stop();
    return;
  }
  if (el('stage').dataset.state !== 'playing' || typeof video.captureStream !== 'function') {
    setStage('error', 'Recording needs a playing picture and a browser with captureStream.');
    return;
  }
  const stream = video.captureStream();
  if (!stream || stream.getVideoTracks().length === 0) {
    setStage('error', 'The browser exposed no video track to record.');
    return;
  }
  const candidates = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'];
  const mimeType = candidates.find((type) => window.MediaRecorder && MediaRecorder.isTypeSupported(type));
  if (!mimeType) {
    setStage('error', 'This browser cannot record to webm.');
    return;
  }
  state.recordChunks = [];
  const recorder = new MediaRecorder(stream, { mimeType });
  state.recorder = recorder;
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
    el('record').setAttribute('aria-pressed', 'false');
    el('record').querySelector('span').textContent = 'Record';
    if (blob.size) {
      download(blob, (device ? device.id : 'camera') + '_ch' + state.channel + '_' + stamp() + '.webm');
    }
    for (const track of stream.getTracks()) {
      track.stop();
    }
  });
  recorder.start();
  el('record').setAttribute('aria-pressed', 'true');
  el('record').querySelector('span').textContent = 'Stop';
}

function stopGrid() {
  for (const [id, player] of state.players) {
    if (id.startsWith('grid:')) {
      player.destroy();
      state.players.delete(id);
    }
  }
}

async function startGrid() {
  stopGrid();
  const grid = el('grid');
  grid.replaceChildren();
  const device = selectedDevice();
  if (!device) {
    el('grid-empty').hidden = false;
    el('grid-empty').textContent = 'Pick a device on the left, then this view fills with one tile per channel.';
    el('grid-summary').textContent = 'No device selected.';
    return;
  }
  const count = Math.max(1, device.channels || 1);
  el('grid-empty').hidden = count > 1;
  if (count < 2) {
    el('grid-empty').textContent = 'This device reports one channel, so there is nothing to tile. Multi-channel devices fill this view with one tile per channel.';
  }
  el('grid-summary').textContent = device.name + ' · ' + count + ' channel' + (count === 1 ? '' : 's') + ' · ' + (state.prefs.quality === 'sub' ? 'SD' : 'HD');
  grid.dataset.count = String(count);

  for (let channel = 1; channel <= count; channel += 1) {
    const tile = document.createElement('article');
    tile.className = 'tile';
    tile.dataset.state = 'loading';

    const head = document.createElement('header');
    head.className = 'tile-head';
    const open = document.createElement('button');
    open.type = 'button';
    open.className = 'tile-open';
    open.textContent = 'Channel ' + channel;
    open.addEventListener('click', async () => {
      await selectDevice(device.id, { channel });
      setView('live');
    });
    const chip = document.createElement('span');
    chip.className = 'tile-chip';
    chip.textContent = state.prefs.quality === 'sub' ? 'SD' : 'HD';
    head.append(open, chip);

    const box = document.createElement('div');
    box.className = 'tile-stage';
    const video = document.createElement('video');
    video.playsInline = true;
    video.muted = true;
    const status = document.createElement('p');
    status.className = 'tile-status';
    status.textContent = 'Requesting stream…';
    box.append(video, status);

    tile.append(head, box);
    grid.append(tile);

    const player = new StreamPlayer(video, (name, message) => {
      tile.dataset.state = name;
      status.textContent = name === 'playing' ? '' : message;
      status.hidden = name === 'playing';
    });
    state.players.set('grid:' + channel, player);

    try {
      const ticket = await api(
        '/api/stream?device=' + enc(device.id) + '&channel=' + channel + '&quality=' + enc(state.prefs.quality),
      );
      await player.play(ticket);
    } catch (error) {
      tile.dataset.state = 'error';
      status.textContent = shortStreamError(error);
      status.title = error.message;
      console.error('grid stream failed', error);
    }
  }
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

function renderPrefsIntoForm() {
  for (const radio of document.querySelectorAll('input[name="theme"]')) {
    radio.checked = radio.value === state.prefs.theme;
  }
  for (const radio of document.querySelectorAll('input[name="quality"]')) {
    radio.checked = radio.value === state.prefs.quality;
  }
  for (const radio of document.querySelectorAll('input[name="lens"]')) {
    radio.checked = radio.value === state.prefs.lens;
  }
  for (const radio of document.querySelectorAll('input[name="qualitybar"]')) {
    radio.checked = radio.value === state.prefs.quality;
  }
  el('set-osd').checked = state.prefs.osd;
  el('set-muted').checked = state.prefs.muted;
}

function switchSignInMode(mode) {
  const cloud = mode !== 'local';
  el('tab-cloud').setAttribute('aria-selected', String(cloud));
  el('tab-local').setAttribute('aria-selected', String(!cloud));
  el('tab-cloud').tabIndex = cloud ? 0 : -1;
  el('tab-local').tabIndex = cloud ? -1 : 0;
  el('panel-cloud').hidden = !cloud;
  el('panel-local').hidden = cloud;
  (cloud ? el('panel-cloud') : el('panel-local')).focus();
}

async function runChecks() {
  const button = el('run-checks');
  setBusy(button, true);
  const write = (id, text, ok) => {
    const cell = el(id);
    cell.textContent = text;
    cell.dataset.ok = ok === undefined ? '' : String(ok);
  };
  write('check-health', 'Checking…');
  write('check-account', 'Checking…');
  write('check-probe', 'Checking…');
  write('check-ticket', 'Checking…');
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
  const device = selectedDevice();
  if (!device) {
    write('check-ticket', 'no device selected', undefined);
  } else {
    try {
      const ticket = await api('/api/stream?device=' + enc(device.id) + '&channel=' + enc(state.channel) + '&quality=' + enc(state.prefs.quality));
      write('check-ticket', ticket.kind + ' · ' + ticket.url, true);
    } catch (error) {
      write('check-ticket', error.status + ' ' + error.message, false);
    }
  }
  setBusy(button, false);
}

el('login-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  setLoginError('');
  const button = el('login-submit');
  setBusy(button, true);
  try {
    await api('/api/login', {
      method: 'POST',
      body: JSON.stringify({ email: el('email').value, password: el('password').value }),
    });
    el('password').value = '';
    await signedIn();
  } catch (error) {
    setLoginError(
      error.status === 401 ? 'That email and password were not accepted.' : 'Sign in failed: ' + error.message,
    );
  } finally {
    setBusy(button, false);
  }
});

el('tab-cloud').addEventListener('click', () => switchSignInMode('cloud'));
el('tab-local').addEventListener('click', () => switchSignInMode('local'));
el('tab-cloud').addEventListener('keydown', (event) => {
  if (event.key === 'ArrowRight') {
    switchSignInMode('local');
    el('tab-local').focus();
  }
});
el('tab-local').addEventListener('keydown', (event) => {
  if (event.key === 'ArrowLeft') {
    switchSignInMode('cloud');
    el('tab-cloud').focus();
  }
});

el('local-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  setLocalError('');
  const button = el('local-submit');
  setBusy(button, true);
  try {
    await api('/api/login', {
      method: 'POST',
      body: JSON.stringify({ email: el('local-user').value, password: el('local-pass').value }),
    });
    el('local-pass').value = '';
    await signedIn();
  } catch (error) {
    setLocalError(
      error.status === 401
        ? 'The camera did not accept that username and password.'
        : 'The camera login failed: ' + error.message,
    );
  } finally {
    setBusy(button, false);
  }
});

for (const radio of document.querySelectorAll('input[name="mode"]')) {
  radio.addEventListener('change', () => {
    if (radio.checked) {
      setMode(radio.value);
    }
  });
}

el('local-connect').addEventListener('click', async () => {
  const button = el('local-connect');
  setLocalError('');
  el('local-report').hidden = true;
  el('local-next').hidden = true;
  setBusy(button, true);
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
    if (error.status === 400 && /PROVIEW_DEVICE_HOST/.test(error.message)) {
      setLocalError(
        'This server has no camera address configured. Set PROVIEW_DEVICE_HOST (and the device login) in the server environment, restart it, then probe again.',
      );
    } else {
      setLocalError('The camera on this network did not answer: ' + error.message);
    }
  } finally {
    setBusy(button, false);
  }
});

el('signout').addEventListener('click', async () => {
  try {
    await api('/api/logout', { method: 'POST' });
  } catch {}
  state.devices = [];
  state.lens = null;
  el('devices').replaceChildren();
  showLogin();
  await loadMode();
});

el('refresh').addEventListener('click', () => {
  loadDevices();
});

el('channel').addEventListener('change', async () => {
  state.channel = Number(el('channel').value) || 1;
  await loadLens(state.selectedId, state.channel);
  await requestStream();
});

for (const radio of document.querySelectorAll('input[name="qualitybar"]')) {
  radio.addEventListener('change', async () => {
    if (!radio.checked) {
      return;
    }
    state.prefs.quality = radio.value;
    savePrefs();
    renderPrefsIntoForm();
    await requestStream();
  });
}

for (const radio of document.querySelectorAll('input[name="lensview"]')) {
  radio.addEventListener('change', () => {
    if (!radio.checked) {
      return;
    }
    applyLensView(radio.value);
    if (state.view === 'grid') {
      startGrid();
    }
  });
}

el('mute').addEventListener('click', toggleMute);
el('record').addEventListener('click', toggleRecord);
el('snapshot').addEventListener('click', captureSnapshot);
el('flip').addEventListener('click', toggleFlip);

el('fullscreen').addEventListener('click', async () => {
  const stage = el('stage');
  if (document.fullscreenElement) {
    await document.exitFullscreen();
  } else {
    await stage.requestFullscreen().catch(() => setStage('error', 'Fullscreen was refused by the browser.'));
  }
});

for (const button of document.querySelectorAll('[data-requires="device"]')) {
  button.addEventListener('click', () => {
    sendControl(button.dataset.act || 'unknown');
  });
}

el('theme-toggle').addEventListener('click', () => {
  state.prefs.theme = resolveTheme() === 'dark' ? 'light' : 'dark';
  savePrefs();
  applyTheme();
});

for (const radio of document.querySelectorAll('input[name="theme"]')) {
  radio.addEventListener('change', () => {
    if (!radio.checked) {
      return;
    }
    state.prefs.theme = radio.value;
    savePrefs();
    applyTheme();
  });
}

for (const radio of document.querySelectorAll('input[name="quality"]')) {
  radio.addEventListener('change', () => {
    if (!radio.checked) {
      return;
    }
    state.prefs.quality = radio.value;
    savePrefs();
    renderPrefsIntoForm();
  });
}

for (const radio of document.querySelectorAll('input[name="lens"]')) {
  radio.addEventListener('change', () => {
    if (!radio.checked) {
      return;
    }
    state.prefs.lens = radio.value;
    savePrefs();
  });
}

el('set-osd').addEventListener('change', () => {
  state.prefs.osd = el('set-osd').checked;
  savePrefs();
  renderOsd();
});

el('set-muted').addEventListener('change', () => {
  state.prefs.muted = el('set-muted').checked;
  savePrefs();
  state.muted = state.prefs.muted;
  renderMute();
});

el('run-checks').addEventListener('click', runChecks);

window.addEventListener('hashchange', () => {
  if (el('app-view').hidden) {
    return;
  }
  setView(location.hash.replace('#', '') || 'live');
});

window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => {
  if (state.prefs.theme === 'system') {
    applyTheme();
  }
});

el('video').addEventListener('loadedmetadata', () => {
  if (!state.lens && state.selectedId) {
    const derived = deriveLensFromFrame();
    if (derived) {
      state.lens = derived;
      renderLensPicker();
      applyLensView(state.lensView);
    }
  }
  renderOsd();
  layoutLensCrop();
});

if (typeof ResizeObserver === 'function') {
  new ResizeObserver(() => layoutLensCrop()).observe(el('stage-shell').parentElement);
}
document.addEventListener('fullscreenchange', () => layoutLensCrop());

el('video').addEventListener('playing', () => {
  renderOsd();
});

state.muted = state.prefs.muted;
state.lensView = state.prefs.lens;
renderMute();
renderPrefsIntoForm();
applyTheme();
renderControlAvailability();
loadMode();
signedIn();
