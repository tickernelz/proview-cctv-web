import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [
  { id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 },
  { id: 'porch', name: 'Porch', model: 'XM-IPC', online: true, channels: 1 },
  { id: 'yard', name: 'Yard', model: 'XM-IPC', online: true, channels: 1 },
];
const CLIPS = Object.fromEntries(
  DEVICES.flatMap((device) => [
    [device.id + '_ch1_main.h264', DUAL_LENS_CLIP],
    [device.id + '_ch1_sub.h264', DUAL_LENS_CLIP],
  ]),
);

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

const allPlaying = "document.querySelectorAll('.tile').length === 3 && [...document.querySelectorAll('.tile')].every((tile) => tile.dataset.state === 'playing')";
const columns = "getComputedStyle(document.getElementById('tiles')).gridTemplateColumns.split(' ').filter(Boolean).length";

async function openGrid() {
  const page = await browser.page();
  await signIn(page, server.base);
  await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the first camera to play', 20000);
  await page.key('g');
  await page.wait("!document.getElementById('grid').hidden && document.querySelectorAll('.tile').length === 3", 'the grid to show three tiles');
  await page.wait(allPlaying, 'every tile to play', 20000);
  return page;
}

function savedGrid() {
  const file = JSON.parse(readFileSync(join(server.dataDir, 'settings.json'), 'utf8'));
  return file.grid;
}

test('the grid plays one tile per device and keeps the layout and the tile stream across a reload', async () => {
  const page = await openGrid();
  try {
    const names = await page.evaluate("[...document.querySelectorAll('.tile .tile-meta strong')].map((node) => node.textContent)");
    assert.deepEqual(names, ['Gate camera', 'Porch', 'Yard']);
    assert.equal(await page.evaluate(columns), 2);

    await page.evaluate("document.querySelector('#grid-bar button[data-layout=\"2\"]').click()");
    await page.wait(
      "fetch('/api/grid', { credentials: 'same-origin' }).then((response) => response.json()).then((spec) => spec.layout === '2')",
      'the layout to reach the server',
    );
    assert.equal(await page.evaluate(columns), 2);
    assert.equal(savedGrid().layout, '2');

    await page.evaluate("document.querySelector('.tile .tile-pick').click()");
    await page.wait("document.querySelector('.tile .tile-pick').textContent === 'SD'", 'the first tile to move to the sub stream');
    await page.wait(
      "fetch('/api/grid', { credentials: 'same-origin' }).then((response) => response.json()).then((spec) => spec.tiles[0].quality === 'sub')",
      'the sub stream to reach the server',
    );
    await page.wait(allPlaying, 'every tile to play again', 20000);
    const stored = savedGrid();
    assert.equal(stored.tiles.length, 3);
    assert.equal(stored.tiles[0].device, 'gate');
    assert.equal(stored.tiles[0].quality, 'sub');
    assert.equal(stored.tiles[1].quality, 'sub');
  } finally {
    await page.close();
  }

  const again = await openGrid();
  try {
    assert.equal(await again.evaluate(columns), 2);
    assert.equal(await again.evaluate("document.querySelector('.tile .tile-pick').textContent"), 'SD');
    const streams = await again.evaluate(
      "[...document.querySelectorAll('.tile')].map((tile) => tile.dataset.key + ':' + tile.querySelector('.tile-pick').textContent)",
    );
    assert.deepEqual(streams, ['gate:1:SD', 'porch:1:SD', 'yard:1:SD']);
  } finally {
    await again.close();
  }
});

test('the grid route refuses a layout it does not know and keeps the stored one', async () => {
  const page = await openGrid();
  try {
    const answer = await page.evaluate(
      "fetch('/api/grid', { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ layout: '9', tiles: [] }) }).then(async (response) => ({ status: response.status, body: await response.json() }))",
    );
    assert.equal(answer.status, 400);
    assert.equal(answer.body.code, 'grid_invalid');
    const current = await page.evaluate("fetch('/api/grid', { credentials: 'same-origin' }).then((response) => response.json())");
    assert.equal(current.layout, savedGrid().layout);
    assert.equal(current.tiles.length, 3);
  } finally {
    await page.close();
  }
});
