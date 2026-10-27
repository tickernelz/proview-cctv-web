import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as sleep } from 'node:timers/promises';
import { startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const PIXEL =
  '/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0a' +
  'HBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAA' +
  'AAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q==';
const CACHE = process.env.PROVIEW_ALARM_CACHE ?? 'private, max-age=86400, immutable';

const ALARMS = {
  serial: 'gate',
  name: 'Gate camera',
  total: 3,
  finished: true,
  latencyMs: 4,
  queriedAt: Date.now(),
  pollSeconds: 5,
  entries: [0, 1, 2].map((index) => ({
    id: '2610272146' + (30 + index),
    event: 'appEventHumanDetectAlarm:2',
    eventName: 'human',
    name: 'human detect',
    time: '2026-10-27 21:0' + index + ':10',
    utcTime: 1793112000 + index,
    channel: '0',
    message: '',
    picture: {
      object: '003_gate_2610272146' + (30 + index) + '.jpeg',
      bucket: 'OSS_oss-ap-id-pic-01',
      size: 66685,
    },
  })),
};

let server;
let browser;

async function until(check, timeout, what) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await check()) return;
    await sleep(100);
  }
  throw new Error('timed out waiting for ' + what);
}

before(async () => {
  server = await startServer(DEVICES);
  browser = await startBrowser();
});

after(async () => {
  try {
    await browser?.close();
  } finally {
    await server?.stop();
  }
});

test('the panel asks object storage once per thumbnail and reuses it across polls', async () => {
  let pictures = 0;
  let polls = 0;
  const page = await browser.page({
    intercept: {
      patterns: [{ urlPattern: '*/api/cloud/alarms?*' }, { urlPattern: '*/api/cloud/alarms/picture*' }],
      respond: (url) => {
        if (url.includes('/api/cloud/alarms/picture')) {
          pictures += 1;
          return { raw: { type: 'image/jpeg', body: PIXEL, cache: CACHE } };
        }
        polls += 1;
        return ALARMS;
      },
    },
  });
  try {
    await signIn(page, server.base);
    await page.wait("!document.querySelector('.alarm-bell').hidden", 'the alarm bell');
    await page.evaluate("document.querySelector('.alarm-bell').click()");
    await page.wait("document.querySelectorAll('.alarm-row').length === 3", 'three alarm rows');
    await until(() => polls >= 4, 45000, 'four poll cycles');
    await sleep(1200);
    const shown = await page.evaluate("document.querySelectorAll('.alarm-row img').length");
    assert.equal(shown, ALARMS.entries.length, 'the panel drew ' + shown + ' thumbnails');
    const first = pictures;
    assert.ok(
      first <= ALARMS.entries.length + 1,
      'object storage was called ' + first + ' times over ' + polls + ' polls for ' + ALARMS.entries.length + ' objects',
    );

    await page.goto(server.base + '/?signed-in#live');
    await page.wait("!document.querySelector('.alarm-bell').hidden", 'the alarm bell after reload');
    await page.evaluate("document.querySelector('.alarm-bell').click()");
    await page.wait("document.querySelectorAll('.alarm-row img').length === 3", 'three thumbnails after reload');
    await sleep(800);
    const reload = pictures - first;
    console.log(
      'polls=' + polls + ' first_load_requests=' + first + ' reload_requests=' + reload + ' cache=' + JSON.stringify(CACHE),
    );
    assert.equal(reload, 0, 'a reload asked object storage ' + reload + ' times for immutable objects');
  } finally {
    await page.close();
  }
});
