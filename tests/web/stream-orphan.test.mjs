import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const CLIPS = { 'gate_ch1_main.h264': DUAL_LENS_CLIP, 'gate_ch1_sub.h264': DUAL_LENS_CLIP };

const TRACK = `
  window.__liveSockets = [];
  (() => {
    const Native = window.WebSocket;
    window.WebSocket = function (url, protocols) {
      const socket = protocols === undefined ? new Native(url) : new Native(url, protocols);
      window.__liveSockets.push({ url: String(url), socket });
      return socket;
    };
    window.WebSocket.prototype = Native.prototype;
  })();
`;

const HOLD_STREAM = TRACK + `
  (() => {
    const real = window.fetch.bind(window);
    window.__streamParked = false;
    window.fetch = (input, init) => {
      const url = typeof input === 'string' ? input : (input && input.url) || '';
      if (!window.__streamParked && url.indexOf('/api/stream') === 0) {
        window.__streamParked = true;
        return new Promise((resolve, reject) => {
          window.__releaseStream = () => real(input, init).then(resolve, reject);
        });
      }
      return real(input, init);
    };
  })();
`;

const MANUAL_MEDIA = TRACK + `
  (() => {
    window.__mediaSources = [];
    class FakeSourceBuffer extends EventTarget {
      constructor() {
        super();
        this.updating = false;
        this.buffered = { length: 0 };
      }
      appendBuffer() {}
      remove() {}
    }
    class FakeMediaSource extends EventTarget {
      constructor() {
        super();
        this.readyState = 'closed';
        window.__mediaSources.push(this);
      }
      addSourceBuffer() {
        this.readyState = 'open';
        return new FakeSourceBuffer();
      }
      removeSourceBuffer() {}
    }
    window.MediaSource = FakeMediaSource;
    URL.createObjectURL = () => 'blob:proview-fake';
    URL.revokeObjectURL = () => {};
  })();
`;

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

const openSettings = async (page) => {
  await page.evaluate("document.getElementById('open-settings').click()");
  await page.wait("!document.getElementById('settings').hidden", 'the Settings screen');
};

const liveSockets = "window.__liveSockets.filter((entry) => entry.url.includes('/ws/live')).map((entry) => entry.socket.readyState)";

test('Settings opened while the boot stream request is in flight leaves no orphan player behind', async () => {
  const page = await browser.page({ init: [HOLD_STREAM] });
  try {
    await signIn(page, server.base);
    await page.wait('window.__streamParked === true', 'the boot stream request to park', 15000);
    assert.equal(await page.evaluate("document.getElementById('well').dataset.state"), 'loading');
    await openSettings(page);
    assert.equal(await page.evaluate('state.players.size'), 0, 'Settings stops every player');
    await page.evaluate('window.__releaseStream()');
    await page.evaluate('new Promise((done) => setTimeout(done, 1500))');
    assert.deepEqual(await page.evaluate(liveSockets), [], 'a player destroyed by stopAll must not open a live socket');
    assert.equal(await page.evaluate('state.ticket'), null, 'the late ticket must not be kept for a screen that shows no camera');
    await page.evaluate("document.getElementById('settings-back').click()");
    await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the live picture after leaving Settings', 20000);
    assert.equal((await page.evaluate(liveSockets)).length, 1);
  } finally {
    await page.close();
  }
});

test('a sourceopen that arrives after the player was destroyed opens no socket', async () => {
  const page = await browser.page({ init: [MANUAL_MEDIA] });
  try {
    await signIn(page, server.base);
    await page.wait('window.__mediaSources.length === 1', 'the player to start a media source', 20000);
    await openSettings(page);
    assert.equal(await page.evaluate('state.players.size'), 0, 'Settings stops every player');
    await page.evaluate("window.__mediaSources[0].dispatchEvent(new Event('sourceopen'))");
    assert.deepEqual(await page.evaluate(liveSockets), [], 'the destroyed player must not open a live socket for a late sourceopen');
  } finally {
    await page.close();
  }
});
