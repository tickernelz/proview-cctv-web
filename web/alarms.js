(() => {
  const STATE = {
    devices: [],
    serial: '',
    entries: new Map(),
    seen: new Set(),
    timer: 0,
    pollSeconds: 20,
    hours: 24,
    busy: false,
    alerts: false,
    photos: 'unknown',
    missing: new Set(),
    last: null,
    focus: '',
    focusShown: '',
  };

  const desktop = () =>
    Boolean(window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen);

  const css = document.createElement('style');
  css.textContent = [
    '.alarm-bell{position:relative}',
    '.alarm-bell[hidden]{display:none}',
    '.alarm-bell .badge{position:absolute;top:-2px;right:-2px;min-width:16px;height:16px;border-radius:999px;background:#e0533d;color:#fff;font-size:10px;line-height:16px;text-align:center;padding:0 4px}',
    '.alarm-sheet{position:fixed;inset:auto 18px 74px auto;width:min(420px,calc(100vw - 36px));max-height:min(70vh,640px);z-index:41;display:flex;flex-direction:column;border-radius:14px;border:1px solid var(--line,#2c3140);background:var(--bg-1,#101218);color:inherit;box-shadow:0 18px 48px rgba(0,0,0,.5);overflow:hidden}',
    '.alarm-sheet[hidden]{display:none}',
    '.alarm-head{display:flex;align-items:center;gap:10px;padding:12px 14px;border-bottom:1px solid var(--line,#2c3140)}',
    '.alarm-head h2{margin:0;font-size:14px;font-weight:600}',
    '.alarm-head .grow{flex:1}',
    '.alarm-head button,.alarm-foot button{background:transparent;border:1px solid var(--line,#2c3140);border-radius:8px;color:inherit;font:inherit;font-size:12px;padding:4px 8px;cursor:pointer}',
    '.alarm-bar{display:flex;gap:8px;align-items:center;padding:8px 14px;border-bottom:1px solid var(--line,#2c3140);font-size:12px;color:var(--dim,#96a0b5)}',
    '.alarm-bar select{background:var(--bg-1,#101218);color:inherit;border:1px solid var(--line,#2c3140);border-radius:8px;font:inherit;font-size:12px;padding:3px 6px}',
    '.alarm-list{overflow:auto;padding:6px 0}',
    '.alarm-row{display:grid;grid-template-columns:64px 1fr;gap:10px;padding:8px 14px;align-items:center}',
    '.alarm-row .thumb{width:64px;height:48px;border-radius:6px;background:#20242e}',
    '.alarm-row .what{font-size:13px}',
    '.alarm-row .when{font-variant-numeric:tabular-nums;font-size:12px;color:var(--dim,#96a0b5)}',
    '.alarm-row .chan{font-size:11px;color:var(--dim,#96a0b5)}',
    '.alarm-row.focus{background:var(--bg-3,#1a1e28)}',
    '.alarm-foot{margin-top:auto;border-top:1px solid var(--line,#2c3140);padding:8px 14px;font-size:11px;color:var(--dim,#96a0b5);display:flex;gap:8px;align-items:center}',
    '.alarm-foot .grow{flex:1}',
  ].join('\n');
  document.head.appendChild(css);

  const bell = document.createElement('button');
  bell.type = 'button';
  bell.className = 'iconbtn alarm-bell';
  bell.hidden = true;
  bell.setAttribute('aria-haspopup', 'dialog');
  bell.setAttribute('aria-label', 'Alarm messages');
  bell.title = 'Alarm messages';
  bell.innerHTML = '<span aria-hidden="true">&#128276;</span><span class="badge" hidden>0</span>';

  const sheet = document.createElement('section');
  sheet.className = 'alarm-sheet';
  sheet.hidden = true;
  sheet.setAttribute('role', 'dialog');
  sheet.setAttribute('aria-label', 'Alarm messages');
  sheet.innerHTML = [
    '<div class="alarm-head"><h2>Alarm messages</h2><span class="grow"></span><button type="button" data-close aria-label="Close">Close</button></div>',
    '<div class="alarm-bar"><select data-device aria-label="Camera"></select><select data-hours aria-label="Window"><option value="24">24 h</option><option value="168">7 days</option></select><span data-window></span></div>',
    '<div class="alarm-list" data-list role="list"></div>',
    '<div class="alarm-foot"><span data-status>not polled yet</span><span class="grow"></span><button type="button" data-alerts>Notify me</button></div>',
  ].join('');

  const topbar = document.querySelector('#watch .topbar');
  const settingsButton = document.getElementById('open-settings');
  if (topbar && settingsButton) topbar.insertBefore(bell, settingsButton);
  else document.body.append(bell);
  document.body.append(sheet);

  const badge = bell.querySelector('.badge');
  const list = sheet.querySelector('[data-list]');
  const status = sheet.querySelector('[data-status]');
  const devicePick = sheet.querySelector('[data-device]');
  const hoursPick = sheet.querySelector('[data-hours]');
  const windowLabel = sheet.querySelector('[data-window]');
  const alertBtn = sheet.querySelector('[data-alerts]');

  const signedIn = () => {
    const watch = document.getElementById('watch');
    return Boolean(watch) && !watch.hidden;
  };

  const fmt = (text) => {
    const match = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})$/.exec(text || '');
    if (!match) return text || '-';
    return match[2] + '-' + match[3] + ' ' + match[4] + ':' + match[5];
  };

  const picUrl = (entry) => {
    if (STATE.photos !== true || STATE.missing.has(entry.id)) return '';
    if (!entry.picture || !entry.picture.object) return '';
    return picUrlOf(entry.picture.object, entry.picture.bucket);
  };

  const probePicture = async () => {
    const serial = STATE.serial;
    if (STATE.photos !== 'unknown' || !serial) return;
    const candidates = [...STATE.entries.values()]
      .filter((entry) => entry.picture && entry.picture.object && !STATE.missing.has(entry.id))
      .sort((a, b) => (b.utcTime || 0) - (a.utcTime || 0))
      .slice(0, 3);
    if (!candidates.length) return;
    STATE.photos = false;
    for (const entry of candidates) {
      let reply;
      try {
        reply = await fetch(picUrlOf(entry.picture.object, entry.picture.bucket), { credentials: 'same-origin' });
      } catch (error) {
        if (STATE.serial === serial) STATE.photos = 'unknown';
        return;
      }
      if (STATE.serial !== serial) return;
      if (reply.ok) {
        STATE.photos = true;
        render();
        return;
      }
      STATE.missing.add(entry.id);
    }
  };

  const picUrlOf = (object, bucket) => '/api/cloud/alarms/picture?sn=' + encodeURIComponent(STATE.serial) + '&object=' + encodeURIComponent(object) + '&bucket=' + encodeURIComponent(bucket || '');

  const render = () => {
    const rows = [...STATE.entries.values()].sort((a, b) => (b.utcTime || 0) - (a.utcTime || 0));
    list.textContent = '';
    if (!rows.length) {
      const empty = document.createElement('p');
      empty.style.cssText = 'padding:14px;font-size:13px;color:var(--dim,#96a0b5)';
      empty.textContent = 'No alarms in this window yet.';
      list.append(empty);
    }
    for (const entry of rows) {
      const row = document.createElement('div');
      const focused = Boolean(STATE.focus) && STATE.focus === entry.id;
      row.className = 'alarm-row' + (focused ? ' focus' : '');
      row.dataset.id = entry.id || '';
      row.setAttribute('role', 'listitem');
      if (focused && STATE.focusShown !== entry.id) {
        STATE.focusShown = entry.id;
        requestAnimationFrame(() => row.scrollIntoView({ block: 'nearest' }));
      }
      const thumb = document.createElement('div');
      thumb.className = 'thumb';
      const src = picUrl(entry);
      if (src) {
        const img = document.createElement('img');
        img.className = 'thumb';
        img.alt = '';
        img.loading = 'lazy';
        img.src = src;
        img.addEventListener('error', () => {
          STATE.missing.add(entry.id);
          render();
        });
        row.append(img);
      } else {
        row.append(thumb);
      }
      const col = document.createElement('div');
      const what = document.createElement('div');
      what.className = 'what';
      what.textContent = entry.name || entry.event || 'alarm';
      const when = document.createElement('div');
      when.className = 'when';
      when.textContent = fmt(entry.time);
      const chan = document.createElement('div');
      chan.className = 'chan';
      chan.textContent = 'channel ' + (entry.channel || '0') + (entry.id ? ' · ' + entry.id : '');
      col.append(what, when, chan);
      row.append(col);
      list.append(row);
    }
    badge.hidden = rows.length === 0;
    badge.textContent = String(rows.length);
  };

  const notify = (entries) => {
    if (desktop()) return;
    if (!STATE.alerts || typeof Notification === 'undefined') return;
    if (Notification.permission !== 'granted') return;
    if (document.visibilityState === 'visible') return;
    for (const entry of entries) {
      if (STATE.seen.has(entry.id)) continue;
      STATE.seen.add(entry.id);
      try {
        const note = new Notification('Alarm · ' + (entry.name || entry.event || 'camera'), {
          body: (entry.time || '') + ' · channel ' + (entry.channel || '0'),
          tag: 'pvw-alarm-' + entry.id,
        });
        note.onclick = () => window.focus();
      } catch (error) {
        return;
      }
      return;
    }
  };

  const poll = async () => {
    if (STATE.busy || !signedIn() || !STATE.serial) return;
    if (document.hidden && !STATE.alerts) return;
    STATE.busy = true;
    const started = performance.now();
    try {
      const until = new Date();
      const since = new Date(until.getTime() - STATE.hours * 3600 * 1000);
      const local = (when) => when.getFullYear() + '-' + String(when.getMonth() + 1).padStart(2, '0') + '-' + String(when.getDate()).padStart(2, '0') + ' ' + String(when.getHours()).padStart(2, '0') + ':' + String(when.getMinutes()).padStart(2, '0') + ':' + String(when.getSeconds()).padStart(2, '0');
      const url = '/api/cloud/alarms?sn=' + encodeURIComponent(STATE.serial) + '&start=' + encodeURIComponent(local(since)) + '&end=' + encodeURIComponent(local(until));
      const reply = await fetch(url, { credentials: 'same-origin' });
      const body = await reply.json().catch(() => ({}));
      if (reply.status === 401) {
        status.textContent = body.message || 'alarms read the vendor cloud; sign in with the cloud account';
        return;
      }
      if (!reply.ok) {
        status.textContent = body.message || ('alarm centre answered ' + reply.status);
        return;
      }
      STATE.pollSeconds = body.pollSeconds || STATE.pollSeconds;
      const fresh = [];
      const next = new Map();
      for (const entry of body.entries || []) {
        next.set(entry.id, entry);
        if (!STATE.seen.has(entry.id)) fresh.push(entry);
      }
      const first = STATE.entries.size === 0 && STATE.last === null;
      STATE.entries = next;
      STATE.last = Date.now();
      if (!first) notify(fresh);
      for (const entry of next.values()) STATE.seen.add(entry.id);
      render();
      if (STATE.photos === 'unknown') probePicture();
      const roundTrip = Math.round(performance.now() - started);
      status.textContent = (body.total || next.size) + ' alarms · vendor ' + Math.round(body.latencyMs || 0) + ' ms · here ' + roundTrip + ' ms · every ' + STATE.pollSeconds + ' s';
      windowLabel.textContent = body.finished === false ? STATE.hours + 'h · more to page' : STATE.hours + 'h window';
    } catch (error) {
      status.textContent = 'alarm poll failed: ' + error;
    } finally {
      STATE.busy = false;
      schedule();
    }
  };

  const schedule = () => {
    clearTimeout(STATE.timer);
    STATE.timer = 0;
    if (!signedIn()) return;
    if (document.hidden && !STATE.alerts) return;
    STATE.timer = setTimeout(poll, Math.max(5, STATE.pollSeconds) * 1000);
  };

  const loadDevices = async () => {
    try {
      const reply = await fetch('/api/devices', { credentials: 'same-origin' });
      if (!reply.ok) return;
      STATE.devices = await reply.json();
      devicePick.textContent = '';
      for (const device of STATE.devices) {
        const option = document.createElement('option');
        option.value = device.id;
        option.textContent = device.name || device.id;
        devicePick.append(option);
      }
      const chosen = STATE.devices.find((device) => device.id === STATE.serial) || STATE.devices[0];
      STATE.serial = chosen ? chosen.id : '';
      if (chosen) devicePick.value = chosen.id;
    } catch (error) {
      status.textContent = 'device list unavailable: ' + error;
    }
  };

  const open = async () => {
    sheet.hidden = false;
    if (!STATE.devices.length) await loadDevices();
    if (!STATE.last) poll();
  };

  const focusAlarm = (detail) => {
    const alarm = detail || {};
    if (alarm.serial && alarm.serial !== STATE.serial) {
      STATE.serial = alarm.serial;
      STATE.entries = new Map();
      STATE.seen = new Set();
      STATE.missing = new Set();
      STATE.photos = 'unknown';
      STATE.last = null;
      STATE.devices = [];
    }
    STATE.focus = alarm.id || '';
    STATE.focusShown = '';
    open();
  };

  const close = () => {
    sheet.hidden = true;
  };

  bell.addEventListener('click', () => (sheet.hidden ? open() : close()));
  sheet.querySelector('[data-close]').addEventListener('click', close);
  hoursPick.addEventListener('change', () => {
    STATE.hours = Number(hoursPick.value) || 24;
    poll();
  });
  devicePick.addEventListener('change', () => {
    STATE.serial = devicePick.value;
    STATE.entries = new Map();
    STATE.seen = new Set();
    STATE.missing = new Set();
    STATE.photos = 'unknown';
    STATE.last = null;
    poll();
  });
  if (desktop()) {
    alertBtn.hidden = true;
    alertBtn.title = 'Alarm notifications are set in Settings, Desktop';
  }
  alertBtn.addEventListener('click', async () => {
    if (typeof Notification === 'undefined') {
      status.textContent = 'this browser has no notification support';
      return;
    }
    const permission = await Notification.requestPermission();
    STATE.alerts = permission === 'granted';
    alertBtn.textContent = STATE.alerts ? 'Notifications on' : 'Notifications blocked';
    schedule();
  });

  const sync = async () => {
    const on = signedIn();
    bell.hidden = !on;
    if (!on) {
      close();
      clearTimeout(STATE.timer);
      STATE.timer = 0;
      STATE.devices = [];
      STATE.entries = new Map();
      STATE.seen = new Set();
      STATE.missing = new Set();
      STATE.last = null;
      STATE.photos = 'unknown';
      return;
    }
    if (!STATE.devices.length) await loadDevices();
    if (!STATE.timer) schedule();
  };

  const watch = document.getElementById('watch');
  if (watch) {
    new MutationObserver(sync).observe(watch, { attributes: true, attributeFilter: ['hidden'] });
  }
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && signedIn()) poll();
    else schedule();
  });
  if (desktop()) {
    window.__TAURI__.event.listen('desktop://alarm', (event) => focusAlarm(event.payload)).catch(() => {});
  }
  window.addEventListener('load', sync);
  if (document.readyState !== 'loading') sync();
})();
