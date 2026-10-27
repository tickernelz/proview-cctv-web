import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const CLIPS = { 'gate_ch1_main.h264': DUAL_LENS_CLIP, 'gate_ch1_sub.h264': DUAL_LENS_CLIP };
const PLAYING = "document.getElementById('well').dataset.state === 'playing'";
const VIDEO = "(function(){var v=document.getElementById('video');return {width:v.videoWidth,height:v.videoHeight,time:v.currentTime};})()";

let server;
let browser;
let downloads;

before(async () => {
  downloads = mkdtempSync(join(tmpdir(), 'proview-snapshot-'));
  server = await startServer(DEVICES, { clips: CLIPS });
  browser = await startBrowser();
});

after(async () => {
  try {
    await browser?.close();
  } finally {
    try {
      await server?.stop();
    } finally {
      rmSync(downloads, { recursive: true, force: true, maxRetries: 5 });
    }
  }
});

async function watching() {
  const page = await browser.page({ downloads });
  await signIn(page, server.base);
  await page.wait(PLAYING, 'the camera to play', 20000);
  return page;
}

function png(path) {
  const bytes = readFileSync(path);
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', 'the file starts with the PNG signature');
  assert.equal(bytes.subarray(12, 16).toString('latin1'), 'IHDR');
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), bytes: bytes.length };
}

async function downloaded(act) {
  const seen = new Set(readdirSync(downloads));
  await act();
  const end = Date.now() + 15000;
  while (Date.now() < end) {
    const names = readdirSync(downloads);
    const fresh = names.filter((name) => name.endsWith('.png') && !seen.has(name));
    if (fresh.length && !names.some((name) => name.endsWith('.crdownload'))) {
      const path = join(downloads, fresh[0]);
      const first = statSync(path).size;
      await sleep(150);
      if (first > 0 && statSync(path).size === first) {
        return { name: fresh[0], path };
      }
    }
    await sleep(50);
  }
  throw new Error('the snapshot download never finished: ' + JSON.stringify(readdirSync(downloads)));
}

test('the Snapshot button downloads a PNG of the live frame at its own size', async () => {
  const page = await watching();
  try {
    const video = await page.evaluate(VIDEO);
    assert.deepEqual([video.width, video.height], [640, 720], 'the fixture streams two stacked 640\u00d7360 pictures');
    assert.ok(video.time > 0, 'the live picture is advancing');
    const file = await downloaded(() => page.evaluate("document.getElementById('snapshot').click()"));
    const image = png(file.path);
    assert.deepEqual([image.width, image.height], [video.width, video.height]);
    assert.match(file.name, /^gate_ch1_lens-combined_\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-\d{3}Z\.png$/);
    console.log('# snapshot ' + file.name + ': ' + image.width + '\u00d7' + image.height + ', ' + image.bytes + ' bytes');
  } finally {
    await page.close();
  }
});

test('the Snapshot shortcut keeps the selected lens crop', async () => {
  const page = await watching();
  try {
    const video = await page.evaluate(VIDEO);
    assert.deepEqual([video.width, video.height], [640, 720], 'the fixture streams two stacked 640\u00d7360 pictures');
    await page.key('l');
    await page.wait("document.getElementById('well').dataset.lens !== 'combined'", 'a single-lens view', 10000);
    const file = await downloaded(() => page.key('s'));
    const image = png(file.path);
    assert.deepEqual([image.width, image.height], [640, 360]);
    assert.match(file.name, /_lens-0_/);
    console.log('# lens snapshot ' + file.name + ': ' + image.width + '\u00d7' + image.height + ', ' + image.bytes + ' bytes');
  } finally {
    await page.close();
  }
});
