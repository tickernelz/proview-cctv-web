import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn, DUAL_LENS_CLIP } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const CLIPS = { 'gate_ch1_main.h264': DUAL_LENS_CLIP, 'gate_ch1_sub.h264': DUAL_LENS_CLIP };
const FIELD = "document.querySelector('#settings-fields [data-key=\"PROVIEW_LISTEN\"]')";
const TYPED = '1270019711';
const BUDGET = 20000;

const COUNT_SETTINGS = `(() => {
  const real = window.fetch.bind(window);
  window.__settingsLoads = 0;
  window.fetch = (target, options) => {
    const url = typeof target === 'string' ? target : (target && target.url) || '';
    if (url.indexOf('/api/settings') !== -1) {
      window.__settingsLoads += 1;
    }
    return real(target, options);
  };
})();`;

const STREAM_COUNT = `(() => {
  const real = window.fetch.bind(window);
  window.__streams = 0;
  window.fetch = (target, options) => {
    const url = typeof target === 'string' ? target : (target && target.url) || '';
    if (url.indexOf('/api/stream') !== -1) {
      window.__streams += 1;
    }
    return real(target, options);
  };
})();`;

const HOLD_SIGNIN = `(() => {
  const real = window.fetch.bind(window);
  const Native = window.WebSocket;
  window.__sockets = [];
  window.WebSocket = class Tracked extends Native {
    constructor(...args) {
      super(...args);
      window.__sockets.push(this);
    }
  };
  const held = [];
  window.__release = () => {
    window.fetch = real;
    for (const run of held.splice(0)) run();
  };
  window.fetch = (target, options) => {
    const url = typeof target === 'string' ? target : (target && target.url) || '';
    if (url.indexOf('/api/login') === -1 && url.indexOf('/api/devices') === -1) {
      return real(target, options);
    }
    return new Promise((done, fail) => {
      held.push(() => real(target, options).then(done, fail));
    });
  };
})();`;

const OPEN_SOCKETS = 'window.__sockets.filter((socket) => socket.readyState === 1).length';

let browser;

before(async () => {
  browser = await startBrowser();
});

after(async () => {
  await browser?.close();
});

async function openConfiguration(page) {
  await page.evaluate("document.getElementById('open-settings').click()");
  await page.evaluate("document.querySelector('.set-nav a[data-section=\"config\"]').click()");
  await page.wait(FIELD + ' !== null', 'the server address field', BUDGET);
  await page.wait(
    "document.getElementById('settings-status').textContent.indexOf('Loading') === -1",
    'the settings read',
    BUDGET,
  );
}

async function typeAddress(page, text) {
  await page.evaluate(FIELD + '.focus()');
  await page.evaluate(FIELD + '.select()');
  for (const character of text) {
    await page.key(character);
  }
}

async function withPage(body, options = {}) {
  const server = await startServer(DEVICES, { clips: CLIPS });
  let page;
  try {
    page = await browser.page({ init: options.init || [COUNT_SETTINGS] });
    if (options.ready !== false) {
      await signIn(page, server.base);
      await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the camera to play', BUDGET);
    }
    await body(page, server.base);
  } finally {
    await page?.close();
    await server.stop();
  }
}

test('a field typed while boot finishes keeps its value, its focus and the Save button', async () => {
  await withPage(async (page) => {
    await openConfiguration(page);
    await typeAddress(page, TYPED);
    assert.equal(await page.evaluate(FIELD + '.value'), TYPED);
    await page.evaluate('signedIn()');
    assert.equal(await page.evaluate('state.signedIn'), true);
    assert.equal(await page.evaluate('document.body.dataset.screen'), 'settings');
    assert.equal(await page.evaluate('location.hash'), '#/settings/config');
    assert.equal(await page.evaluate(FIELD + '.value'), TYPED);
    assert.equal(await page.evaluate('document.activeElement.dataset.key'), 'PROVIEW_LISTEN');
    assert.equal(await page.evaluate("document.getElementById('settings-save').hidden"), false);
    assert.equal(await page.evaluate("document.getElementById('settings-discard').hidden"), false);
    assert.equal(await page.evaluate('window.__settingsLoads'), 1);
  });
});

test('Settings opened during a held sign-in keeps the edits and starts no stream behind it', async () => {
  await withPage(
    async (page, base) => {
      await page.goto(base + '/');
      await page.wait("!document.getElementById('signin').hidden", 'the sign-in screen', BUDGET);
      await page.evaluate(
        "for (const [id, value] of [['email', 'fixture@example.invalid'], ['password', 'fixture-password']]) { const input = document.getElementById(id); input.value = value; input.dispatchEvent(new Event('input')); }",
      );
      await page.evaluate("document.getElementById('login-submit').click()");
      await page.evaluate("document.getElementById('login-settings').click()");
      await page.wait(FIELD + ' !== null', 'the server address field', BUDGET);
      await typeAddress(page, TYPED);
      assert.equal(await page.evaluate(FIELD + '.value'), TYPED);
      assert.equal(await page.evaluate('state.screen'), 'settings');
      await page.evaluate('window.__release()');
      await page.wait(
        "state.signedIn && document.getElementById('settings-back-label').textContent === 'Live'",
        'the boot to finish',
        BUDGET,
      );
      assert.equal(await page.evaluate('state.signedIn'), true);
      assert.equal(await page.evaluate('state.screen'), 'settings');
      assert.equal(await page.evaluate(FIELD + '.value'), TYPED);
      assert.equal(await page.evaluate("document.getElementById('settings-save').hidden"), false);
      assert.equal(await page.evaluate('window.__streams'), 0);
      assert.equal(await page.evaluate('window.__sockets.length'), 0);
      await page.evaluate("document.getElementById('settings-discard').click()");
      await page.evaluate("document.getElementById('settings-back').click()");
      await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the stream after leaving Settings', BUDGET);
      assert.equal(await page.evaluate(OPEN_SOCKETS), 1);
    },
    { init: [STREAM_COUNT, HOLD_SIGNIN], ready: false },
  );
});

test('a re-render keeps unsaved edits and Discard puts the saved values back', async () => {
  await withPage(async (page) => {
    await openConfiguration(page);
    const saved = await page.evaluate(FIELD + '.value');
    assert.notEqual(saved, TYPED);
    await typeAddress(page, TYPED);
    await page.evaluate('loadSettings()');
    assert.equal(await page.evaluate(FIELD + '.value'), TYPED);
    assert.equal(await page.evaluate('document.activeElement.dataset.key'), 'PROVIEW_LISTEN');
    assert.equal(await page.evaluate("document.getElementById('settings-save').hidden"), false);
    assert.match(await page.evaluate("document.getElementById('save-state').textContent"), /Unsaved changes/);
    await page.evaluate("document.getElementById('settings-discard').click()");
    assert.equal(await page.evaluate(FIELD + '.value'), saved);
    assert.equal(await page.evaluate("document.getElementById('settings-save').hidden"), true);
    assert.equal(await page.evaluate("document.getElementById('settings-discard').hidden"), true);
  });
});
