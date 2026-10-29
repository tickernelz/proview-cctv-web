import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const CLIPS = { 'gate_ch1_main.h264': DUAL_LENS_CLIP, 'gate_ch1_sub.h264': DUAL_LENS_CLIP };

const COUNT_FRAMES = `
  window.__frames = 0;
  window.__bytes = 0;
  (() => {
    const Native = window.WebSocket;
    window.WebSocket = function (url, protocols) {
      const socket = protocols === undefined ? new Native(url) : new Native(url, protocols);
      if (String(url).indexOf('/ws/live') !== -1) {
        socket.addEventListener('message', (event) => {
          if (typeof event.data === 'string') return;
          window.__frames += 1;
          window.__bytes += event.data.size || event.data.byteLength || 0;
        });
      }
      return socket;
    };
    window.WebSocket.prototype = Native.prototype;
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

const playing = "document.getElementById('well').dataset.state === 'playing'";

test('a viewer that joins a running stream keeps receiving fragments', async () => {
  const first = await browser.page({ init: [COUNT_FRAMES] });
  try {
    await signIn(first, server.base);
    await first.wait(playing, 'the first picture', 20000);
    const second = await browser.page({ init: [COUNT_FRAMES] });
    try {
      await signIn(second, server.base);
      await second.wait(playing, 'the second picture', 20000);
      await second.wait('window.__frames >= 2', 'the replay for the late viewer', 20000);
      const lateBefore = await second.evaluate('window.__frames');
      const earlyBefore = await first.evaluate('window.__frames');
      await second.evaluate('new Promise((done) => setTimeout(done, 4000))');
      const lateAfter = await second.evaluate('window.__frames');
      const earlyAfter = await first.evaluate('window.__frames');
      assert.ok(
        lateAfter > lateBefore,
        `the late viewer keeps receiving fragments: ${lateBefore} -> ${lateAfter}`,
      );
      assert.ok(
        earlyAfter > earlyBefore,
        `the first viewer keeps receiving fragments: ${earlyBefore} -> ${earlyAfter}`,
      );
    } finally {
      await second.close();
    }
  } finally {
    await first.close();
  }
});
