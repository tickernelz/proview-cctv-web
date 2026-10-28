import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];

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

async function openSettings() {
  const page = await browser.page();
  await signIn(page, server.base);
  await page.goto(server.base + '/?signed-in#settings/config');
  await page.wait("!document.getElementById('settings').hidden", 'the Settings screen', 20000);
  await page.wait("document.getElementById('setting-proview-desktop-alarm-notify') !== null", 'the Desktop settings group', 20000);
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
