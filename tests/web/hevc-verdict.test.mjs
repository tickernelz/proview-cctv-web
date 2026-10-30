import { test, mock, beforeEach } from 'node:test';
import assert from 'node:assert/strict';

const store = new Map();
globalThis.window = {
  sessionStorage: {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
  },
  setTimeout: (callback, ms) => setTimeout(callback, ms),
  clearTimeout: (handle) => clearTimeout(handle),
};
const page = new EventTarget();
page.hidden = false;
globalThis.document = page;

mock.timers.enable({ apis: ['setTimeout'] });
const { HEVC_FIRST_FRAME_MS, hevcVerdict, rememberHevc, watchFirstFrame } = await import('../../apps/web/src/lib/hevc.ts');

function fakeVideo() {
  const video = new EventTarget();
  video.frames = 0;
  video.currentTime = 0;
  video.getVideoPlaybackQuality = () => ({ totalVideoFrames: video.frames });
  return video;
}

function watch(video) {
  const settled = [];
  const stop = watchFirstFrame(video, () => true, (ok, reason) => settled.push({ ok, reason }));
  return { settled, stop };
}

beforeEach(() => {
  store.clear();
  page.hidden = false;
});

test('a later yes from another player never overwrites a no', () => {
  rememberHevc('no');
  rememberHevc('yes', { codec: 'hvc1.1.6.L63', width: 640, height: 720 });
  assert.equal(hevcVerdict(), 'no');
  store.clear();
  rememberHevc('yes');
  rememberHevc('no');
  assert.equal(hevcVerdict(), 'no');
});

test('a visible page without a decoded frame falls back after the deadline', () => {
  const { settled } = watch(fakeVideo());
  mock.timers.tick(HEVC_FIRST_FRAME_MS - 1);
  assert.deepEqual(settled, []);
  mock.timers.tick(1);
  assert.equal(settled.length, 1);
  assert.equal(settled[0].ok, false);
});

test('frames the decoder counted confirm the stream even without a frame callback', () => {
  const video = fakeVideo();
  const { settled } = watch(video);
  video.frames = 3;
  mock.timers.tick(HEVC_FIRST_FRAME_MS);
  assert.deepEqual(settled, [{ ok: true, reason: '' }]);
});

test('a seek alone does not count as a decoded frame', () => {
  const video = fakeVideo();
  const { settled } = watch(video);
  video.currentTime = 12;
  mock.timers.tick(HEVC_FIRST_FRAME_MS);
  assert.equal(settled[0].ok, false);
});

test('a hidden page decides nothing until it is shown again', () => {
  page.hidden = true;
  const video = fakeVideo();
  const { settled } = watch(video);
  mock.timers.tick(HEVC_FIRST_FRAME_MS * 10);
  assert.deepEqual(settled, [], 'a background tab or a window hidden to the tray must not record no');
  page.hidden = false;
  page.dispatchEvent(new Event('visibilitychange'));
  mock.timers.tick(HEVC_FIRST_FRAME_MS - 1);
  assert.deepEqual(settled, []);
  mock.timers.tick(1);
  assert.equal(settled.length, 1);
  assert.equal(settled[0].ok, false);
});

test('a hidden page that decodes once shown is confirmed', () => {
  page.hidden = true;
  const video = fakeVideo();
  const { settled } = watch(video);
  mock.timers.tick(HEVC_FIRST_FRAME_MS);
  page.hidden = false;
  page.dispatchEvent(new Event('visibilitychange'));
  video.dispatchEvent(new Event('loadeddata'));
  assert.deepEqual(settled, [{ ok: true, reason: '' }]);
  mock.timers.tick(HEVC_FIRST_FRAME_MS * 2);
  assert.equal(settled.length, 1);
});

test('a decode error settles at once and a stopped watch stays quiet', () => {
  const video = fakeVideo();
  const first = watch(video);
  video.dispatchEvent(new Event('error'));
  assert.equal(first.settled[0].ok, false);
  const second = watch(fakeVideo());
  second.stop();
  mock.timers.tick(HEVC_FIRST_FRAME_MS * 2);
  assert.deepEqual(second.settled, []);
});
