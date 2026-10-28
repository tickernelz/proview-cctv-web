import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];
const TOKEN = 'probe-desktop-token';

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

async function saveARestartField(page) {
  await page.evaluate("document.getElementById('open-settings').click()");
  await page.wait("document.querySelector('.set-nav a[data-section=\"config\"]') !== null", 'the Configuration link');
  await page.evaluate("document.querySelector('.set-nav a[data-section=\"config\"]').click()");
  await page.wait("document.querySelector('#settings-fields [data-key=\"PROVIEW_LISTEN\"]') !== null", 'the server address field');
  await page.evaluate(`(() => {
    const input = document.querySelector('#settings-fields [data-key="PROVIEW_LISTEN"]');
    input.value = input.value === '127.0.0.1:9797' ? '127.0.0.1:9798' : '127.0.0.1:9797';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await page.wait("!document.getElementById('settings-save').hidden", 'the Save button');
  await page.evaluate("document.getElementById('settings-save').click()");
  await page.wait("document.getElementById('save-state').textContent.includes('Restart')", 'the save bar');
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
  const page = await browser.page();
  try {
    await signIn(page, server.base);
    await saveARestartField(page);
    assert.equal(await page.evaluate("document.getElementById('settings-restart').hidden"), true);
    const state = await page.evaluate("document.getElementById('save-state').textContent");
    assert.match(state, /Restart the server process to apply 1 field/);
  } finally {
    await page.close();
  }
});

test('the desktop page restarts through the local route with its token', async () => {
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
    await page.wait("document.getElementById('settings-restart').textContent === 'Restarting…'", 'the busy label');
    assert.equal(seen.length, 1);
    assert.equal(seen[0].method, 'POST');
    assert.equal(headerValue(seen[0], 'x-proview-desktop-token'), TOKEN);
  } finally {
    await page.close();
  }
});
