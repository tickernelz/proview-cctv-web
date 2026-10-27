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
      "(() => { const text = document.getElementById('control-status').textContent; return text.includes('no path to the camera') || text.includes('PROVIEW_DEVICE_PASS is empty'); })()",
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
          { uuid: '7f92970ddf5282743hrc', nickname: 'Pintu keluar masuk', group: 'pintu', favourite: false, shared_accounts: 1, administrator: true, extension: 'just_test_ext' },
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
    await page.evaluate("document.getElementById('device-admin-open').click()");
    await page.wait("document.querySelectorAll('#device-admin-list .menu-row').length === 2", 'the device rows', 10000);
    const rows = await page.evaluate("[...document.querySelectorAll('#device-admin-list .menu-row .name')].map((node) => node.textContent)");
    assert.deepEqual(rows, ['Pintu keluar masuk', 'Teras']);
    const tags = await page.evaluate("[...document.querySelectorAll('#device-admin-list .menu-row .sub')].map((node) => node.textContent)");
    assert.match(tags[0], /pintu/);
    assert.match(tags[0], /administrator/);
    assert.match(tags[0], /1 shared/);
    assert.match(tags[1], /favourite/);

    await page.evaluate("document.getElementById('device-admin-group').value = 'teras'");
    await page.evaluate("[...document.querySelectorAll('#device-admin button')].find((button) => button.textContent.includes('Save group')).click()");
    await page.wait("document.getElementById('device-admin-status').textContent === 'stub saved the group'", 'the save to come back', 10000);
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
    assert.deepEqual(tourCalls, ['/api/device/controls/tour/start?tour=1', '/api/device/controls/tour/stop?tour=1']);
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
  return misses;
})()`;
const EXTRA_BUTTON = `(() => {
  const extra = document.getElementById('record').cloneNode(true);
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
        await page.wait("!document.getElementById('lens-pick').hidden", 'the lens picker of the dual-lens clip');
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
