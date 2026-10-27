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
const NO_POPOVER = 'delete HTMLElement.prototype.showPopover; delete HTMLElement.prototype.hidePopover; delete HTMLElement.prototype.togglePopover;';
const TALK_CAPS = {
  patterns: [{ urlPattern: '*/api/device/controls' }],
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
  "(() => { const menu = document.getElementById('camera-menu'); return getComputedStyle(menu).display !== 'none' && menu.getBoundingClientRect().height > 0; })()";

for (const [label, init, mode] of [
  ['native popover', [], 'native'],
  ['fallback without the Popover API', [NO_POPOVER], 'fallback'],
]) {
  test('camera menu with ' + label + ': arrows rove and wrap, Escape returns focus to the switcher', async () => {
    const page = await open({ init });
    try {
      assert.equal(await page.evaluate('document.documentElement.dataset.popover'), mode);
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
      assert.equal(await page.evaluate(active), 'switcher');
      assert.equal(await page.evaluate("document.getElementById('switcher').getAttribute('aria-expanded')"), 'false');
      assert.equal(await page.evaluate("document.getElementById('watch').hidden"), false, 'Escape inside the menu must not leave Live');
    } finally {
      await page.close();
    }
  });

  test('camera menu with ' + label + ': picking a row switches the camera and a click outside closes it', async () => {
    const page = await open({ init });
    try {
      await page.evaluate("document.getElementById('switcher').click()");
      await page.wait(menuShown, 'the camera menu to show');
      await page.key('ArrowDown');
      await page.key('Enter');
      await page.wait("document.getElementById('switcher-name').textContent === 'Porch'", 'the switch to Porch');
      assert.equal(await page.evaluate(active), 'switcher');
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
}

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
    env: { PROVIEW_DEVICE_HOST: '127.0.0.1:' + device.port, PROVIEW_DEVICE_PASS: 'hunter2' },
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
    await page.key('t', 'up');
    await page.wait("!document.getElementById('talk').classList.contains('talk-on')", 'talk to stop');
    assert.equal(await page.evaluate("document.getElementById('level').hidden"), true);
    assert.doesNotMatch(await page.evaluate("document.getElementById('caption-state').textContent"), /microphone/);
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
      "document.getElementById('control-status').textContent.includes('did not open')",
      'the server reason',
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
    await page.key('t', 'down');
    assert.equal(await page.evaluate("document.getElementById('talk').classList.contains('talk-on')"), false);
    await page.key('t', 'up');
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
    assert.deepEqual(tourCalls, ['/api/device/controls/tour/start?tour=1', '/api/device/controls/tour/stop?tour=1']);
  } finally {
    await page.close();
  }
});

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
const PLAYBACK_FILES = {
  patterns: [{ urlPattern: '*/api/playback/files*' }],
  respond: () => PLAYBACK_BODY,
};
const clockAt = (fraction) => new Date(new Date(2026, 9, 24, 0, 0, 0).getTime() + fraction * 86400000).getTime();

test('the playback overlay draws the whole day and plays the file under the pointer', async () => {
  const page = await open({ intercept: PLAYBACK_FILES });
  try {
    await page.evaluate("document.getElementById('pb-open').click()");
    await page.wait("!document.getElementById('pb').hidden", 'the playback overlay');
    await page.evaluate("(() => { const day = document.getElementById('pb-day'); day.value = '2026-10-24'; day.dispatchEvent(new Event('change')); })()");
    await page.wait("document.querySelectorAll('.pb-row').length === 557", 'the 557 file rows');
    assert.equal(await page.evaluate("document.querySelectorAll('.pb-seg').length"), 557);
    assert.match(await page.evaluate("document.getElementById('pb-summary').textContent"), /^557 files · /);

    const box = await page.evaluate("(() => { const track = document.getElementById('pb-timeline').getBoundingClientRect(); return { left: track.left, top: track.top, width: track.width, height: track.height }; })()");
    for (const fraction of [0.2, 0.63]) {
      const hit = await page.evaluate(
        "(() => {" +
        " const box = document.getElementById('pb-timeline').getBoundingClientRect();" +
        " const x = box.left + box.width * " + fraction + ";" +
        " const y = box.top + box.height / 2;" +
        " const node = document.elementFromPoint(x, y);" +
        " node.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: x, clientY: y }));" +
        " return node.id || node.className;" +
        " })()",
      );
      assert.equal(hit, 'pb-timeline');
      await page.wait("document.getElementById('pb-playing').textContent.indexOf(' \u2192 ') > 0", 'the playing label after a timeline click');
      const label = await page.evaluate("document.getElementById('pb-playing').textContent");
      const [begin, end] = label.split(' \u2192 ');
      const at = clockAt(fraction);
      assert.ok(Date.parse(begin.replace(' ', 'T')) <= at, 'the clip starts before the clicked time: ' + label);
      assert.ok(Date.parse(end.replace(' ', 'T')) >= at, 'the clip ends after the clicked time: ' + label);
    }

    const before = await page.evaluate("document.getElementById('pb-day').value");
    await page.evaluate("document.querySelector('#pb button[aria-label=\"Next day\"]').click()");
    await page.wait("document.getElementById('pb-day').value === '2026-10-25'", 'the next day');
    await page.wait("document.querySelectorAll('.pb-row').length === 557", 'the next day rows');
    await page.evaluate("document.querySelector('#pb button[aria-label=\"Previous day\"]').click()");
    await page.wait("document.getElementById('pb-day').value === '2026-10-24'", 'the day before');

    await page.evaluate("(() => { const row = Array.from(document.querySelectorAll('.pb-row')).find((node) => node.textContent.indexOf('00:00:00') === 0); row.click(); })()");
    await page.wait("/00:00:00 → /.test(document.getElementById('pb-playing').textContent)", 'the first clip');
    assert.equal(await page.evaluate("document.getElementById('pb-day').value"), before);
  } finally {
    await page.close();
  }
});
