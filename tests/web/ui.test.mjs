import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP, fakeTalkDevice, toneWave } from './cdp.mjs';

const DEVICES = [
  { id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 },
  { id: 'porch', name: 'Porch', model: 'XM-IPC', online: true, channels: 1 },
];
const CLIPS = Object.fromEntries(DEVICES.flatMap((device) => [[device.id + '_ch1_main.h264', DUAL_LENS_CLIP], [device.id + '_ch1_sub.h264', DUAL_LENS_CLIP]]));
const TALK_CAPS = {
  patterns: [{ urlPattern: '*/api/device/controls*' }],
  respond: () => ({
    onvif: false,
    dvrip: true,
    capabilities: [{ feature: 'two-way audio', lens: 'device', backend: 'dvrip', state: 'supported', evidence: 'test row' }],
  }),
};

let server;
let browser;

let tone;

before(async () => {
  tone = toneWave(join(tmpdir(), 'proview-talk-tone.wav'));
  server = await startServer(DEVICES, { clips: CLIPS });
  browser = await startBrowser(['--use-file-for-fake-audio-capture=' + tone]);
});

after(async () => {
  try {
    await browser?.close();
  } finally {
    await server?.stop();
  }
});

async function open(options = {}) {
  const page = await browser.page(options);
  await signIn(page, server.base);
  await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the first camera to play', 20000);
  return page;
}

const active = "document.activeElement && (document.activeElement.id || document.activeElement.querySelector('.name')?.textContent)";
const menuShown =
  "(() => { const menu = document.getElementById('camera-menu'); return Boolean(menu) && getComputedStyle(menu).display !== 'none' && menu.getBoundingClientRect().height > 0; })()";

test('camera menu: arrows rove and wrap, Escape returns focus to the switcher', async () => {
  const page = await open();
  try {
    await page.evaluate("document.getElementById('switcher').focus()");
    await page.key('Enter');
    await page.wait(menuShown, 'the camera menu to show');
    assert.equal(await page.evaluate("document.getElementById('switcher').getAttribute('aria-expanded')"), 'true');
    assert.equal(await page.evaluate(active), 'Gate camera');
    await page.key('ArrowDown');
    assert.equal(await page.evaluate(active), 'Porch');
    await page.key('End');
    assert.equal(await page.evaluate(active), 'Add a camera');
    await page.key('ArrowDown');
    assert.equal(await page.evaluate(active), 'Gate camera');
    await page.key('ArrowUp');
    assert.equal(await page.evaluate(active), 'Add a camera');
    await page.key('Escape');
    await page.wait('!' + menuShown, 'the camera menu to close');
    await page.wait("document.activeElement && document.activeElement.id === 'switcher'", 'focus back on the switcher');
    assert.equal(await page.evaluate("document.getElementById('switcher').getAttribute('aria-expanded')"), 'false');
    assert.equal(await page.evaluate("document.getElementById('watch').hidden"), false, 'Escape inside the menu must not leave Live');
  } finally {
    await page.close();
  }
});

test('camera menu: picking a row switches the camera and a click outside closes it', async () => {
  const page = await open();
  try {
    await page.evaluate("document.getElementById('switcher').click()");
    await page.wait(menuShown, 'the camera menu to show');
    await page.key('ArrowDown');
    await page.key('Enter');
    await page.wait("document.getElementById('switcher-name').textContent === 'Porch'", 'the switch to Porch');
    await page.wait("document.activeElement && document.activeElement.id === 'switcher'", 'focus back on the switcher');
    await page.evaluate("document.getElementById('switcher').click()");
    await page.wait(menuShown, 'the camera menu to reopen');
    await page.call('Input.dispatchMouseEvent', { type: 'mousePressed', x: 700, y: 400, button: 'left', clickCount: 1 });
    await page.call('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 700, y: 400, button: 'left', clickCount: 1 });
    await page.wait('!' + menuShown, 'the camera menu to close on an outside click');
    assert.equal(await page.evaluate("document.getElementById('switcher').getAttribute('aria-expanded')"), 'false');
  } finally {
    await page.close();
  }
});

test('shortcuts switch the camera and the view, and stay out of text fields', async () => {
  const page = await open();
  try {
    await page.key('2');
    await page.wait("document.getElementById('switcher-name').textContent === 'Porch'", 'digit 2 to open the second camera');
    await page.key('g');
    await page.wait("!document.getElementById('grid').hidden && document.querySelectorAll('.tile').length === 2", 'g to show every camera');
    await page.key('g');
    await page.wait("!document.getElementById('live').hidden", 'g to return to Live');
    await page.evaluate("const input = document.createElement('input'); input.id = 'probe'; document.getElementById('watch').append(input); input.focus()");
    await page.key('1');
    await page.key('g');
    assert.equal(await page.evaluate("document.getElementById('switcher-name').textContent"), 'Porch');
    assert.equal(await page.evaluate("document.getElementById('grid').hidden"), true);
    assert.equal(await page.evaluate("document.getElementById('probe').value"), '1g');
  } finally {
    await page.close();
  }
});

test('? opens the keyboard list without moving focus out of reach, Escape closes it', async () => {
  const page = await open();
  try {
    await page.key('?');
    await page.wait("getComputedStyle(document.getElementById('keys')).display !== 'none'", 'the keyboard list');
    assert.equal(await page.evaluate(active), 'keys-close');
    await page.key('Escape');
    await page.wait("getComputedStyle(document.getElementById('keys')).display === 'none'", 'the keyboard list to close');
    assert.equal(await page.evaluate("document.getElementById('watch').hidden"), false);
  } finally {
    await page.close();
  }
});

test('holding T carries the microphone to the camera speaker', async () => {
  const device = await fakeTalkDevice();
  const talking = await startServer(DEVICES, {
    clips: CLIPS,
    env: { PROVIEW_DEVICE_HOST: '127.0.0.1:' + device.port, PROVIEW_DEVICE_PASS: 'hunter2', PROVIEW_DEVICE_SERIAL: 'gate' },
  });
  const page = await browser.page({ intercept: TALK_CAPS });
  try {
    await signIn(page, talking.base);
    await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the first camera to play', 20000);
    await page.wait("!document.getElementById('talk').disabled", 'the Talk button to enable');
    await page.key('t', 'down');
    await page.wait("document.getElementById('talk').classList.contains('talk-on')", 'talk to start');
    assert.match(await page.evaluate("document.getElementById('caption-state').textContent"), /microphone is open/);
    await page.wait("Number(document.getElementById('level').dataset.level) > 0", 'the microphone level to move');
    await page.wait("window.proviewTalk.state().chunks >= 25", 'a second of microphone audio to go up', 15000);
    await page.wait("window.proviewTalk.state().mic === 1", 'the microphone track to be live');
    await page.wait(
      "/^sending · \\d+ frames · \\d+ G\\.711 bytes · /.test(document.getElementById('talk-note').textContent)",
      'the byte and latency report to reach the page',
      8000,
    );
    await page.key('t', 'up');
    await page.wait("!document.getElementById('talk').classList.contains('talk-on')", 'talk to stop');
    assert.equal(await page.evaluate("document.getElementById('level').hidden"), true);
    assert.doesNotMatch(await page.evaluate("document.getElementById('caption-state').textContent"), /microphone/);
    await page.wait("window.proviewTalk.state().mic === 0", 'the microphone track to be released');
    assert.match(await page.evaluate("document.getElementById('talk-note').textContent"), /^talk closed · the direct address · \d+ chunks · \d+ PCM16 bytes/);
    const frames = device.frames();
    assert.ok(frames.length > 15, 'frames reached the camera: ' + frames.length);
    assert.equal(frames[0].id, 1432);
    assert.equal(frames[0].header, '000001fa0e024001');
    assert.ok(frames.some((frame) => frame.silent === false), 'at least one frame carries audio');
  } finally {
    await page.close();
    await talking.stop();
    device.close();
  }
});

test('Talk says why it cannot open when the camera is not reachable', async () => {
  const page = await open({ intercept: TALK_CAPS });
  try {
    await page.wait("!document.getElementById('talk').disabled", 'the Talk button to enable');
    await page.key('t', 'down');
    await page.wait(
      "(() => { const note = document.getElementById('talk-note'); return Boolean(note) && /talk failed: (no path to the camera|the camera password is not set|no camera serial is configured)/.test(note.textContent); })()",
      'the server reason to reach the page',
    );
    assert.equal(await page.evaluate("document.getElementById('talk').classList.contains('talk-on')"), false);
    assert.equal(await page.evaluate("document.getElementById('level').hidden"), true);
    await page.key('t', 'up');
  } finally {
    await page.close();
  }
});

test('Talk stays disabled with the server reason when two-way audio is not reported', async () => {
  const page = await open();
  try {
    assert.equal(await page.evaluate("document.getElementById('talk').disabled"), true);
    assert.notEqual(await page.evaluate("document.getElementById('talk').title"), '');
    const reason = await page.evaluate("document.getElementById('talk-note').textContent");
    assert.ok(reason.length > 20, 'the reason is on screen, not only in the tooltip: ' + reason);
    assert.equal(reason, await page.evaluate("document.getElementById('talk').title"));
    await page.key('t', 'down');
    assert.equal(await page.evaluate("document.getElementById('talk').classList.contains('talk-on')"), false);
    await page.key('t', 'up');
  } finally {
    await page.close();
  }
});

const ATTRIBUTE_STUB = {
  patterns: [
    { urlPattern: '*/api/cloud/device-attributes' },
    { urlPattern: '*/api/cloud/device/group' },
    { urlPattern: '*/api/cloud/device/favourite' },
  ],
  respond: (url) => {
    if (url.includes('device-attributes')) {
      return {
        ok: true,
        groups: ['pintu'],
        devices: [
          { uuid: 'AASN000000000001', nickname: 'Pintu keluar masuk', group: 'pintu', favourite: false, shared_accounts: 1, administrator: true, extension: 'just_test_ext' },
          { uuid: '0123456789abcdef0123', nickname: 'Teras', group: '', favourite: true, shared_accounts: 0, administrator: false, extension: '' },
        ],
      };
    }
    return { ok: true, message: 'stub saved the group' };
  },
};

test('the devices panel lists the account devices and saves a group through the API', async () => {
  const page = await open({ intercept: ATTRIBUTE_STUB });
  try {
    await page.evaluate("document.getElementById('open-settings').click()");
    await page.wait("Boolean(document.querySelector('[data-testid=settings-cameras]:not(:disabled)'))", 'the Manage cameras button in Settings');
    await page.evaluate("document.querySelector('[data-testid=settings-cameras]').click()");
    await page.wait("document.querySelectorAll('#device-admin-list .menu-row').length === 2", 'the device rows', 10000);
    const rows = await page.evaluate("[...document.querySelectorAll('#device-admin-list .menu-row .name')].map((node) => node.textContent)");
    assert.deepEqual(rows, ['Pintu keluar masuk', 'Teras']);
    const tags = await page.evaluate("[...document.querySelectorAll('#device-admin-list .menu-row .sub')].map((node) => node.textContent)");
    assert.match(tags[0], /pintu/);
    assert.match(tags[0], /administrator/);
    assert.match(tags[0], /1 shared/);
    assert.match(tags[1], /favourite/);

    await page.evaluate("(() => { const input = document.getElementById('device-admin-group'); input.value = 'teras'; input.dispatchEvent(new Event('input')); })()");
    await page.evaluate("[...document.querySelectorAll('#device-admin button')].find((button) => button.textContent.includes('Save group')).click()");
    await page.wait("document.getElementById('device-admin-status').textContent === 'Stub saved the group'", 'the save to come back', 10000);
    assert.equal(await page.evaluate("document.getElementById('device-admin').hidden"), false);
  } finally {
    await page.close();
  }
});

const tourCalls = [];
const TOUR_CAPS = {
  patterns: [{ urlPattern: '*/api/device/controls*' }],
  respond: (url) => {
    const parsed = new URL(url);
    if (parsed.pathname === '/api/device/controls') {
      return {
        onvif: false,
        dvrip: true,
        capabilities: [
          { feature: 'ptz move', lens: 'bottom', backend: 'dvrip', state: 'supported', evidence: 'DirectionRight on PTZ channel 0 shifted the lower lens' },
          { feature: 'ptz patrol', lens: 'bottom', backend: 'dvrip', state: 'supported', evidence: 'Uart.PTZTour holds 1 line(s) with 2 point(s) in total' },
        ],
      };
    }
    if (parsed.pathname === '/api/device/controls/presets') {
      return [];
    }
    if (parsed.pathname === '/api/device/controls/tour') {
      return {
        ok: true,
        backend: 'dvrip',
        lines: [
          {
            channel: 0,
            id: 1,
            name: '',
            points: [
              { preset: 1, seconds: 3 },
              { preset: 2, seconds: 3 },
            ],
          },
        ],
        presets: [
          { channel: 0, id: 1, name: 'biasa' },
          { channel: 0, id: 2, name: 'biasabaru' },
        ],
        locate: null,
        note: 'the device answers every OPPTZControl with Ret 100 before it moves',
      };
    }
    if (parsed.pathname === '/api/device/controls/tour/start' || parsed.pathname === '/api/device/controls/tour/stop') {
      tourCalls.push(parsed.pathname + parsed.search);
      const action = parsed.pathname.endsWith('start') ? 'start' : 'stop';
      return { ok: true, backend: 'dvrip', action, tour: 1, times: 1, note: 'the device answers every OPPTZControl with Ret 100 before it moves' };
    }
    return {};
  },
};

test('the pad starts and stops the device tour, and reports that the ack is not motion', async () => {
  tourCalls.length = 0;
  const page = await open({ intercept: TOUR_CAPS });
  try {
    await page.evaluate("document.getElementById('move').click()");
    await page.wait("!document.getElementById('pad').hidden", 'the pad to open');
    await page.evaluate("document.getElementById('tour').click()");
    await page.wait("/Tour 1 running over 2 stops/.test(document.getElementById('control-status').textContent)", 'the tour to start');
    assert.equal(await page.evaluate("document.getElementById('tour').getAttribute('aria-pressed')"), 'true');
    assert.match(await page.evaluate("document.getElementById('control-status').textContent"), /watch the picture/);
    await page.evaluate("document.getElementById('tour').click()");
    await page.wait("/Stopped tour 1/.test(document.getElementById('control-status').textContent)", 'the tour to stop');
    assert.equal(await page.evaluate("document.getElementById('tour').getAttribute('aria-pressed')"), 'false');
    assert.deepEqual(tourCalls, ['/api/device/controls/tour/start?tour=1&device=gate', '/api/device/controls/tour/stop?tour=1&device=gate']);
  } finally {
    await page.close();
  }
});
const TOOLBAR_HITS = `(() => {
  const misses = [];
  for (const button of document.querySelectorAll('#controls button')) {
    if (!button.getClientRects().length) continue;
    const box = button.getBoundingClientRect();
    const name = button.id || button.textContent.trim();
    if (box.left < 0 || box.top < 0 || box.right > innerWidth || box.bottom > innerHeight) {
      misses.push(name + ' outside the viewport at ' + [box.left, box.top, box.right, box.bottom].map(Math.round).join(','));
      continue;
    }
    const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
    if (hit?.closest('button') !== button) misses.push(name + ' centre hits ' + (hit?.closest('button')?.id || hit?.tagName));
  }
  const rows = new Set([...document.querySelectorAll('#controls > button')].filter((button) => button.getClientRects().length).map((button) => Math.round(button.getBoundingClientRect().top)));
  if (matchMedia('(max-width: 640px) and (min-height: 481px)').matches && rows.size !== 1) misses.push('the phone dock wraps to ' + rows.size + ' rows');
  return misses;
})()`;
const EXTRA_BUTTON = `(() => {
  const extra = document.getElementById('snapshot').cloneNode(true);
  extra.id = 'extra-ctl';
  extra.querySelector('span').textContent = 'Cloud';
  const controls = document.getElementById('controls');
  controls.insertBefore(extra, controls.querySelector('.spacer'));
})()`;

for (const touch of [false, true]) {
  test('every live control is on screen and takes its own tap at phone widths' + (touch ? ' with touch-sized targets' : ''), async () => {
    for (const width of [360, 390, 480, 600, 700]) {
      const page = await open({ width, height: 844 });
      try {
        await page.wait(width <= 640 ? "Boolean(document.querySelector('[data-testid=pip-inset]'))" : "document.getElementById('lens-pick')?.hidden === false", 'the lens control of the dual-lens clip');
        if (touch) {
          await page.call('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
          await page.call('Emulation.setEmulatedMedia', { features: [{ name: 'pointer', value: 'coarse' }] });
        }
        assert.deepEqual(await page.evaluate(TOOLBAR_HITS), [], 'at ' + width + ' px');
        await page.evaluate(EXTRA_BUTTON);
        assert.deepEqual(await page.evaluate(TOOLBAR_HITS), [], 'at ' + width + ' px with one more control');
      } finally {
        await page.close();
      }
    }
  });
}

const pad = (value) => String(value).padStart(2, '0');
const dayStamp = (ms) => {
  const when = new Date(ms);
  return when.getFullYear() + '-' + pad(when.getMonth() + 1) + '-' + pad(when.getDate()) +
    ' ' + pad(when.getHours()) + ':' + pad(when.getMinutes()) + ':' + pad(when.getSeconds());
};
const PLAYBACK_DAY = (() => {
  const first = new Date(2026, 9, 24, 0, 0, 0).getTime();
  const step = 86400000 / 557;
  return Array.from({ length: 557 }, (_, index) => {
    const from = first + Math.round(index * step);
    const until = index === 556 ? first + 86400000 : first + Math.round((index + 1) * step);
    const begin = dayStamp(from);
    const end = dayStamp(until);
    const kind = index % 3 === 0 ? '[M]' : '[R]';
    return {
      name: '/idea0/2026-10-24/002/' + begin.slice(11) + '-' + end.slice(11) + kind + '[@18][0].h264',
      begin,
      end,
      bytes: (until - from) * 470,
      disk: null,
    };
  });
})();
const PLAYBACK_BODY = {
  ok: true,
  source: 'cloud',
  device: DEVICES[0].id,
  channel: 0,
  begin: '2026-10-24 00:00:00',
  end: '2026-10-24 23:59:59',
  count: PLAYBACK_DAY.length,
  bytes: PLAYBACK_DAY.reduce((total, file) => total + file.bytes, 0),
  truncated: false,
  cap: 2048,
  elapsed_ms: 12,
  files: PLAYBACK_DAY,
};
const GAP_DAY = ['00:00:00-00:05:00', '03:00:00-03:05:00', '06:00:00-06:05:00'].map((span) => ({
  name: '/idea0/2026-10-23/002/' + span + '[R][@18][0].h264',
  begin: '2026-10-23 ' + span.slice(0, 8),
  end: '2026-10-23 ' + span.slice(9),
  bytes: 300 * 470,
  disk: null,
}));
const GAP_BODY = {
  ok: true,
  source: 'cloud',
  device: DEVICES[0].id,
  channel: 0,
  begin: '2026-10-23 00:00:00',
  end: '2026-10-23 23:59:59',
  count: GAP_DAY.length,
  bytes: GAP_DAY.reduce((total, file) => total + file.bytes, 0),
  truncated: false,
  cap: 2048,
  elapsed_ms: 6,
  files: GAP_DAY,
};
const PLAYBACK_FILES = {
  patterns: [{ urlPattern: '*/api/playback/files*' }],
  respond: (url) => (String(url).includes('2026-10-23') ? GAP_BODY : PLAYBACK_BODY),
};
const clockAt = (fraction) => new Date(new Date(2026, 9, 24, 0, 0, 0).getTime() + fraction * 86400000).getTime();

const SOCKET_LOG =
  "(() => { const Real = window.WebSocket; window.__playback = []; window.WebSocket = class extends Real { constructor(url, protocols) { super(url, protocols); if (String(url).includes('/ws/playback')) window.__playback.push(String(url)); } }; })()";
const lastBegin = "(() => { const url = window.__playback[window.__playback.length - 1]; return url ? new URL(url).searchParams.get('begin') : ''; })()";

async function pickDay(page, day) {
  await page.evaluate("(() => { const input = document.getElementById('pb-day'); input.value = '" + day + "'; input.dispatchEvent(new Event('change')); })()");
}

async function clickTrack(page, x, y) {
  for (const type of ['mousePressed', 'mouseReleased']) {
    await page.call('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons: type === 'mousePressed' ? 1 : 0, clickCount: 1 });
  }
}

test('the timeline draws the whole day and plays the file under the pointer', async () => {
  const page = await open({ intercept: PLAYBACK_FILES, init: [SOCKET_LOG] });
  try {
    await page.evaluate("document.getElementById('tl-recordings').click()");
    await page.wait("Boolean(document.getElementById('pb'))", 'the recordings sheet');
    await pickDay(page, '2026-10-24');
    await page.wait("document.querySelectorAll('.pb-row').length === 557", 'the 557 file rows');
    assert.match(await page.evaluate("document.getElementById('pb-summary').textContent"), /^557 files · /);
    await page.evaluate("document.querySelector('#recordings-panel button[aria-label=\"Close\"]').click()");
    await page.wait("!document.getElementById('pb')", 'the recordings sheet to close');
    await page.wait("document.querySelectorAll('[data-testid=tl-bar]').length === 1", 'the day drawn as one recorded span');
    const span = await page.evaluate("(() => { const bar = document.querySelector('[data-testid=tl-bar]').getBoundingClientRect(); const track = document.querySelector('[data-testid=tl-track]').getBoundingClientRect(); return { left: bar.left - track.left, right: track.right - bar.right, width: track.width }; })()");
    assert.ok(span.left < 1 && span.right < 1, 'the 557 files cover the whole track: ' + JSON.stringify(span));

    const box = await page.evaluate("(() => { const track = document.querySelector('[data-testid=tl-track]').getBoundingClientRect(); return { left: track.left, top: track.top, width: track.width, height: track.height }; })()");
    const inside = (entry, ms) => Date.parse(entry.begin.replace(' ', 'T')) <= ms && Date.parse(entry.end.replace(' ', 'T')) > ms;
    for (const [index, fraction] of [0.2, 0.63].entries()) {
      const x = Math.round(box.left + box.width * fraction);
      const at = clockAt((x - box.left) / box.width);
      const file = PLAYBACK_DAY.find((entry) => inside(entry, at));
      await clickTrack(page, x, box.top + box.height / 2);
      await page.wait('window.__playback.length === ' + (index + 1), 'a playback socket for the click at ' + fraction);
      const url = new URL(await page.evaluate('window.__playback[window.__playback.length - 1]'));
      assert.equal(url.searchParams.get('mode'), 'time');
      assert.equal(url.searchParams.get('device'), DEVICES[0].id);
      const begin = Date.parse(url.searchParams.get('begin').replace(' ', 'T'));
      assert.ok(begin <= at && at - begin < 1000, 'playback starts at the clicked second: ' + url.searchParams.get('begin') + ' for ' + new Date(at).toTimeString());
      assert.ok(file && inside(file, begin), 'the file under the pointer plays: ' + (file && file.name) + ' from ' + url.searchParams.get('begin'));
      await page.wait("document.querySelector('[data-testid=playback-controls]') !== null", 'the playback controls in the dock');
    }

    await page.evaluate("document.querySelector('[data-testid=tl-recordings]').click()");
    await page.wait("Boolean(document.getElementById('pb'))", 'the recordings sheet again');
    await pickDay(page, '2026-10-23');
    await page.wait("document.querySelectorAll('.pb-row').length === 3", 'the sparse day');
    await page.evaluate("document.querySelector('#recordings-panel button[aria-label=\"Close\"]').click()");
    await page.wait("document.querySelectorAll('[data-testid=tl-bar]').length === 3", 'three recorded spans');
    const gap = await page.evaluate("(() => { const first = document.querySelector('[data-testid=tl-bar]').getBoundingClientRect(); return { x: first.right + 40, y: first.top + first.height / 2 }; })()");
    const sockets = await page.evaluate('window.__playback.length');
    await clickTrack(page, gap.x, gap.y);
    await page.wait('window.__playback.length === ' + (sockets + 1), 'a playback socket for the gap click');
    assert.equal(await page.evaluate(lastBegin), GAP_DAY[1].begin, 'a click in a gap plays the recording that follows it');

    await page.evaluate("document.querySelector('[data-testid=tl-recordings]').click()");
    await page.wait("Boolean(document.getElementById('pb'))", 'the recordings sheet once more');
    await pickDay(page, '2026-10-24');
    await page.wait("document.querySelectorAll('.pb-row').length === 557", 'the dense day again');
    await page.evaluate("document.querySelector('#pb button[aria-label=\"Next day\"]').click()");
    const loaded = "document.querySelector('[data-testid=pb-refresh]').getAttribute('aria-busy') !== 'true' && document.querySelectorAll('.pb-row').length === 557";
    await page.wait("document.getElementById('pb-day').value === '2026-10-25' && " + loaded, 'the next day rows');
    await page.evaluate("document.querySelector('#pb button[aria-label=\"Previous day\"]').click()");
    await page.wait("document.getElementById('pb-day').value === '2026-10-24' && " + loaded, 'the rows of the day before');

    const opened = await page.evaluate('window.__playback.length');
    await page.evaluate("(() => { const row = Array.from(document.querySelectorAll('.pb-row')).find((node) => node.textContent.trim().indexOf('00:00:00') === 0); row.click(); })()");
    await page.wait('window.__playback.length === ' + (opened + 1), 'the first clip');
    assert.equal(await page.evaluate(lastBegin), '2026-10-24 00:00:00');
    await page.wait("!document.getElementById('pb')", 'the recordings sheet to close after the row click');
    assert.match(await page.evaluate("document.querySelector('[data-testid=tl-day]').getAttribute('aria-label')"), /24 Oct/, 'the day stays the same after a row click');
    await page.evaluate("document.querySelector('[data-testid=tl-recordings]').click()");
    await page.wait("Boolean(document.getElementById('pb-day'))", 'the recordings sheet to reopen');
    assert.equal(await page.evaluate("document.getElementById('pb-day').value"), '2026-10-24');
  } finally {
    await page.close();
  }
});
