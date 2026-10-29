import { api } from '../lib/api';
import { live } from '../live/store';
import { state } from '../app/state';
import { selectedDevice } from '../app/devices';
import { renderSwitcher } from '../app/chrome';
import { renderCaption } from '../app/caption';
import { sessionLost } from '../app/shell';
import { renderFsLine } from '../app/stage';

export const el = (id) => document.getElementById(id);
export const enc = encodeURIComponent;

export function make(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

window.addEventListener('resize', () => layoutLensCrop());

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

export function deriveLensFromFrame() {
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

export async function loadLens(deviceId, channel) {
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

export function lensEntries() {
  return state.lens && state.lens.lenses.length ? state.lens.lenses : [];
}

export function lensViewLabel() {
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

export function renderLensPicker() {
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

export function chooseLens(value) {
  applyLensView(value);
  renderLensPicker();
}

export function cycleLens() {
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

export function applyLensView(value) {
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

export function currentCrop() {
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

export async function probeControlChannel(device) {
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

export async function sendControl(action) {
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

export function closePad() {
  if (!state.padOpen) {
    return;
  }
  state.padOpen = false;
  el('pad').hidden = true;
  live.padOpen = false;
  renderCaption();
}

export function togglePad() {
  if (state.padOpen) {
    closePad();
  } else {
    openPad();
  }
}
