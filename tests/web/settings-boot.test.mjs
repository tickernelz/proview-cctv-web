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

async function withPage(body) {
  const server = await startServer(DEVICES, { clips: CLIPS });
  let page;
  try {
    page = await browser.page({ init: [COUNT_SETTINGS] });
    await signIn(page, server.base);
    await page.wait("document.getElementById('well').dataset.state === 'playing'", 'the camera to play', BUDGET);
    await body(page);
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
    assert.equal(await page.evaluate('location.hash'), '#settings/config');
    assert.equal(await page.evaluate(FIELD + '.value'), TYPED);
    assert.equal(await page.evaluate('document.activeElement.dataset.key'), 'PROVIEW_LISTEN');
    assert.equal(await page.evaluate("document.getElementById('settings-save').hidden"), false);
    assert.equal(await page.evaluate("document.getElementById('settings-discard').hidden"), false);
    assert.equal(await page.evaluate('window.__settingsLoads'), 1);
  });
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
