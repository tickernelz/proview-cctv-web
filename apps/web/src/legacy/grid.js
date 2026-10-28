import { StreamPlayer, api, deviceStatus, el, enc, hm, hms, make, openCamera, sessionLost, shortStreamError, state, statusLine } from './app.js';

const GRID_MAX_TILES = 9;
const GRID_QUALITY_WORDS = { main: 'HD', sub: 'SD' };
const GRID_LAYOUTS = [
  { value: 'auto', label: 'Auto', columns: 0 },
  { value: '1', label: '1', columns: 1 },
  { value: '2', label: '2', columns: 2 },
  { value: '3', label: '3', columns: 3 },
];

const gridView = {
  open: false,
  spec: null,
  load: null,
  token: 0,
  players: new Map(),
  saving: false,
  queued: false,
  error: '',
  savedAt: 0,
};

function gridLayout(value) {
  return GRID_LAYOUTS.find((entry) => entry.value === value) || GRID_LAYOUTS[0];
}

function gridTileKey(view) {
  return view.device + ':' + view.channel;
}

function gridDefaultQuality(index) {
  return index === 0 ? state.prefs.quality : 'sub';
}

function gridSpecFrom(payload) {
  const layout = gridLayout(payload && payload.layout ? payload.layout : 'auto');
  const tiles = Array.isArray(payload && payload.tiles) ? payload.tiles : [];
  const seen = new Set();
  const out = [];
  for (const tile of tiles) {
    const device = typeof tile.device === 'string' ? tile.device.trim() : '';
    const channel = Number(tile.channel) || 0;
    const quality = GRID_QUALITY_WORDS[tile.quality] ? tile.quality : 'main';
    if (!device || channel < 1) {
      continue;
    }
    const key = device + ':' + channel;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    out.push({ device, channel, quality });
    if (out.length >= GRID_MAX_TILES) {
      break;
    }
  }
  return { layout: layout.value, tiles: out };
}

function gridReconcile(spec) {
  const cameras = new Map(state.cameras.map((camera) => [camera.key, camera]));
  const tiles = [];
  const seen = new Set();
  for (const tile of spec.tiles) {
    const key = gridTileKey(tile);
    if (!cameras.has(key) || seen.has(key)) {
      continue;
    }
    seen.add(key);
    tiles.push({ device: tile.device, channel: tile.channel, quality: tile.quality });
  }
  for (const camera of state.cameras) {
    if (tiles.length >= GRID_MAX_TILES) {
      break;
    }
    if (seen.has(camera.key)) {
      continue;
    }
    seen.add(camera.key);
    tiles.push({ device: camera.device.id, channel: camera.channel, quality: gridDefaultQuality(tiles.length) });
  }
  return { layout: spec.layout, tiles };
}

async function gridReadSpec() {
  if (gridView.spec) {
    return gridView.spec;
  }
  if (!gridView.load) {
    gridView.load = api('/api/grid')
      .then((payload) => {
        gridView.spec = gridSpecFrom(payload);
        gridView.error = '';
        return gridView.spec;
      })
      .catch((error) => {
        if (sessionLost(error)) {
          return gridSpecFrom(null);
        }
        gridView.error = 'Could not read the saved grid: ' + error.message;
        return null;
      })
      .finally(() => {
        gridView.load = null;
      });
  }
  return gridView.load;
}

async function gridSave() {
  if (!gridView.spec) {
    return;
  }
  if (gridView.saving) {
    gridView.queued = true;
    return;
  }
  gridView.saving = true;
  gridView.queued = false;
  try {
    await api('/api/grid', {
      method: 'POST',
      body: JSON.stringify({ layout: gridView.spec.layout, tiles: gridView.spec.tiles }),
    });
    gridView.error = '';
    gridView.savedAt = Date.now();
  } catch (error) {
    if (!sessionLost(error)) {
      gridView.error = 'The grid was not saved: ' + error.message;
    }
  } finally {
    gridView.saving = false;
  }
  gridRenderNote();
  gridRenderCount();
  if (gridView.queued) {
    gridSave();
  }
}

function gridBar() {
  let bar = el('grid-bar');
  if (bar) {
    return bar;
  }
  bar = make('div', 'grid-bar');
  bar.id = 'grid-bar';
  const group = make('div', 'grid-seg');
  group.setAttribute('role', 'group');
  group.setAttribute('aria-label', 'Grid layout');
  group.dataset.testid = 'grid-layout';
  for (const layout of GRID_LAYOUTS) {
    const button = make('button', '', layout.label);
    button.type = 'button';
    button.dataset.layout = layout.value;
    button.title = layout.columns ? layout.columns + ' columns' : 'As many columns as the window allows';
    button.addEventListener('click', () => gridSetLayout(layout.value));
    group.append(button);
  }
  const note = make('p', 'grid-note');
  note.id = 'grid-note';
  bar.append(group, note);
  el('grid').insertBefore(bar, el('tiles'));
  return bar;
}

function gridRenderBar() {
  const bar = gridBar();
  const layout = gridLayout(gridView.spec ? gridView.spec.layout : 'auto');
  for (const button of bar.querySelectorAll('button[data-layout]')) {
    button.disabled = !gridView.spec;
    button.setAttribute('aria-pressed', String(button.dataset.layout === layout.value));
  }
  gridRenderNote();
}

function gridRenderNote() {
  const note = el('grid-note');
  if (!note) {
    return;
  }
  if (gridView.error) {
    note.textContent = gridView.error;
    note.dataset.state = 'error';
    return;
  }
  note.dataset.state = 'ok';
  if (gridView.saving) {
    note.textContent = 'Saving the layout';
    return;
  }
  note.textContent = gridView.savedAt
    ? 'Layout and streams saved at ' + hm(gridView.savedAt)
    : 'Layout and streams are saved on this server';
}

function gridRenderCount() {
  const tiles = [...document.querySelectorAll('.tile')];
  const count = (name) => tiles.filter((tile) => tile.dataset.state === name).length;
  const cameras = state.cameras.length;
  const parts = [cameras + ' camera' + (cameras === 1 ? '' : 's')];
  if (count('playing')) {
    parts.push(count('playing') + ' live');
  }
  if (count('loading')) {
    parts.push(count('loading') + ' connecting');
  }
  if (count('error')) {
    parts.push(count('error') + ' failed');
  }
  if (count('offline')) {
    parts.push(count('offline') + ' offline');
  }
  if (cameras > GRID_MAX_TILES) {
    parts.push('showing the first ' + GRID_MAX_TILES);
  }
  parts.push('layout ' + gridLayout(gridView.spec ? gridView.spec.layout : 'auto').label.toLowerCase());
  el('grid-count').textContent = parts.join(' · ');
}

function gridSetTileState(tile, name, detail) {
  tile.dataset.state = name;
  const fail = tile.querySelector('.tile-fail');
  const right = tile.querySelector('.tile-meta .right');
  tile.querySelector('.prog').hidden = name !== 'loading';
  tile.querySelector('video').hidden = name !== 'playing' && name !== 'stalled';
  tile.querySelector('.tile-state').textContent =
    { loading: 'connecting', playing: 'live', stalled: 'frozen', error: 'failed', offline: 'offline' }[name] || name;
  right.classList.toggle('warn', name === 'error' || name === 'offline' || name === 'stalled');
  right.textContent = name === 'playing' || name === 'error' ? hms(Date.now()) : name === 'offline' ? 'offline' : '';
  fail.hidden = name !== 'error' && name !== 'offline';
  if (!fail.hidden) {
    const camera = state.cameras.find((entry) => entry.key === tile.dataset.key);
    fail.querySelector('strong').textContent = name === 'offline' ? 'Offline' : 'No picture';
    fail.querySelector('span').textContent =
      name === 'offline' ? statusLine(camera ? camera.device : null) + '.' : detail || 'The stream stopped.';
    fail.querySelector('button').hidden = name !== 'error';
  }
  gridRenderCount();
}

function gridStopTile(key) {
  const player = gridView.players.get(key);
  if (player) {
    player.destroy();
    gridView.players.delete(key);
  }
}

async function gridPlayTile(tile, view) {
  const key = gridTileKey(view);
  const camera = state.cameras.find((entry) => entry.key === key);
  if (!camera) {
    return;
  }
  gridStopTile(key);
  if (deviceStatus(camera.device).known && !camera.device.online) {
    gridSetTileState(tile, 'offline');
    return;
  }
  const player = new StreamPlayer(tile.querySelector('video'), (name, message) => gridSetTileState(tile, name, message), () => state.muted);
  gridView.players.set(key, player);
  gridSetTileState(tile, 'loading');
  try {
    const ticket = await api(
      '/api/stream?device=' + enc(view.device) + '&channel=' + view.channel + '&quality=' + enc(view.quality),
    );
    if (!gridView.open || gridView.players.get(key) !== player) {
      player.destroy();
      return;
    }
    await player.play(ticket);
  } catch (error) {
    if (sessionLost(error)) {
      return;
    }
    gridSetTileState(tile, 'error', shortStreamError(error));
    const fail = tile.querySelector('.tile-fail span');
    if (fail) {
      fail.title = error.message;
    }
  }
}

function gridEntry(key) {
  return gridView.spec ? gridView.spec.tiles.find((tile) => gridTileKey(tile) === key) : null;
}

function gridToggleQuality(key) {
  const entry = gridEntry(key);
  if (!entry) {
    return;
  }
  const quality = entry.quality === 'main' ? 'sub' : 'main';
  entry.quality = quality;
  const tile = document.querySelector('.tile[data-key="' + key + '"]');
  if (tile) {
    const pick = tile.querySelector('.tile-pick');
    if (pick) {
      pick.textContent = GRID_QUALITY_WORDS[quality];
      pick.setAttribute('aria-label', 'This tile plays the ' + (quality === 'main' ? 'main stream' : 'sub stream') + '. Switch it.');
    }
    gridPlayTile(tile, entry);
  }
  gridSave();
}

function gridSetLayout(value) {
  if (!gridView.spec) {
    return;
  }
  gridView.spec.layout = gridLayout(value).value;
  gridApplyLayout();
  gridRenderBar();
  gridSave();
}

const GRID_WIDE = window.matchMedia('(min-width: 1024px)');
const GRID_MEDIUM = window.matchMedia('(min-width: 640px)');

function gridAutoColumns(count) {
  return Math.min(count, GRID_WIDE.matches ? 4 : GRID_MEDIUM.matches ? 2 : 1);
}

for (const query of [GRID_WIDE, GRID_MEDIUM]) {
  query.addEventListener('change', () => {
    if (gridView.spec && gridLayout(gridView.spec.layout).value === 'auto') {
      gridApplyLayout();
    }
  });
}

function gridApplyLayout() {
  const tiles = el('tiles');
  const layout = gridLayout(gridView.spec ? gridView.spec.layout : 'auto');
  const count = Math.max(1, gridView.spec ? gridView.spec.tiles.length : 1);
  const columns = layout.columns || gridAutoColumns(count);
  tiles.dataset.layout = layout.value;
  tiles.style.gridTemplateColumns = 'repeat(' + columns + ', minmax(0, 1fr))';
  tiles.style.gridTemplateRows = 'repeat(' + Math.ceil(count / columns) + ', minmax(0, 1fr))';
}

function gridBuildTile(view, index) {
  const camera = state.cameras.find((entry) => entry.key === gridTileKey(view));
  const tile = make('article', 'tile');
  tile.dataset.key = gridTileKey(view);
  tile.dataset.testid = 'grid-tile';
  const open = make('button', 'tile-open');
  open.type = 'button';
  open.setAttribute('aria-label', 'Open ' + camera.name + ' in Live');
  open.addEventListener('click', () => openCamera(state.cameras.indexOf(camera)));
  const well = make('div', 'well');
  const video = document.createElement('video');
  video.dataset.testid = 'grid-tile-video';
  video.playsInline = true;
  video.muted = true;
  video.hidden = true;
  const prog = make('div', 'prog');
  const fail = make('div', 'tile-fail');
  fail.hidden = true;
  const retry = make('button', '', 'Try again');
  retry.type = 'button';
  retry.addEventListener('click', () => gridPlayTile(tile, gridEntry(gridTileKey(view)) || view));
  fail.append(make('strong'), make('span'), retry);
  well.append(video, prog, fail);
  const meta = make('div', 'tile-meta');
  const left = make('p');
  left.append(make('strong', '', camera.name), make('span', 'tile-state'));
  const pick = make('button', 'tile-pick', GRID_QUALITY_WORDS[view.quality]);
  pick.type = 'button';
  pick.title = 'Main stream or the lighter sub stream, for this tile';
  pick.setAttribute(
    'aria-label',
    'This tile plays the ' + (view.quality === 'main' ? 'main stream' : 'sub stream') + '. Switch it.',
  );
  pick.addEventListener('click', () => gridToggleQuality(gridTileKey(view)));
  const clock = make('span', 'right');
  clock.dataset.testid = 'grid-tile-clock';
  meta.append(left, pick, clock);
  tile.append(open, well, meta);
  tile.dataset.index = String(index);
  return tile;
}

async function gridStart() {
  gridView.token += 1;
  const token = gridView.token;
  for (const key of [...gridView.players.keys()]) {
    gridStopTile(key);
  }
  gridBar();
  const stored = await gridReadSpec();
  if (token !== gridView.token || !gridView.open) {
    return;
  }
  gridView.spec = gridReconcile(gridView.spec || gridSpecFrom(stored));
  const tiles = el('tiles');
  tiles.replaceChildren();
  tiles.dataset.count = String(gridView.spec.tiles.length);
  gridApplyLayout();
  gridRenderBar();
  const views = gridView.spec.tiles.filter((view) => state.cameras.some((camera) => camera.key === gridTileKey(view)));
  views.forEach((view, index) => {
    const tile = gridBuildTile(view, index);
    tiles.append(tile);
    gridPlayTile(tile, view);
  });
  gridSyncSelection();
  gridRenderCount();
}

function gridSyncSelection() {
  for (const tile of document.querySelectorAll('.tile')) {
    tile.classList.toggle('sel', tile.dataset.key === state.selectedId + ':' + state.channel);
  }
}

function gridMoveFocus(key) {
  const buttons = [...document.querySelectorAll('.tile-open')];
  if (!buttons.length) {
    return;
  }
  const tiles = el('tiles');
  const columns = Math.max(1, getComputedStyle(tiles).gridTemplateColumns.split(' ').filter(Boolean).length);
  const index = Math.max(0, buttons.indexOf(document.activeElement));
  const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns }[key];
  const next = buttons[Math.min(buttons.length - 1, Math.max(0, index + step))];
  next.focus();
}

function gridOpen() {
  gridView.open = true;
  gridBar();
  gridRenderBar();
  gridStart();
}

function gridClose() {
  gridView.open = false;
  gridView.token += 1;
  for (const key of [...gridView.players.keys()]) {
    gridStopTile(key);
  }
}

window.ProviewGrid = {
  open: gridOpen,
  close: gridClose,
  syncSelection: gridSyncSelection,
  moveFocus: gridMoveFocus,
};
