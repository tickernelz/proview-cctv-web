(function () {
  'use strict';

  const style = document.createElement('style');
  style.textContent = [
    '#recordings-launcher{position:fixed;right:16px;bottom:16px;z-index:60;padding:10px 16px;border-radius:999px;border:0;',
    'background:var(--live,#b3123c);color:#fff;font:inherit;font-weight:600;cursor:pointer;box-shadow:var(--shadow-menu,0 6px 20px rgba(0,0,0,.28))}',
    '#recordings-launcher:focus-visible{outline:2px solid var(--focus,#fff);outline-offset:2px}',
    '#recordings-panel{position:fixed;right:16px;bottom:72px;z-index:61;width:min(420px,calc(100vw - 32px));max-height:min(70vh,560px);',
    'overflow:auto;padding:16px;border-radius:16px;background:var(--bg,#fff);color:var(--ink,#111);border:1px solid var(--line,#d0d0d0);',
    'box-shadow:var(--shadow-menu,0 18px 48px rgba(0,0,0,.35));font:inherit}',
    '#recordings-panel header{display:flex;align-items:center;justify-content:space-between;margin-bottom:10px}',
    '#recordings-panel form{display:grid;gap:8px;margin-bottom:10px}',
    '#recordings-panel label{display:grid;gap:4px;font-size:.85rem}',
    '#recordings-panel input,#recordings-panel select,#recordings-panel button{font:inherit;padding:6px 8px;border-radius:8px;border:1px solid var(--line,#c8c8c8);background:var(--bg,#fff);color:var(--ink,#111)}',
    '#recordings-panel button[type=submit]{background:var(--live,#b3123c);color:#fff;border-color:transparent;cursor:pointer}',
    '#recordings-panel ul{list-style:none;margin:0;padding:0;display:grid;gap:6px}',
    '#recordings-panel li{display:flex;justify-content:space-between;gap:10px;padding:8px;border-radius:8px;background:var(--line-soft,rgba(0,0,0,.05));font-variant-numeric:tabular-nums}',
    '#recordings-panel li span:last-child{opacity:.8}'
  ].join('');
  document.head.appendChild(style);

  const panel = document.createElement('section');
  panel.id = 'recordings-panel';
  panel.hidden = true;
  panel.setAttribute('aria-label', 'Cloud recordings');
  panel.innerHTML = [
    '<header><strong>Recordings</strong><button type="button" data-close aria-label="Close recordings">×</button></header>',
    '<form data-form><label>Day <input type="date" name="day" required></label>',
    '<label>Camera <select name="device" aria-label="Camera"></select></label>',
    '<button type="submit">Load</button></form>',
    '<p data-status role="status">Choose a day and load the cloud list.</p>',
    '<ul data-list></ul>'
  ].join('');
  document.body.appendChild(panel);

  const launcher = document.createElement('button');
  launcher.type = 'button';
  launcher.id = 'recordings-launcher';
  launcher.textContent = 'Recordings';
  document.body.appendChild(launcher);

  const form = panel.querySelector('[data-form]');
  const dayInput = form.elements.day;
  const deviceSelect = form.elements.device;
  const status = panel.querySelector('[data-status]');
  const list = panel.querySelector('[data-list]');

  dayInput.value = localDay();

  function localDay() {
    const now = new Date();
    const month = String(now.getMonth() + 1).padStart(2, '0');
    const day = String(now.getDate()).padStart(2, '0');
    return now.getFullYear() + '-' + month + '-' + day;
  }

  function show(open) {
    panel.hidden = !open;
    if (open) {
      loadDevices().catch(function (error) {
        status.textContent = error.message;
      });
    }
  }

  launcher.addEventListener('click', function () {
    show(panel.hidden);
  });
  panel.querySelector('[data-close]').addEventListener('click', function () {
    show(false);
  });

  let devicesLoaded = false;
  async function loadDevices() {
    if (devicesLoaded) return;
    const response = await fetch('/api/devices', { credentials: 'same-origin' });
    if (!response.ok) throw new Error('camera list failed: ' + response.status);
    const devices = await response.json();
    deviceSelect.innerHTML = '';
    devices.forEach(function (camera) {
      const option = document.createElement('option');
      option.value = camera.id || '';
      option.textContent = camera.name || option.value;
      deviceSelect.appendChild(option);
    });
    devicesLoaded = true;
  }

  function size(bytes) {
    if (!bytes) return '—';
    if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + ' MB';
    if (bytes >= 1024) return Math.round(bytes / 1024) + ' KB';
    return bytes + ' B';
  }

  function duration(seconds) {
    if (!seconds) return '—';
    const minutes = Math.floor(seconds / 60);
    const rest = seconds % 60;
    return minutes + 'm ' + String(rest).padStart(2, '0') + 's';
  }

  function render(page) {
    list.innerHTML = '';
    if (!page.recordings || page.recordings.length === 0) {
      status.textContent = page.notice || 'The cloud returned no clips for that day.';
      return;
    }
    status.textContent = page.recordings.length + ' clip(s)';
    page.recordings.forEach(function (item) {
      const entry = document.createElement('li');
      const start = document.createElement('span');
      start.textContent = item.start;
      const meta = document.createElement('span');
      meta.textContent = duration(item.duration_secs) + ' · ' + size(item.size_bytes);
      entry.appendChild(start);
      entry.appendChild(meta);
      list.appendChild(entry);
    });
  }

  form.addEventListener('submit', async function (event) {
    event.preventDefault();
    status.textContent = 'Loading…';
    list.innerHTML = '';
    const query = new URLSearchParams({ day: dayInput.value, channel: '0' });
    if (deviceSelect.value) query.set('device', deviceSelect.value);
    try {
      const response = await fetch('/api/recordings?' + query.toString(), {
        credentials: 'same-origin'
      });
      const text = await response.text();
      if (!response.ok) {
        let message = text;
        try {
          message = JSON.parse(text).message || text;
        } catch (ignored) {}
        throw new Error(message || 'recordings failed: ' + response.status);
      }
      render(JSON.parse(text));
    } catch (error) {
      status.textContent = error.message;
    }
  });
})();
