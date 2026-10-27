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
    last: null,
  };

  const css = document.createElement('style');
  css.textContent = [
    '.alarm-fab{position:fixed;right:18px;bottom:18px;z-index:40;display:flex;align-items:center;gap:8px;padding:10px 14px;border-radius:999px;border:1px solid var(--line,#2c3140);background:var(--bg-2,#171a22);color:inherit;font:inherit;cursor:pointer;box-shadow:0 8px 24px rgba(0,0,0,.35)}',
    '.alarm-fab[hidden]{display:none}',
    '.alarm-fab .badge{min-width:20px;height:20px;border-radius:999px;background:#e0533d;color:#fff;font-size:11px;line-height:20px;text-align:center;padding:0 5px}',
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
    '.alarm-foot{margin-top:auto;border-top:1px solid var(--line,#2c3140);padding:8px 14px;font-size:11px;color:var(--dim,#96a0b5);display:flex;gap:8px;align-items:center}',
    '.alarm-foot .grow{flex:1}',
  ].join('\n');
  document.head.appendChild(css);

  const bell = document.createElement('button');
  bell.type = 'button';
  bell.className = 'alarm-fab';
  bell.hidden = true;
  bell.setAttribute('aria-haspopup', 'dialog');
  bell.innerHTML = '<span aria-hidden="true">&#128276;</span><span>Alarms</span><span class="badge" hidden>0</span>';

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

  document.body.append(bell, sheet);

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
    if (STATE.photos !== true || !entry.picture || !entry.picture.object) return '';
    return picUrlOf(entry.picture.object, entry.picture.bucket);
  };

  const probePicture = async () => {
    if (STATE.photos !== 'unknown' || !STATE.serial) return;
    const newest = [...STATE.entries.values()].sort((a, b) => (b.utcTime || 0) - (a.utcTime || 0))[0];
    if (!newest || !newest.picture || !newest.picture.object) {
      STATE.photos = false;
      return;
    }
    STATE.photos = false;
    try {
      const reply = await fetch(picUrlOf(newest.picture.object, newest.picture.bucket), { credentials: 'same-origin' });
      STATE.photos = reply.ok;
    } catch (error) {
      STATE.photos = false;
    }
    if (STATE.photos) render();
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
      row.className = 'alarm-row';
      row.setAttribute('role', 'listitem');
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
          STATE.photos = false;
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
    STATE.last = null;
    poll();
  });
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
  window.addEventListener('load', sync);
  if (document.readyState !== 'loading') sync();
})();
