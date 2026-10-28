import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const TOKEN = 'probe-desktop-token';
const LISTEN = '127.0.0.1:9797';
const BUDGET = 60000;

let browser;

before(async () => {
  browser = await startBrowser();
});

after(async () => {
  await browser?.close();
});

async function openConfiguration(page) {
  await page.evaluate("document.getElementById('open-settings').click()");
  await page.wait(
    "document.querySelector('.set-nav a[data-section=\"config\"]') !== null",
    'the Configuration link',
    BUDGET,
  );
  await page.evaluate("document.querySelector('.set-nav a[data-section=\"config\"]').click()");
  await page.wait(
    "document.getElementById('settings-status').textContent !== 'Not loaded yet.'",
    'the settings view to load',
    BUDGET,
  );
  await page.wait(
    "document.querySelector('#settings-fields [data-key=\"PROVIEW_LISTEN\"]') !== null",
    'the server address field',
    BUDGET,
  );
}

async function saveARestartField(page) {
  await openConfiguration(page);
  await page.evaluate(`(() => {
    const input = document.querySelector('#settings-fields [data-key="PROVIEW_LISTEN"]');
    input.value = ${JSON.stringify(LISTEN)};
    input.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await page.wait("!document.getElementById('settings-save').hidden", 'the Save button', BUDGET);
  await page.evaluate("document.getElementById('settings-save').click()");
  await page.wait("document.getElementById('save-state').textContent.includes('Restart')", 'the save bar', BUDGET);
}

function headerValue(request, name) {
  const headers = request.headers || {};
  if (Array.isArray(headers)) {
    const entry = headers.find((header) => header.name.toLowerCase() === name);
    return entry ? entry.value : null;
  }
  const key = Object.keys(headers).find((header) => header.toLowerCase() === name);
  return key === undefined ? null : headers[key];
}

test('a server build keeps the restart button away and names the process', async () => {
  const server = await startServer(DEVICES);
  const page = await browser.page();
  try {
    await signIn(page, server.base);
    await saveARestartField(page);
    assert.equal(await page.evaluate("document.getElementById('settings-restart').hidden"), true);
    const state = await page.evaluate("document.getElementById('save-state').textContent");
    assert.match(state, /Restart the server process to apply 1 field/);
  } finally {
    await page.close();
    await server.stop();
  }
});

test('the desktop page restarts through the local route with its token', async () => {
  const server = await startServer(DEVICES);
  const seen = [];
  const page = await browser.page({
    init: ['window.__PROVIEW_DESKTOP__ = { token: ' + JSON.stringify(TOKEN) + ' };'],
    intercept: {
      patterns: [{ urlPattern: '*/api/desktop/restart', requestStage: 'Request' }],
      respond: (url, request) => {
        seen.push(request);
        return {
          raw: {
            status: 202,
            type: 'application/json',
            body: Buffer.from('{"ok":true}').toString('base64'),
          },
        };
      },
    },
  });
  try {
    await signIn(page, server.base);
    await saveARestartField(page);
    const state = await page.evaluate("document.getElementById('save-state').textContent");
    assert.match(state, /Restart the app to apply 1 field/);
    assert.equal(await page.evaluate("document.getElementById('settings-restart').hidden"), false);
    await page.evaluate("document.getElementById('settings-restart').click()");
    await page.wait(
      "document.getElementById('settings-restart').textContent === 'Restarting…'",
      'the busy label',
      BUDGET,
    );
    assert.equal(seen.length, 1);
    assert.equal(seen[0].method, 'POST');
    assert.equal(headerValue(seen[0], 'x-proview-desktop-token'), TOKEN);
  } finally {
    await page.close();
    await server.stop();
  }
});
