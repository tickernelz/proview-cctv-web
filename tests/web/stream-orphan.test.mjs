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

const HOLD_ANY = TRACK + `
  (() => {
    const real = window.fetch.bind(window);
    window.__holding = false;
    window.__held = [];
    window.fetch = (input, init) => {
      const url = typeof input === 'string' ? input : (input && input.url) || '';
      if (window.__holding && url.indexOf('/api/stream') === 0) {
        return new Promise((resolve, reject) => {
          window.__held.push({ url, release: () => real(input, init).then(resolve, reject) });
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

const DROP_FRAMES = `
  window.__liveSockets = [];
  window.__dropFrames = false;
  (() => {
    const Native = window.WebSocket;
    window.WebSocket = function (url, protocols) {
      const socket = protocols === undefined ? new Native(url) : new Native(url, protocols);
      const drop = window.__dropFrames;
      socket.addEventListener('message', (event) => {
        if (drop) {
          event.stopImmediatePropagation();
        }
      });
      window.__liveSockets.push({ url: String(url), socket, drop });
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

const openSettings = async (page) => {
  await page.evaluate("document.getElementById('open-settings').click()");
  await page.wait("!document.getElementById('settings').hidden", 'the Settings screen');
};

const liveSockets = "window.__liveSockets.filter((entry) => entry.url.includes('/ws/live')).map((entry) => entry.socket.readyState)";

const socketsWithOwner = "window.__liveSockets.filter((entry) => entry.url.includes('/ws/live')).map((entry) => ({ q: entry.url, rs: entry.socket.readyState, owned: Boolean(state.players.get('stage')) && state.players.get('stage').socket === entry.socket }))";

const stageSocketOpen = "Boolean(state.players.get('stage') && state.players.get('stage').socket && state.players.get('stage').socket.readyState === 1)";

const ownedOpenSocket = "window.__liveSockets.some((entry) => entry.url.includes('/ws/live') && entry.socket.readyState === 1 && state.players.get('stage') && state.players.get('stage').socket === entry.socket)";

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

test('a player replaced before its first frame stays quiet on the shared video', async () => {
  const page = await browser.page({ init: [DROP_FRAMES] });
  try {
    await signIn(page, server.base);
    await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the boot picture', 20000);
    await page.evaluate("window.__dropFrames = true; setQuality('sub')");
    await page.wait(stageSocketOpen, 'the replaced socket to open without frames', 10000);
    await page.evaluate("window.__replaced = state.players.get('stage')");
    await page.evaluate("window.__dropFrames = false; setQuality('main')");
    await page.wait("document.getElementById('well').dataset.state === 'playing' && state.players.get('stage').announced", 'the next picture', 20000);
    const replaced = await page.evaluate("({ same: state.players.get('stage') === window.__replaced, watchdog: window.__replaced.watchdog !== null })");
    assert.equal(replaced.same, false, 'every request owns its stage player');
    assert.equal(replaced.watchdog, false, 'a replaced player runs no watchdog: ' + JSON.stringify(replaced));
    await page.evaluate("window.__dropFrames = true; setQuality('sub')");
    await page.wait(stageSocketOpen, 'a socket that sends no frame', 10000);
    await page.evaluate('new Promise((done) => setTimeout(done, 4500))');
    const after = await page.evaluate("({ well: document.getElementById('well').dataset.state, watchdog: window.__replaced.watchdog !== null })");
    assert.equal(after.watchdog, false, 'the replaced player stays quiet: ' + JSON.stringify(after));
    assert.equal(after.well, 'loading', 'a load with no frame yet stays loading: ' + JSON.stringify(after));
  } finally {
    await page.close();
  }
});

for (const order of [[0, 1], [1, 0]]) {
  test('two quality switches overlap in flight, replies in order ' + order.join(','), async () => {
    const page = await browser.page({ init: [HOLD_ANY] });
    try {
      await signIn(page, server.base);
      await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the boot picture', 20000);
      assert.equal((await page.evaluate(liveSockets)).length, 1);
      const started = await page.evaluate('state.prefs.quality');
      const other = started === 'main' ? 'sub' : 'main';
      await page.evaluate('window.__holding = true');
      await page.evaluate("setQuality('" + other + "')");
      await page.evaluate("setQuality('" + started + "')");
      await page.wait('window.__held.length === 2', 'both stream requests parked', 10000);
      const held = await page.evaluate('window.__held.map((entry) => entry.url)');
      assert.ok(held[0].includes('quality=' + other) && held[1].includes('quality=' + started), 'two different qualities are in flight: ' + JSON.stringify(held));
      await page.evaluate('window.__held[' + order[0] + '].release()');
      await page.evaluate('new Promise((done) => setTimeout(done, 1000))');
      await page.evaluate('window.__held[' + order[1] + '].release()');
      await page.wait(ownedOpenSocket, 'the socket the stage owns to open', 10000);
      await page.evaluate('new Promise((done) => setTimeout(done, 1500))');
      const sockets = await page.evaluate(socketsWithOwner);
      const info = await page.evaluate("({ prefs: state.prefs.quality, ticket: state.ticket && state.ticket.url, well: document.getElementById('well').dataset.state })");
      assert.deepEqual(
        sockets.filter((entry) => entry.rs === 1 && !entry.owned),
        [],
        'an open /ws/live socket no player owns: ' + JSON.stringify(sockets),
      );
      assert.ok(info.ticket && info.ticket.endsWith('quality=' + info.prefs), 'the stored ticket matches the chosen quality: ' + JSON.stringify(info));
      assert.equal(info.well, 'playing', 'the stage keeps playing: ' + JSON.stringify(info));
    } finally {
      await page.close();
    }
  });
}
