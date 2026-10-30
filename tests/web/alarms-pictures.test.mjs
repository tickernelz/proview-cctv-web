import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as sleep } from 'node:timers/promises';
import { EVENTS_PANEL_CLOSED, startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [
  { id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 },
  { id: 'yard', name: 'Yard camera', model: 'XM-IPC', online: true, channels: 1 },
];
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

const YARD = {
  ...ALARMS,
  serial: 'yard',
  name: 'Yard camera',
  entries: ALARMS.entries.map((entry, index) => ({
    ...entry,
    id: '2610272150' + (10 + index),
    time: '2026-10-27 21:1' + index + ':10',
    picture: { ...entry.picture, object: '003_yard_2610272150' + (10 + index) + '.jpeg' },
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

const PATTERNS = [{ urlPattern: '*/api/cloud/alarms?*' }, { urlPattern: '*/api/cloud/alarms/picture*' }];

const refusedReply = () => ({
  raw: { status: 403, type: 'text/plain', body: Buffer.from('gone').toString('base64'), cache: 'no-store' },
});

const objectOf = (url) => new URL(url, 'http://panel.invalid').searchParams.get('object');

const picturesIn = (page) => page.evaluate("document.querySelectorAll('.alarm-row img').length");

const rowOf = (page, id) =>
  page.evaluate(`(() => {
    const row = document.querySelector('.alarm-row[data-id=' + JSON.stringify(${JSON.stringify(id)}) + ']');
    return row ? { found: true, images: row.querySelectorAll('img').length } : { found: false, images: -1 };
  })()`);

async function openPanel(respond, ready = "document.querySelectorAll('.alarm-row').length === 3") {
  const page = await browser.page({ init: [EVENTS_PANEL_CLOSED], intercept: { patterns: PATTERNS, respond } });
  await signIn(page, server.base);
  await page.wait("!document.querySelector('.alarm-bell').hidden", 'the alarm bell');
  await page.evaluate("document.querySelector('.alarm-bell').click()");
  await page.wait(ready, 'the panel to open');
  return page;
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

test('the panel asks object storage once per thumbnail across poll cycles', async () => {
  let pictures = 0;
  let polls = 0;
  const page = await openPanel((url) => {
    if (url.includes('/api/cloud/alarms/picture')) {
      pictures += 1;
      return { raw: { type: 'image/jpeg', body: PIXEL, cache: CACHE } };
    }
    polls += 1;
    return ALARMS;
  });
  try {
    await until(() => polls >= 2, 40000, 'two poll cycles');
    await sleep(600);
    const shown = await page.evaluate("document.querySelectorAll('.alarm-row img').length");
    assert.equal(shown, ALARMS.entries.length, 'the panel drew ' + shown + ' thumbnails');
    assert.ok(
      pictures <= ALARMS.entries.length + 1,
      'object storage was called ' + pictures + ' times over ' + polls + ' polls for ' + ALARMS.entries.length + ' objects',
    );
    console.log('polls=' + polls + ' picture_requests=' + pictures + ' cache=' + JSON.stringify(CACHE));
  } finally {
    await page.close();
  }
});

test('one refused object leaves every other thumbnail on screen', async () => {
  const refused = ALARMS.entries[1];
  let refusedHits = 0;
  let polls = 0;
  const page = await openPanel((url) => {
    if (url.includes('/api/cloud/alarms/picture')) {
      if (objectOf(url) === refused.picture.object) {
        refusedHits += 1;
        return refusedReply();
      }
      return { raw: { type: 'image/jpeg', body: PIXEL, cache: CACHE } };
    }
    polls += 1;
    return ALARMS;
  });
  try {
    await until(async () => (await picturesIn(page)) === 2, 20000, 'the panel to drop the refused row');
    await until(() => polls >= 2, 40000, 'two poll cycles');
    await sleep(600);
    const images = await picturesIn(page);
    assert.equal(images, 2, 'one 403 left ' + images + ' of 3 thumbnails on screen');
    assert.deepEqual(await rowOf(page, refused.id), { found: true, images: 0 }, 'the refused row kept a picture');
    assert.equal(refusedHits, 1, 'the refused object was fetched ' + refusedHits + ' times');
    console.log('refused_row=blank kept_thumbnails=' + images + ' refused_fetches=' + refusedHits + ' polls=' + polls);
  } finally {
    await page.close();
  }
});

test('a refused newest picture does not turn the whole panel off', async () => {
  const refused = ALARMS.entries[2];
  let refusedHits = 0;
  let polls = 0;
  const page = await openPanel((url) => {
    if (url.includes('/api/cloud/alarms/picture')) {
      if (objectOf(url) === refused.picture.object) {
        refusedHits += 1;
        return refusedReply();
      }
      return { raw: { type: 'image/jpeg', body: PIXEL, cache: CACHE } };
    }
    polls += 1;
    return ALARMS;
  });
  try {
    await until(async () => (await picturesIn(page)) === 2, 20000, 'the probe to fall back to the next picture');
    await until(() => polls >= 2, 40000, 'two poll cycles');
    await sleep(600);
    const images = await picturesIn(page);
    assert.equal(images, 2, 'a refused probe left ' + images + ' of 3 thumbnails on screen');
    assert.deepEqual(await rowOf(page, refused.id), { found: true, images: 0 }, 'the refused row kept a picture');
    assert.equal(refusedHits, 1, 'the refused object was fetched ' + refusedHits + ' times');
    console.log('probe_fallback=picture kept_thumbnails=' + images + ' refused_fetches=' + refusedHits + ' polls=' + polls);
  } finally {
    await page.close();
  }
});

test('an empty first window does not switch pictures off for later alarms', async () => {
  let polls = 0;
  const page = await openPanel(
    (url) => {
      if (url.includes('/api/cloud/alarms/picture')) return { raw: { type: 'image/jpeg', body: PIXEL, cache: CACHE } };
      polls += 1;
      return polls === 1 ? { ...ALARMS, entries: [], total: 0 } : ALARMS;
    },
    "document.querySelector('.alarm-list p')",
  );
  try {
    await until(() => polls >= 2, 40000, 'a second poll with alarms in it');
    await until(async () => (await picturesIn(page)) === 3, 20000, 'three thumbnails on the late alarms');
    const images = await picturesIn(page);
    assert.equal(images, 3, 'alarms that arrived after an empty window showed ' + images + ' of 3 thumbnails');
    console.log('empty_first_window=1 late_thumbnails=' + images + ' polls=' + polls);
  } finally {
    await page.close();
  }
});

test('a camera switch does not carry a picture failure from the previous camera', async () => {
  const dead = new Set(ALARMS.entries.map((entry) => entry.picture.object));
  let refusedHits = 0;
  const page = await openPanel((url) => {
    if (url.includes('/api/cloud/alarms/picture')) {
      if (dead.has(objectOf(url))) {
        refusedHits += 1;
        return refusedReply();
      }
      return { raw: { type: 'image/jpeg', body: PIXEL, cache: CACHE } };
    }
    return new URL(url, 'http://panel.invalid').searchParams.get('sn') === 'yard' ? YARD : ALARMS;
  });
  try {
    await until(() => refusedHits === 3, 20000, 'the gate camera to refuse three candidates');
    assert.equal(await picturesIn(page), 0, 'the gate camera still had thumbnails');
    await page.wait("document.querySelectorAll('[data-device] option').length === 2", 'two cameras');
    await page.evaluate(
      "(() => { const pick = document.querySelector('[data-device]'); pick.value = 'yard'; pick.dispatchEvent(new Event('change')); })()",
    );
    await until(async () => (await picturesIn(page)) === 3, 30000, 'the yard camera thumbnails');
    const images = await picturesIn(page);
    assert.equal(images, 3, 'the second camera showed ' + images + ' of 3 thumbnails after the first one failed');
    console.log('gate_refused=' + refusedHits + ' yard_thumbnails=' + images);
  } finally {
    await page.close();
  }
});
