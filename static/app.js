const state = { devices: [], current: null };

const el = (id) => document.getElementById(id);

function showError(message) {
  const box = el('error');
  box.textContent = message;
  box.hidden = !message;
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    ...options,
  });
  const text = await response.text();
  const body = text ? JSON.parse(text) : null;
  if (!response.ok) {
    throw new Error(body && body.message ? body.message : response.status + ' ' + response.statusText);
  }
  return body;
}

function signedIn() {
  el('login-view').hidden = true;
  el('device-view').hidden = false;
  el('signout').hidden = false;
}

function signedOut() {
  el('login-view').hidden = false;
  el('device-view').hidden = true;
  el('signout').hidden = true;
  el('devices').replaceChildren();
  el('video').hidden = true;
}

async function loadDevices() {
  state.devices = await api('/api/devices');
  const list = el('devices');
  list.replaceChildren();
  for (const device of state.devices) {
    const item = document.createElement('li');
    const button = document.createElement('button');
    button.textContent = device.name + ' (' + device.model + ', ' + device.channels + ' ch)';
    button.addEventListener('click', () => openDevice(device));
    item.append(button);
    if (!device.online) {
      const badge = document.createElement('span');
      badge.className = 'offline';
      badge.textContent = 'offline';
      item.append(badge);
    }
    list.append(item);
  }
}

async function openDevice(device) {
  state.current = device;
  el('player-status').textContent = 'Requesting stream for ' + device.name + '…';
  el('video').hidden = true;
  try {
    const ticket = await api('/api/stream?device=' + encodeURIComponent(device.id) + '&channel=1&quality=main');
    el('player-status').textContent = 'Stream ' + ticket.kind + ' for ' + device.name;
    if (ticket.kind === 'hls' && window.Hls && window.Hls.isSupported()) {
      const video = el('video');
      const hls = new window.Hls();
      hls.loadSource(ticket.url);
      hls.attachMedia(video);
      video.hidden = false;
    }
  } catch (error) {
    el('player-status').textContent = 'No stream: ' + error.message;
  }
}

el('login-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  showError('');
  try {
    await api('/api/login', {
      method: 'POST',
      body: JSON.stringify({ email: el('email').value, password: el('password').value }),
    });
    el('password').value = '';
    signedIn();
    await loadDevices();
  } catch (error) {
    showError(error.message);
  }
});

el('signout').addEventListener('click', async () => {
  await api('/api/logout', { method: 'POST' }).catch(() => {});
  signedOut();
});
