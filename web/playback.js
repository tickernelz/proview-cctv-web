(function () {
  const MEDIA_TYPE = 'video/mp4; codecs="avc1.42E01E"';

  const state = {
    open: false,
    devices: [],
    device: null,
    files: [],
    truncatedCap: 0,
    chosen: null,
    socket: null,
    media: null,
    buffer: null,
    queue: [],
    init: false,
    bytes: 0,
    status: {},
    startedAt: 0,
    frameAt: null,
  };

  function el(id) {
    return document.getElementById(id);
  }

  function make(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function mb(bytes) {
    return (bytes / 1048576).toFixed(2) + ' MB';
  }

  function clock(seconds) {
    const whole = Math.max(0, Math.round(seconds));
    const mm = String(Math.floor(whole / 60)).padStart(2, '0');
    const ss = String(whole % 60).padStart(2, '0');
    return mm + ':' + ss;
  }

  function secondsBetween(begin, end) {
    const parse = (text) => {
      const stamp = Date.parse(text.replace(' ', 'T'));
      return Number.isFinite(stamp) ? stamp : null;
    };
    const a = parse(begin);
    const b = parse(end);
    if (a === null || b === null || b <= a) return 0;
    return Math.round((b - a) / 1000);
  }

  function deviceEndpoint() {
    return encodeURIComponent(state.device || '');
  }

  async function api(path) {
    const response = await fetch(path, { credentials: 'same-origin', headers: { 'content-type': 'application/json' } });
    const text = await response.text();
    let body = null;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      body = null;
    }
    if (!response.ok) {
      const message = body && body.message ? body.message : response.status + ' ' + response.statusText;
      throw new Error(message);
    }
    return body;
  }

  function setStatus(text) {
    const line = el('pb-status');
    if (line) line.textContent = text;
  }

  function describeStats() {
    const parts = [];
    if (state.status.openMs !== undefined) parts.push('tunnel ' + (state.status.openMs / 1000).toFixed(2) + 's');
    if (state.status.primeMs !== undefined) parts.push('prime ' + (state.status.primeMs / 1000).toFixed(2) + 's');
    if (state.status.fps) parts.push(state.status.fps + ' fps');
    if (state.frameAt !== null) {
      parts.push('first frame ' + (state.frameAt / 1000).toFixed(2) + 's');
    } else if (state.stats && state.stats.firstFrameMs !== null && state.stats.firstFrameMs !== undefined) {
      parts.push('muxed after ' + (state.stats.firstFrameMs / 1000).toFixed(2) + 's');
    }
    if (state.bytes) parts.push(mb(state.bytes));
    if (state.stats && state.stats.framesWritten) parts.push(state.stats.framesWritten + ' frames out');
    return parts.join(' · ');
  }

  function renderFiles() {
    const list = el('pb-files');
    if (!list) return;
    list.textContent = '';
    const summary = el('pb-summary');
    if (!state.files.length) {
      if (summary) summary.textContent = 'No files in that range.';
      return;
    }
    const bytes = state.files.reduce((total, file) => total + file.bytes, 0);
    if (summary) {
      const cap = state.truncatedCap ? ' · the device list is capped at ' + state.truncatedCap : '';
      summary.textContent = state.files.length + ' files · ' + mb(bytes) + cap;
    }
    for (const file of state.files) {
      const row = make('button', 'pb-row');
      row.type = 'button';
      const span = make('span', 'pb-when', file.begin.slice(11) + ' – ' + file.end.slice(11));
      const size = make('span', 'pb-size', mb(file.bytes));
      row.append(span, size);
      row.addEventListener('click', () => play(file, 0));
      list.append(row);
    }
  }

  function stop() {
    if (state.socket) {
      state.socket.onclose = null;
      state.socket.close();
      state.socket = null;
    }
    if (state.media) {
      try {
        if (state.media.readyState === 'open') state.media.endOfStream();
      } catch {}
      state.media = null;
    }
    state.buffer = null;
    state.queue = [];
    state.init = false;
    state.bytes = 0;
    state.stats = null;
    state.frameAt = null;
  }

  function pump() {
    const buffer = state.buffer;
    if (!buffer || buffer.updating || !state.queue.length) return;
    try {
      buffer.appendBuffer(state.queue.shift());
    } catch (error) {
      console.error('playback append failed', error);
      state.queue = [];
    }
  }

  function play(file, offset) {
    stop();
    state.chosen = file;
    state.status = {};
    state.startedAt = performance.now();
    const video = el('pb-video');
    if (!video) return;
    const duration = secondsBetween(file.begin, file.end);
    const scrub = el('pb-scrub');
    if (scrub) {
      scrub.max = String(duration);
      scrub.value = String(Math.min(offset, duration));
    }
    const label = el('pb-playing');
    if (label) label.textContent = file.begin + ' → ' + file.end;
    if (!window.MediaSource) {
      setStatus('This browser has no MediaSource, so the clip cannot play here.');
      return;
    }
    setStatus('opening the cloud tunnel…');
    state.frameAt = null;
    const media = new MediaSource();
    state.media = media;
    video.src = URL.createObjectURL(media);
    media.addEventListener(
      'sourceopen',
      () => {
        let buffer;
        try {
          buffer = media.addSourceBuffer(MEDIA_TYPE);
        } catch (error) {
          setStatus('this browser cannot decode H.264: ' + error.message);
          return;
        }
        state.buffer = buffer;
        buffer.addEventListener('updateend', () => {
          pump();
          if (video.buffered.length && state.frameAt === null) {
            state.frameAt = performance.now() - state.startedAt;
            setStatus(describeStats());
          }
        });
        const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
        const query =
          'device=' + deviceEndpoint() +
          '&channel=0' +
          '&name=' + encodeURIComponent(file.name) +
          '&begin=' + encodeURIComponent(file.begin) +
          '&end=' + encodeURIComponent(file.end) +
          '&start=' + Math.round(offset);
        const socket = new WebSocket(scheme + '://' + location.host + '/ws/playback?' + query);
        socket.binaryType = 'arraybuffer';
        state.socket = socket;
        socket.onmessage = (event) => {
          if (typeof event.data === 'string') {
            let frame = null;
            try {
              frame = JSON.parse(event.data);
            } catch {
              frame = null;
            }
            if (!frame) return;
            if (frame.type === 'status') {
              Object.assign(state.status, frame);
              setStatus(describeStats() || frame.message);
            } else if (frame.type === 'error') {
              setStatus(frame.message);
            } else if (frame.type === 'stats') {
              state.stats = frame;
              setStatus(describeStats());
            }
            return;
          }
          const bytes = new Uint8Array(event.data);
          state.bytes += bytes.length;
          if (!state.init) {
            state.init = true;
          }
          state.queue.push(bytes);
          pump();
          if (state.status.openMs !== undefined) setStatus(describeStats());
        };
        socket.onerror = () => setStatus('the playback socket failed to connect');
        socket.onclose = () => {
          if (state.media && state.media.readyState === 'open') {
            try {
              state.media.endOfStream();
            } catch {}
          }
        };
        video.play().catch(() => {});
      },
      { once: true }
    );
  }

  async function loadDevices() {
    const select = el('pb-device');
    if (!select) return;
    setStatus('reading the camera list…');
    let devices = [];
    try {
      devices = await api('/api/devices');
    } catch (error) {
      setStatus(error.message);
      return;
    }
    state.devices = Array.isArray(devices) ? devices : [];
    select.textContent = '';
    for (const device of state.devices) {
      const option = make('option', '', device.name || device.id);
      option.value = device.id;
      select.append(option);
    }
    const remembered = state.device;
    const known = state.devices.find((device) => device.id === remembered);
    state.device = known ? known.id : state.devices.length ? state.devices[0].id : null;
    if (state.device) select.value = state.device;
    setStatus(state.devices.length ? '' : 'No cameras on this account yet.');
    if (state.device) await loadFiles();
  }

  async function loadFiles() {
    if (!state.device) return;
    const day = el('pb-day');
    const date = day && day.value ? day.value : new Date().toISOString().slice(0, 10);
    setStatus('asking the device for its file list…');
    try {
      const body = await api(
        '/api/playback/files?device=' + deviceEndpoint() + '&channel=0&begin=' +
          encodeURIComponent(date + ' 00:00:00') + '&end=' + encodeURIComponent(date + ' 23:59:59')
      );
      state.files = (body && body.files) || [];
      state.truncatedCap = body && body.truncated ? body.cap : 0;
      renderFiles();
      setStatus('file list in ' + (body.elapsed_ms / 1000).toFixed(2) + 's');
    } catch (error) {
      state.files = [];
      renderFiles();
      setStatus(error.message);
    }
  }

  function buildOverlay() {
    if (el('pb')) return;
    const overlay = make('div', 'pb-screen');
    overlay.id = 'pb';
    overlay.hidden = true;
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-label', 'Playback');

    const bar = make('header', 'topbar');
    const back = make('button', 'ctl back');
    back.type = 'button';
    back.innerHTML = '<svg class="i i-16" aria-hidden="true"><use href="#i-arrow-left"></use></svg><span>Live</span>';
    back.addEventListener('click', () => close());
    const title = make('h1', 'screen-title', 'Playback');
    title.tabIndex = -1;
    bar.append(back, title, make('div', 'spacer'));
    overlay.append(bar);

    const body = make('div', 'pb-body');

    const range = make('div', 'pb-controls');
    const day = document.createElement('input');
    day.type = 'date';
    day.id = 'pb-day';
    day.value = new Date().toISOString().slice(0, 10);
    day.addEventListener('change', () => loadFiles());
    const select = document.createElement('select');
    select.id = 'pb-device';
    select.addEventListener('change', () => {
      state.device = select.value;
      loadFiles();
    });
    const refresh = make('button', 'btn btn-line', 'Refresh');
    refresh.type = 'button';
    refresh.addEventListener('click', () => loadFiles());
    range.append(select, day, refresh);

    const summary = make('p', 'pb-summary');
    summary.id = 'pb-summary';
    const files = make('div', 'pb-list');
    files.id = 'pb-files';

    const stage = make('div', 'pb-stage');
    const video = document.createElement('video');
    video.id = 'pb-video';
    video.playsInline = true;
    video.muted = true;
    video.controls = true;
    const playing = make('p', 'pb-playing');
    playing.id = 'pb-playing';
    const scrubRow = make('div', 'pb-scrub-row');
    const scrub = document.createElement('input');
    scrub.type = 'range';
    scrub.id = 'pb-scrub';
    scrub.min = '0';
    scrub.max = '0';
    scrub.step = '1';
    const seek = make('button', 'btn btn-line', 'Play from here');
    seek.type = 'button';
    seek.addEventListener('click', () => {
      if (!state.chosen) return;
      play(state.chosen, Number(scrub.value) || 0);
    });
    scrubRow.append(scrub, seek);
    const status = make('p', 'pb-status');
    status.id = 'pb-status';
    status.setAttribute('role', 'status');
    stage.append(video, playing, scrubRow, status);

    body.append(range, summary, files, stage);
    overlay.append(body);
    document.body.append(overlay);

    const style = document.createElement('style');
    style.textContent = [
      '.pb-screen{position:fixed;inset:0;z-index:60;background:var(--bg,#0b0d10);display:flex;flex-direction:column;overflow:auto}',
      '.pb-screen[hidden]{display:none}',
      '.pb-body{display:grid;gap:16px;padding:16px;grid-template-columns:minmax(220px,320px) 1fr;align-items:start}',
      '@media (max-width:820px){.pb-body{grid-template-columns:1fr}}',
      '.pb-controls{display:flex;gap:8px;grid-column:1/-1;flex-wrap:wrap}',
      '.pb-controls input,.pb-controls select{font:inherit;padding:6px 8px;border-radius:8px;border:1px solid currentColor;background:transparent;color:inherit}',
      '.pb-summary{grid-column:1/-1;margin:0;opacity:.75}',
      '.pb-list{display:flex;flex-direction:column;gap:6px;max-height:60vh;overflow:auto}',
      '.pb-row{display:flex;justify-content:space-between;gap:12px;font:inherit;text-align:left;padding:8px 10px;border-radius:8px;border:1px solid rgba(128,128,128,.35);background:transparent;color:inherit;cursor:pointer}',
      '.pb-row:hover{border-color:currentColor}',
      '.pb-size{opacity:.7}',
      '.pb-stage{display:flex;flex-direction:column;gap:8px}',
      '.pb-stage video{width:100%;max-height:65vh;background:#000;border-radius:12px}',
      '.pb-playing{margin:0;font-variant-numeric:tabular-nums;opacity:.8}',
      '.pb-scrub-row{display:flex;gap:8px;align-items:center}',
      '.pb-scrub-row input{flex:1}',
      '.pb-status{margin:0;min-height:1.4em;font-variant-numeric:tabular-nums}',
    ].join('\n');
    document.head.append(style);
  }

  function open() {
    buildOverlay();
    state.open = true;
    el('pb').hidden = false;
    loadDevices();
  }

  function close() {
    stop();
    state.open = false;
    const overlay = el('pb');
    if (overlay) overlay.hidden = true;
  }

  function mount() {
    const controls = el('controls');
    if (!controls || el('pb-open')) return;
    const button = make('button', 'ctl');
    button.id = 'pb-open';
    button.type = 'button';
    button.innerHTML = '<svg class="i" aria-hidden="true"><use href="#i-clock"></use></svg><span>Playback</span>';
    button.addEventListener('click', open);
    const anchor = el('move');
    if (anchor) controls.insertBefore(button, anchor);
    else controls.append(button);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', mount, { once: true });
  } else {
    mount();
  }
  window.addEventListener('hashchange', () => {
    if ((location.hash || '').replace(/^#/, '') === 'playback') open();
  });
  if ((location.hash || '').replace(/^#/, '') === 'playback') open();
})();
