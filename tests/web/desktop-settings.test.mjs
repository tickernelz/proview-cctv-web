import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const SHELL = ['window.__TAURI__ = { core: { invoke: () => Promise.reject(new Error("no ipc in this test")) } };'];

let server;
let browser;

before(async () => {
  server = await startServer(DEVICES);
  browser = await startBrowser();
});

after(async () => {
  try {
    await browser?.close();
  } finally {
    await server?.stop();
  }
});

const idOf = (key) => 'setting-' + key.toLowerCase().replace(/_/g, '-');

async function openSettings(init = SHELL) {
  const page = await browser.page({ init });
  await signIn(page, server.base);
  await page.goto(server.base + '/?signed-in#settings/config');
  await page.wait(init.length ? "document.getElementById('setting-proview-desktop-alarm-notify') !== null" : "document.getElementById('setting-proview-device-host') !== null", 'the settings fields', 20000);
  return page;
}

const checked = (key) => "document.getElementById('" + idOf(key) + "').checked";
const saved = "document.getElementById('settings-status').textContent";

test('the desktop switches sit in Configuration with their documented defaults', async () => {
  const page = await openSettings();
  try {
    const pause = await page.evaluate(
      "(() => { const row = document.getElementById('setting-proview-desktop-alarm-interval'); return { min: row.min, max: row.max, placeholder: row.placeholder, type: row.type }; })()",
    );
    assert.equal(pause.type, 'number');
    assert.equal(pause.min, '15');
    assert.equal(pause.max, '3600');
    assert.equal(pause.placeholder, '60');
    assert.equal(await page.evaluate(checked('proview_desktop_alarm_notify')), false, 'notifications start off');
    assert.equal(await page.evaluate(checked('proview_desktop_close_to_tray')), true, 'the window keeps the app alive');
    assert.equal(await page.evaluate(checked('proview_desktop_minimize_to_tray')), false, 'minimizing does not hide by default');
    const labelled = await page.evaluate(
      "(() => { const box = document.getElementById('setting-proview-desktop-alarm-notify'); return box.closest('.row').textContent; })()",
    );
    assert.match(labelled, /Alarm notifications/);
  } finally {
    await page.close();
  }
});

test('a switched toggle is saved as on or off through the same settings call', async () => {
  const page = await openSettings();
  try {
    await page.evaluate("document.getElementById('setting-proview-desktop-alarm-notify').click()");
    await page.evaluate("document.getElementById('setting-proview-desktop-close-to-tray').click()");
    await page.wait("!document.getElementById('settings-save').hidden", 'the save button');
    await page.evaluate("document.getElementById('settings-save').click()");
    await page.wait("document.getElementById('settings-status').textContent.startsWith('Saved')", 'the save to land', 15000);
    assert.match(await page.evaluate(saved), /Saved 2 fields/);
    const stored = await page.evaluate("fetch('/api/settings').then((r) => r.json())");
    const value = (key) => stored.fields.find((field) => field.key === key)?.value;
    assert.equal(value('PROVIEW_DESKTOP_ALARM_NOTIFY'), 'on');
    assert.equal(value('PROVIEW_DESKTOP_CLOSE_TO_TRAY'), 'off');
    assert.equal(await page.evaluate(checked('proview_desktop_alarm_notify')), true, 'the switch keeps its state after the save');
  } finally {
    await page.close();
  }
});

test('the desktop group stays out of the way on a self-hosted page', async () => {
  const page = await openSettings([]);
  try {
    const hasDesktop = await page.evaluate("document.getElementById('setting-proview-desktop-alarm-notify') !== null");
    assert.equal(hasDesktop, false, 'nothing reads these keys outside the desktop shell');
  } finally {
    await page.close();
  }
});

test('the desktop page hands its run token to the settings calls', async () => {
  const page = await browser.page({
    init: [
      "window.__PROVIEW_DESKTOP__ = { token: 'web-test-token' };",
      "window.__seenSettings = []; window.fetch = ((inner) => (url, options) => { if (String(url).indexOf('/api/settings') === 0) { const headers = (options && options.headers) || {}; window.__seenSettings.push(headers['x-proview-desktop-token'] || ''); } return inner(url, options); })(window.fetch);",
    ],
  });
  try {
    await page.call('Network.clearBrowserCookies');
    await page.goto(server.base + '/');
    await page.wait("!document.getElementById('signin').hidden", 'the sign-in screen', 20000);
    await page.evaluate("document.getElementById('login-settings').click()");
    await page.wait("document.getElementById('setting-proview-device-host') !== null", 'the settings fields', 20000);
    assert.deepEqual(
      await page.evaluate('window.__seenSettings'),
      ['web-test-token'],
      'the page token rides every settings call, which is what the desktop build asks for',
    );
  } finally {
    await page.close();
  }
});

test('a desktop alarm event opens the sheet on that camera', async () => {
  const page = await browser.page({ init: SHELL });
  try {
    await signIn(page, server.base);
    await page.wait("document.getElementById('alarm-bell') !== null || document.querySelector('.alarm-bell') !== null", 'the alarm bell', 20000);
    await page.evaluate(
      "window.dispatchEvent(new CustomEvent('desktop-alarm', { detail: { id: '261027171005', serial: 'gate', channel: '0', device: 'Gate camera', event: 'Motion', time: '2026-10-27 17:10:05', utcTime: 1793095807 } }))",
    );
    await page.wait("document.querySelector('.alarm-sheet') && !document.querySelector('.alarm-sheet').hidden", 'the alarm sheet', 15000);
    const chosen = await page.evaluate("document.querySelector('[data-device]').value");
    assert.equal(chosen, 'gate', 'the sheet follows the camera the alarm came from');
  } finally {
    await page.close();
  }
});

test('a desktop live event brings the live view back', async () => {
  const page = await browser.page({ init: SHELL });
  try {
    await signIn(page, server.base);
    await page.evaluate("location.hash = '#settings/config'; document.getElementById('open-settings').click()");
    await page.wait("!document.getElementById('settings').hidden", 'the settings screen', 15000);
    await page.evaluate("window.dispatchEvent(new CustomEvent('desktop-live'))");
    await page.wait("!document.getElementById('watch').hidden", 'the live screen', 15000);
  } finally {
    await page.close();
  }
});
