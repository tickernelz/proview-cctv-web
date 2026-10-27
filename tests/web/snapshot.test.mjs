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
const ADVANCING = "document.getElementById('video').currentTime > 0";
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
  await page.wait(ADVANCING, 'the live clock to advance', 20000);
  return page;
}

function png(path) {
  const bytes = readFileSync(path);
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', 'the file starts with the PNG signature');
  assert.equal(bytes.subarray(12, 16).toString('latin1'), 'IHDR');
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), bytes: bytes.length };
}

function pixels(page, path) {
  const data = readFileSync(path).toString('base64');
  return page.evaluate(
    "(function(){return fetch('data:image/png;base64," + data + "').then(function(r){return r.blob();}).then(function(b){return createImageBitmap(b);}).then(function(img){" +
      'var canvas = document.createElement("canvas");canvas.width = img.width;canvas.height = img.height;' +
      'var context = canvas.getContext("2d");context.drawImage(img, 0, 0);' +
      'var d = context.getImageData(0, 0, canvas.width, canvas.height).data;' +
      'var seen = new Set();var transparent = 0;var sum = [0, 0, 0];' +
      'for (var i = 0; i < d.length; i += 4) {if (d[i + 3] !== 255) transparent++;seen.add(d[i] + "," + d[i + 1] + "," + d[i + 2]);sum[0] += d[i];sum[1] += d[i + 1];sum[2] += d[i + 2];}' +
      'var n = d.length / 4;' +
      'return { colours: seen.size, transparent: transparent, mean: [sum[0] / n, sum[1] / n, sum[2] / n] };' +
    '});})()',
  );
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
    const file = await downloaded(() => page.evaluate("document.getElementById('snapshot').click()"));
    const image = png(file.path);
    assert.deepEqual([image.width, image.height], [video.width, video.height]);
    assert.match(file.name, /^gate_ch1_lens-combined_\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-\d{3}Z\.png$/);
    const drawn = await pixels(page, file.path);
    assert.equal(drawn.transparent, 0, 'every pixel is opaque');
    assert.ok(drawn.colours >= 10, 'the picture carries ' + drawn.colours + ' colours, not a blank canvas');
    console.log('# snapshot ' + file.name + ': ' + image.width + '\u00d7' + image.height + ', ' + image.bytes + ' bytes, ' + drawn.colours + ' colours');
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
    const drawn = await pixels(page, file.path);
    assert.equal(drawn.transparent, 0, 'every pixel is opaque');
    assert.ok(drawn.colours >= 10, 'the crop carries ' + drawn.colours + ' colours, not a blank canvas');
    assert.ok(drawn.mean[2] > drawn.mean[0], 'lens 0 is the blue top half, not the green bottom half');
    console.log('# lens snapshot ' + file.name + ': ' + image.width + '\u00d7' + image.height + ', ' + image.bytes + ' bytes, ' + drawn.colours + ' colours');
  } finally {
    await page.close();
  }
});
