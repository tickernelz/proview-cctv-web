const state = {
  devices: [],
  selected: null,
  hls: null,
  busy: false,
};

const el = (id) => document.getElementById(id);

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

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem('pvw_theme', theme);
  } catch {}
}

function initTheme() {
  let stored = null;
  try {
    stored = localStorage.getItem('pvw_theme');
  } catch {}
  if (!stored) {
    stored = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  applyTheme(stored);
  el('theme-toggle').addEventListener('click', () => {
    applyTheme(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark');
  });
}

function showLogin() {
  el('login-view').hidden = false;
  el('app-view').hidden = true;
  el('account').hidden = true;
  el('signout').hidden = true;
}

function showApp(account) {
  el('login-view').hidden = true;
  el('app-view').hidden = false;
  el('account').textContent = account || '';
  el('account').hidden = !account;
  el('signout').hidden = false;
}

function renderDevices() {
  const list = el('devices');
  list.replaceChildren();
  for (const device of state.devices) {
    const item = document.createElement('li');
    item.className = 'device';
    item.dataset.selected = state.selected && state.selected.id === device.id ? 'true' : 'false';

    const dot = document.createElement('span');
    dot.className = 'status-dot';
    dot.dataset.online = device.online ? 'true' : 'false';
    dot.setAttribute('role', 'img');
    dot.setAttribute('aria-label', device.online ? 'Online' : 'Offline');

    const body = document.createElement('div');
    const name = document.createElement('div');
    name.className = 'device-name';
    name.textContent = device.name;
    const meta = document.createElement('div');
    meta.className = 'device-meta';
    meta.textContent = [device.model, device.channels + ' ch', device.id].filter(Boolean).join(' · ');
    body.append(name, meta);

    const action = document.createElement('button');
    action.type = 'button';
    action.className = 'ghost';
    action.textContent = state.selected && state.selected.id === device.id ? 'Watching' : 'Watch';
    action.addEventListener('click', () => selectDevice(device));

    item.append(dot, body, action);
    list.append(item);
  }
  el('devices-empty').hidden = state.devices.length > 0;
}

function fillChannels(device) {
  const select = el('channel');
  select.replaceChildren();
  const count = Math.max(1, device.channels || 1);
  for (let index = 1; index <= count; index += 1) {
    const option = document.createElement('option');
    option.value = String(index);
    option.textContent = String(index);
    select.append(option);
  }
}

function setStage(stateName, message) {
  const stage = el('stage-status').parentElement;
  stage.dataset.state = stateName;
  el('stage-status').textContent = message;
  el('stage-status').hidden = stateName === 'playing';
  el('video').hidden = stateName !== 'playing';
}

function stopPlayback() {
  stopLive();
  if (state.hls) {
    state.hls.destroy();
    state.hls = null;
  }
  const video = el('video');
  video.pause();
  video.removeAttribute('src');
  video.load();
}

async function selectDevice(device) {
  state.selected = device;
  renderDevices();
  fillChannels(device);
  el('player').hidden = false;
  el('player-title').textContent = 'Live · ' + device.name;
  await requestStream();
}

async function requestStream() {
  const device = state.selected;
  if (!device) {
    return;
  }
  stopPlayback();
  setStage('loading', 'Requesting stream…');
  try {
    const channel = el('channel').value || '1';
    const quality = el('quality').value;
    const ticket = await api(
      '/api/stream?device=' + encodeURIComponent(device.id) + '&channel=' + encodeURIComponent(channel) + '&quality=' + encodeURIComponent(quality),
    );
    await play(ticket);
  } catch (error) {
    setStage('error', explainStreamError(error));
    el('stage-status').title = error.message;
  }
}

function explainStreamError(error) {
  console.error('stream request failed', error);
  if (error.status === 501) {
    return 'Live video is not wired yet: the device transport is still being built, so there is no frame to show.';
  }
  if (error.status === 404) {
    return 'That channel does not exist on this device.';
  }
  if (error.status === 401) {
    return 'Session expired. Sign in again.';
  }
  if (error.status === 502) {
    return 'The stream source did not answer with a playable stream.';
  }
  return 'The stream could not be started.';
}

const live = { socket: null, media: null, buffer: null, queue: [], url: null };

function stopLive() {
  if (live.socket) {
    live.socket.onmessage = null;
    live.socket.close();
    live.socket = null;
  }
  if (live.buffer && live.media && live.media.readyState === 'open') {
    try {
      live.media.removeSourceBuffer(live.buffer);
    } catch {}
  }
  if (live.url) {
    URL.revokeObjectURL(live.url);
    live.url = null;
  }
  live.buffer = null;
  live.media = null;
  live.queue = [];
}

function pumpFragments() {
  const buffer = live.buffer;
  if (!buffer || buffer.updating) {
    return;
  }
  const next = live.queue.shift();
  if (!next) {
    return;
  }
  try {
    buffer.appendBuffer(next);
  } catch (error) {
    console.error('append failed, dropping to the next keyframe', error);
    live.queue = [];
  }
}

function playOverWebSocket(ticket) {
  const video = el('video');
  stopLive();
  const media = new MediaSource();
  live.media = media;
  live.url = URL.createObjectURL(media);
  video.src = live.url;

  media.addEventListener('sourceopen', () => {
    const buffer = media.addSourceBuffer('video/mp4; codecs="avc1.42E01E"');
    live.buffer = buffer;
    buffer.addEventListener('updateend', pumpFragments);

    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    const target = ticket.url.startsWith('/') ? scheme + '://' + location.host + ticket.url : ticket.url.replace(/^http/, 'ws');
    const socket = new WebSocket(target);
    socket.binaryType = 'arraybuffer';
    live.socket = socket;
    socket.onmessage = (event) => {
      live.queue.push(new Uint8Array(event.data));
      pumpFragments();
    };
    socket.onerror = () => setStage('error', 'The live socket failed to connect.');
    socket.onclose = () => {
      if (!video.paused) setStage('error', 'Live socket closed.');
    };
    video.play().catch(() => {});
    setStage('playing', '');
  }, { once: true });
}

async function play(ticket) {
  const video = el('video');
  if (ticket.kind === 'ws') {
    if (!window.MediaSource) {
      setStage('error', 'This browser cannot play the live socket (no MediaSource).');
      return;
    }
    playOverWebSocket(ticket);
    return;
  }
  if (ticket.kind === 'mp4' || ticket.url.endsWith('.mp4')) {
    video.src = ticket.url;
    await video.play().catch(() => {});
    setStage('playing', '');
    return;
  }
  if (ticket.kind !== 'hls') {
    setStage('error', 'Unsupported stream kind: ' + ticket.kind);
    return;
  }
  if (window.Hls && window.Hls.isSupported()) {
    const hls = new window.Hls({ lowLatencyMode: true });
    hls.on(window.Hls.Events.ERROR, (_event, data) => {
      if (data && data.fatal) {
        setStage('error', 'Playlist could not be loaded: ' + data.type);
      }
    });
    hls.loadSource(ticket.url);
    hls.attachMedia(video);
    state.hls = hls;
    await video.play().catch(() => {});
    setStage('playing', '');
    return;
  }
  if (video.canPlayType('application/vnd.apple.mpegurl')) {
    video.src = ticket.url;
    await video.play().catch(() => {});
    setStage('playing', '');
    return;
  }
  setStage('error', 'This browser has no HLS support and no hls.js is loaded.');
}

async function loadDevices() {
  setBanner('');
  el('devices-skeleton').hidden = false;
  el('devices').replaceChildren();
  el('devices-empty').hidden = true;
  try {
    state.devices = await api('/api/devices');
    renderDevices();
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
    await loadDevices();
  } catch {
    showLogin();
  }
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

el('signout').addEventListener('click', async () => {
  try {
    await api('/api/logout', { method: 'POST' });
  } catch {}
  stopPlayback();
  state.selected = null;
  state.devices = [];
  el('player').hidden = true;
  showLogin();
});

el('refresh').addEventListener('click', () => {
  loadDevices();
});

el('channel').addEventListener('change', () => {
  if (state.selected) {
    requestStream();
  }
});

el('quality').addEventListener('change', () => {
  if (state.selected) {
    requestStream();
  }
});

el('snapshot').addEventListener('click', () => {
  const video = el('video');
  if (!state.selected || video.hidden || !video.videoWidth) {
    setStage('error', 'No frame to capture yet.');
    return;
  }
  const canvas = document.createElement('canvas');
  canvas.width = video.videoWidth;
  canvas.height = video.videoHeight;
  canvas.getContext('2d').drawImage(video, 0, 0);
  const link = document.createElement('a');
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  link.download = state.selected.id + '_' + stamp + '.png';
  link.href = canvas.toDataURL('image/png');
  link.click();
});

el('fullscreen').addEventListener('click', async () => {
  const stage = el('stage-status').parentElement;
  if (document.fullscreenElement) {
    await document.exitFullscreen();
  } else {
    await stage.requestFullscreen().catch(() => setStage('error', 'Fullscreen was refused by the browser.'));
  }
});

initTheme();
signedIn();
