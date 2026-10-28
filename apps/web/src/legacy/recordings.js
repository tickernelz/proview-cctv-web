(function () {
  'use strict';


  const panel = document.createElement('section');
  panel.id = 'recordings-panel';
  panel.hidden = true;
  panel.setAttribute('aria-label', 'Cloud recordings');
  panel.innerHTML = [
    '<header><strong>Recordings</strong><button class="iconbtn" type="button" data-close aria-label="Close recordings"><svg class="i" aria-hidden="true"><use href="#i-x"></use></svg></button></header>',
    '<form data-form><label>Day <input type="date" name="day" required></label>',
    '<label>Camera <select name="device" aria-label="Camera"></select></label>',
    '<button class="btn btn-fill" type="submit">Load</button></form>',
    '<p data-status role="status">Choose a day and load the cloud list.</p>',
    '<ul data-list></ul>'
  ].join('');
  document.body.appendChild(panel);

  const controls = document.getElementById('controls');
  const launcher = document.createElement('button');
  launcher.type = 'button';
  launcher.id = 'recordings-launcher';
  launcher.className = 'ctl';
  launcher.setAttribute('aria-pressed', 'false');
  launcher.setAttribute('aria-controls', 'recordings-panel');
  launcher.innerHTML = '<svg class="i" aria-hidden="true"><use href="#i-cloud"></use></svg><span>Cloud</span>';
  if (!controls) return;
  const spacer = controls.querySelector('.spacer');
  controls.insertBefore(launcher, spacer || null);

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
    launcher.setAttribute('aria-pressed', open ? 'true' : 'false');
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
