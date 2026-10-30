import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const CLIPS = { 'gate_ch1_main.h264': DUAL_LENS_CLIP, 'gate_ch1_sub.h264': DUAL_LENS_CLIP };

const ENCODE = {
  ok: true,
  backend: 'dvrip',
  encode: {
    main: { resolution: '3072*2048', compression: 'H.264', fps: 12, bitrate: 1536, gop: 36, quality: 3, bitrate_control: 'CBR' },
    extra: { resolution: '640*360', compression: 'H.264', fps: 12, bitrate: 512, gop: 24, quality: 3, bitrate_control: 'CBR' },
    snap: { resolution: '3072*2048', compression: 'H.264', fps: 1, bitrate: 512, gop: 1, quality: 3, bitrate_control: null },
  },
};

const NETWORK = {
  ok: true,
  backend: 'dvrip',
  network: {
    host_ip: '192.168.0.100',
    submask: '255.255.255.0',
    gateway: '192.168.0.1',
    mac: '3C:EF:8C:11:22:33',
    dhcp: false,
    dns: '192.168.0.1',
    spare_dns: '8.8.8.8',
    block: { HostIP: '192.168.0.100', UserName: '<redacted>', Password: '<redacted>' },
  },
};

const STORAGE = {
  ok: true,
  backend: 'dvrip',
  storage: {
    partitions: [{ index: 0, current: true, status: 1, total_space: '30528MB', remain_space: '29303MB', start: '2026-10-20 11:02:07', end: '2026-10-27 13:55:41' }],
    overwrite: 'OverWrite',
    record_mode: 'ConfigRecord',
  },
};

const REPORT = {
  ok: true,
  backend: 'dvrip',
  reverted: false,
  stream: 'main',
  reports: [
    {
      block: 'Simplify.Encode',
      backup: '/tmp/config-backup-simplify-encode.json',
      fields: [{ field: 'MainFormat.Video.FPS', before: 12, requested: 10, after: 10, matches: true }],
      written: {},
      read_back: {},
      reverted: false,
      restored: [],
    },
  ],
  encode: ENCODE.encode,
  storage: null,
};

let server;
let browser;

before(async () => {
  server = await startServer(DEVICES, { clips: CLIPS });
  browser = await startBrowser();
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

const shown = (id) => "(() => { const node = document.getElementById('" + id + "'); return node && node.getBoundingClientRect().height > 0; })()";

test('the Device settings page reads the camera and writes only the fields that changed', async () => {
  const seen = [];
  const page = await open({
    intercept: {
      patterns: [{ urlPattern: '*/api/device/settings/*' }],
      respond: (url) => {
        seen.push(url);
        if (url.includes('/encode?') && url.includes('stream=')) return REPORT;
        if (url.includes('/encode')) return ENCODE;
        if (url.includes('/network')) return NETWORK;
        return STORAGE;
      },
    },
  });
  try {
    await page.evaluate("document.querySelector('.set-nav a[data-section=\"device\"]').click()");
    await page.wait("!!document.querySelector('#device-encode input[type=number]')", 'the encoder fields to render');
    assert.equal(await page.evaluate("document.querySelector('#device-encode input[type=number]').value"), '12');
    const text = await page.evaluate("document.getElementById('set-body').textContent");
    assert.match(text, /3072\*2048/);
    assert.match(text, /192\.168\.0\.100/);
    assert.match(text, /29303MB/);
    assert.doesNotMatch(text, /device-password/);
    await page.evaluate(
      "(() => { const input = document.querySelector('#device-encode input[type=number]'); input.value = '10'; input.dispatchEvent(new Event('input', { bubbles: true })); })()",
    );
    await page.evaluate("document.querySelector('#device-encode button').click()");
    await page.wait("document.getElementById('device-report').textContent.includes('MainFormat.Video.FPS')", 'the write report to render');
    const write = seen.find((url) => url.includes('/encode?') && url.includes('stream='));
    assert.ok(seen.every((url) => url.includes('device=gate')), seen.join(' '));
    assert.match(write, /stream=main/);
    assert.match(write, /fps=10/);
    assert.doesNotMatch(write, /bitrate=/);
    assert.match(await page.evaluate("document.getElementById('device-report').textContent"), /matches/);
  } finally {
    await page.close();
  }
});

test('the Device settings page names the server reason when no camera address is set', async () => {
  const page = await open();
  try {
    await page.evaluate("document.querySelector('.set-nav a[data-section=\"device\"]').click()");
    await page.wait("document.getElementById('device-status').textContent.includes('no camera address')", 'the server reason');
    assert.equal(await page.evaluate(shown('h-device')), true);
  } finally {
    await page.close();
  }
});
