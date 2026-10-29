import { api } from '../lib/api';
import { StreamPlayer } from '../lib/stream-player';
import { live, menuOpen } from '../live/store';
import { startTalk, stopTalk } from '../live/talk';
import { savePrefs, state } from '../app/state';
import { hms } from '../app/format';
import { buildCameras, currentCamera, deviceStatus, selectedDevice, statusLine } from '../app/devices';
import { applyTheme } from '../app/theme';
import { modeName, openSettings, renderPrefsIntoForm, sessionLost, showSection, signedIn, sourceLabel } from '../app/shell';

export const el = (id) => document.getElementById(id);
export const enc = encodeURIComponent;

const POPOVER = typeof HTMLElement.prototype.showPopover === 'function';

document.documentElement.dataset.popover = POPOVER ? 'native' : 'fallback';

function icon(name, extra) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'i' + (extra ? ' ' + extra : ''));
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttribute('href', '#i-' + name);
  svg.append(use);
  return svg;
}

export function make(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
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

export function closeFloating(restoreFocus) {
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

function lensMeta() {
  const count = lensEntries().length;
  if (count > 1) {
    return 'dual lens';
  }
  return state.lens ? 'single lens' : '';
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
    row.dataset.testid = 'camera-option';
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

export function openCamera(index) {
  const camera = state.cameras[index];
  if (!camera) {
    return;
  }
  if (state.view !== 'live') {
    setView('live', { keepStream: true });
  }
  selectDevice(camera.device.id, { channel: camera.channel });
}

export async function loadDevices(options = {}) {
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
  window.dispatchEvent(new Event('proview:devices'));
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
  live.record.disabled = name !== 'playing';
  live.snapshotDisabled = !showing;
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
  if (live.talk.on) {
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

export function renderCaption() {
  const camera = currentCamera();
  live.camera = camera ? camera.name : '';
  live.state = captionText();
  renderFlag();
}

function renderFlag() {
  const name = state.stage.name;
  let text = '';
  let tone = '';
  if (state.recorder) {
    text = 'Recording ' + elapsed(state.recordStart);
    tone = 'rec';
  } else if (live.talk.on) {
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
  live.flag.text = text;
  live.flag.tone = tone;
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

export function renderFsLine() {
  const camera = currentCamera();
  const showing = state.stage.name === 'playing' || state.stage.name === 'stalled';
  el('fs-line').hidden = !(document.fullscreenElement === el('well') && state.prefs.osd && showing && camera);
  el('fs-name').textContent = camera ? camera.name + (lensEntries().length > 1 ? ' · ' + lensViewLabel().toLowerCase() : '') : '';
  el('fs-time').textContent = (state.stage.name === 'playing' ? 'live' : state.stage.name) + ' · ' + hms(Date.now());
}

function tick() {
  const now = hms(Date.now());
  live.clock = now;
  if (!el('fs-line').hidden) {
    renderFsLine();
  }
  if (state.recorder) {
    live.record.elapsed = elapsed(state.recordStart);
  }
  renderFlag();
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
  if (error.code === 'misconfigured') {
    return error.message;
  }
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
    }, () => state.muted);
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

export function stopAll() {
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
    live.settings.lensNote = 'The server answered ' + error.status + ' for GET /api/lens.';
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
  live.lens.shown = pair;
  live.lens.label = lensViewLabel();
  live.settings.lensNote = lensNote();
  live.settings.lensTitle = state.lens && state.lens.ptzEvidence ? state.lens.ptzEvidence : '';
  live.settings.lensRolesKnown = entries.some((entry) => entry.role !== 'unknown');
  live.lens.options = [{ value: 'combined', name: 'Both lenses', sub: 'the frame as the camera sends it' }].concat(
    entries.map((entry) => ({ value: String(entry.index), name: entry.label, sub: lensSub(entry) })),
  );
  live.lens.value = String(state.lensView);
  live.lens.note = lensNote();
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

export function layoutLensCrop() {
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

export function renderControlAvailability() {
  const rows = state.control.rows;
  if (state.control.error) {
    live.settings.controlNote = 'Device controls could not be read: ' + state.control.error;
  } else if (!rows.length) {
    live.settings.controlNote = 'Device controls have not been checked yet.';
  } else {
    const supported = rows.filter((row) => row.state === 'supported').length;
    const backends = [state.control.onvif ? 'ONVIF' : null, state.control.dvrip ? 'DVRIP' : null].filter(Boolean);
    live.settings.controlNote =
      supported + ' of ' + rows.length + ' rows are usable' + (backends.length ? ' over ' + backends.join(' and ') : '') + '. Blocked rows keep the device\u2019s own reason.';
  }
  live.settings.light = buttonVerdict({ dataset: { act: 'strobe' } });
  for (const button of document.querySelectorAll('[data-requires="device"]')) {
    const verdict = buttonVerdict(button);
    if (button.closest('#controls') && live.gate[button.dataset.act]) {
      live.gate[button.dataset.act] = { ok: verdict.ok, why: verdict.why };
      continue;
    }
    button.disabled = !verdict.ok;
    button.title = verdict.why;
  }
  if (state.padOpen && !live.gate.move.ok) {
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
  live.settings.capabilities = rows.map((row) => row.map((cell) => (cell == null ? '' : String(cell))));
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

export function setControlStatus(message) {
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
      button.dataset.testid = 'ptz-preset';
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

export async function toggleWhiteLight() {
  try {
    const current = await api('/api/device/controls/light');
    const next = current.mode === 'always' ? 'off' : 'always';
    const applied = await api('/api/device/controls/light?mode=' + enc(next), { method: 'POST' });
    live.settings.lightStatus = 'White light: ' + (applied.mode || next) + ' (it was ' + (current.mode || 'not reported') + ').';
  } catch (error) {
    if (!sessionLost(error)) {
      live.settings.lightStatus = 'The camera refused the white light: ' + error.status + ' ' + error.message;
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
  if (!live.gate.move.ok || state.view !== 'live') {
    return;
  }
  state.padOpen = true;
  el('pad').hidden = false;
  live.padOpen = true;
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
  live.padOpen = false;
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
  live.settings.lightStatus = '';
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
  const previous = state.players.get('stage');
  if (previous) {
    previous.destroy();
    state.players.delete('stage');
  }
  const player = stagePlayer();
  state.ticket = null;
  setStage('loading');
  try {
    const ticket = await api(
      '/api/stream?device=' + enc(camera.device.id) + '&channel=' + enc(camera.channel) + '&quality=' + enc(state.prefs.quality),
    );
    if (currentCamera() !== camera || state.players.get('stage') !== player) {
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
    const misconfigured = error.code === 'misconfigured';
    setStage('error', { text: base, code: misconfigured ? '' : error.message, stage: attempt });
    console.error('stream request failed', error);
    if (error.status === 501 && !misconfigured && camera.device) {
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

export function renderCameraInfo() {
  const camera = currentCamera();
  if (!camera) {
    live.settings.camera = null;
    return;
  }
  const device = camera.device;
  const ticket = state.ticket;
  live.settings.camera = {
    name: camera.name,
    id: device.id,
    channel: String(camera.channel),
    model: device.model || 'model not reported',
    connection:
      (state.mode ? modeName(state.mode.mode)[0].toUpperCase() + modeName(state.mode.mode).slice(1) + ' · answering ' + sourceLabel() : sourceLabel()) +
      (device.lan_host ? ' · ' + device.lan_host : ''),
    stream: ticket
      ? 'Stream: ' + ticket.kind.toUpperCase() + ' · ' + (state.prefs.quality === 'sub' ? 'SD' : 'HD') + ' · ' + ticket.url + (ticket.expires_in ? ' · ticket expires in ' + ticket.expires_in + ' s' : '')
      : 'No stream ticket yet.',
  };
}

export function shortStreamError(error) {
  if (error.code === 'misconfigured') {
    return error.message;
  }
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

export function renderMute() {
  const video = el('video');
  video.muted = state.muted;
  live.muted = state.muted;
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
  live.record.on = Boolean(state.recorder);
  live.record.elapsed = live.record.on ? elapsed(state.recordStart) : '';
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

export async function toggleFullscreen() {
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
  renderTileSelection();
  live.grid.generation += 1;
  live.grid.open = true;
}

function stopGrid() {
  live.grid.open = false;
}

function renderTileSelection() {
  live.grid.selected = state.selectedId + ':' + state.channel;
}

function moveTileFocus(key) {
  live.grid.moveFocus(key);
}

export function setView(name, options = {}) {
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

export function setQuality(quality) {
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
    r: () => (live.record.disabled ? null : toggleRecord()),
    s: () => (live.snapshotDisabled ? null : captureSnapshot()),
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
  if (event.defaultPrevented || menuOpen() || live.screen) {
    return;
  }
  if (state.screen === 'watch') {
    onWatchKey(event);
  }
});

document.addEventListener('keyup', (event) => {
  if (event.key.toLowerCase() === 't') {
    stopTalk();
  }
});

window.addEventListener('blur', () => stopTalk());

el('empty-settings').addEventListener('click', () => openSettings('config', { focus: true }));
el('open-settings').addEventListener('click', () => openSettings(state.section));
el('refresh').addEventListener('click', () => loadDevices());
el('banner-action').addEventListener('click', () => {
  if (bannerAction) {
    bannerAction();
  }
});
el('retry').addEventListener('click', () => requestStream());
el('check-password').addEventListener('click', () => {
  live.settings.focusKey = 'PROVIEW_DEVICE_PASS';
  openSettings('config');
});
el('switcher').addEventListener('click', () => toggleFloating(el('camera-menu'), el('switcher'), renderCameraMenu));
el('grid-toggle').addEventListener('click', () => setView(state.view === 'grid' ? 'live' : 'grid'));
el('keys-close').addEventListener('click', () => closeFloating(false));
for (const key of el('pad').querySelectorAll('.pk')) {
  key.addEventListener('click', () => sendControl(key.dataset.act));
}
el('tour').addEventListener('click', () => sendControl('cruise'));
document.querySelector('.skip').addEventListener('click', (event) => {
  event.preventDefault();
  const first = el('controls').querySelector('button:not(:disabled):not([hidden])');
  (first || el('controls')).focus();
});

Object.assign(live.actions, {
  startTalk,
  stopTalk,
  toggleMute,
  toggleRecord,
  captureSnapshot,
  togglePad,
  toggleFullscreen,
  chooseLens,
  setQuality,
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

Object.assign(window, { openSettings, showSection, state, signedIn, setQuality });
