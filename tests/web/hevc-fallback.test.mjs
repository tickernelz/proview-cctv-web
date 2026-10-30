import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const CLIPS = { 'gate_ch1_main.h264': DUAL_LENS_CLIP, 'gate_ch1_sub.h264': DUAL_LENS_CLIP };

const TRACK_SOCKETS = `
  window.__liveUrls = [];
  (() => {
    const Native = window.WebSocket;
    const Fake = window.__FakeHevcSocket;
    window.WebSocket = function (url, protocols) {
      if (String(url).indexOf('/ws/live') !== -1) window.__liveUrls.push(String(url));
      if (Fake && String(url).indexOf('codec=hevc') !== -1) return new Fake(String(url));
      return protocols === undefined ? new Native(url) : new Native(url, protocols);
    };
    window.WebSocket.prototype = Native.prototype;
    Object.assign(window.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  })();
`;

const NO_HEVC = `
  (() => {
    const native = MediaSource.isTypeSupported.bind(MediaSource);
    MediaSource.isTypeSupported = (type) => (/hvc1|hev1/.test(type) ? false : native(type));
  })();
`;

const CLAIMS_HEVC = `
  (() => {
    const native = MediaSource.isTypeSupported.bind(MediaSource);
    MediaSource.isTypeSupported = (type) => (/hvc1|hev1/.test(type) ? true : native(type));
    window.VideoDecoder = window.VideoDecoder || function () {};
    window.VideoDecoder.isConfigSupported = async (config) => ({ supported: true, config });
    window.__FakeHevcSocket = class extends EventTarget {
      constructor(url) {
        super();
        this.url = url;
        this.readyState = 0;
        this.binaryType = 'blob';
        this.onopen = null;
        this.onmessage = null;
        this.onerror = null;
        this.onclose = null;
        setTimeout(() => {
          if (this.readyState === 3) return;
          this.readyState = 1;
          this.deliver(JSON.stringify({ type: 'hello', mode: 'passthrough', codec: 'hvc1.1.6.L63', width: 2304, height: 2592 }));
          this.deliver(new Uint8Array([0, 0, 0, 16, 109, 111, 111, 102, 1, 2, 3, 4, 5, 6, 7, 8]).buffer);
        }, 50);
      }
      deliver(data) {
        if (this.onmessage) this.onmessage(new MessageEvent('message', { data }));
      }
      send() {}
      close() {
        this.readyState = 3;
      }
    };
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
const transport = "document.querySelector('#well video') ? document.querySelector('#well video').dataset.transport || '' : ''";

test('a browser without H.265 asks for the transcode and still plays', async () => {
  const page = await browser.page({ init: [NO_HEVC, TRACK_SOCKETS] });
  try {
    await signIn(page, server.base);
    await page.wait(playing, 'the transcoded picture', 20000);
    const urls = await page.evaluate('window.__liveUrls');
    assert.ok(urls.length >= 1, 'a live socket opened');
    assert.ok(urls.every((url) => url.indexOf('codec=hevc') === -1), 'no socket asked for H.265: ' + JSON.stringify(urls));
    assert.equal(await page.evaluate(transport), 'avc');
  } finally {
    await page.close();
  }
});

test('an H.265 stream the browser cannot decode falls back to the transcode and is remembered', async () => {
  const page = await browser.page({ init: [CLAIMS_HEVC, TRACK_SOCKETS] });
  try {
    await page.call('Network.clearBrowserCookies');
    await signIn(page, server.base);
    await page.wait(playing, 'the picture after the fallback', 30000);
    const urls = await page.evaluate('window.__liveUrls');
    assert.ok(urls[0].indexOf('codec=hevc') !== -1, 'the first socket asked for H.265: ' + JSON.stringify(urls));
    assert.ok(urls[urls.length - 1].indexOf('codec=hevc') === -1, 'the fallback socket asked for the transcode: ' + JSON.stringify(urls));
    assert.equal(await page.evaluate("sessionStorage.getItem('proview.hevc')"), 'no');
    assert.equal(await page.evaluate(transport), 'avc');
  } finally {
    await page.close();
  }
});
