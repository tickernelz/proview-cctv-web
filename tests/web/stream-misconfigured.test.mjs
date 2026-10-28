import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startServer, startBrowser, signIn } from './cdp.mjs';

const DEVICES = [{ id: 'gate', name: 'Gate camera', model: 'XM-IPC', online: true, channels: 1 }];

const NO_SERIAL =
  'no camera serial is configured, so the camera credentials are not sent to any device: set PROVIEW_DEVICE_SERIAL under Settings, Local network';

const NO_PASSWORD =
  'no camera user and password for gate: set PROVIEW_DEVICE_USER and PROVIEW_DEVICE_PASS';

const answer = (status, body) => ({
  raw: {
    status,
    type: 'application/json',
    body: Buffer.from(JSON.stringify(body)).toString('base64'),
    cache: 'no-store',
  },
});

const well = `(() => ({
  state: document.getElementById('well').dataset.state,
  headline: document.getElementById('well-text').textContent,
  code: document.getElementById('well-code').textContent,
  passwordShown: !document.getElementById('check-password').hidden,
}))()`;

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

async function openStreamPage(reply) {
  const seen = [];
  const page = await browser.page({
    intercept: {
      patterns: [{ urlPattern: '*/api/stream?*' }, { urlPattern: '*/api/cloud-path?*' }],
      respond: (url) => {
        seen.push(new URL(url).pathname);
        return url.includes('/api/cloud-path') ? { detail: 'the cloud path must not be asked' } : reply;
      },
    },
  });
  await signIn(page, server.base);
  await page.wait("document.getElementById('well').dataset.state === 'error'", 'the stage to fail closed', 20000);
  await page.evaluate('new Promise((done) => setTimeout(done, 800))');
  return { page, seen };
}

test('a misconfigured answer is the headline and pulls in no cloud path', async () => {
  const { page, seen } = await openStreamPage(answer(501, { ok: false, code: 'misconfigured', message: NO_SERIAL }));
  try {
    const shown = await page.evaluate(well);
    assert.match(shown.headline, /PROVIEW_DEVICE_SERIAL/, 'the headline carries the server reason: ' + JSON.stringify(shown));
    assert.doesNotMatch(shown.headline, /not wired yet/, 'the 501 placeholder is gone for a misconfigured answer');
    assert.doesNotMatch(shown.headline, /Cloud reach/, 'no cloud reach text is appended');
    assert.deepEqual(
      seen.filter((path) => path === '/api/cloud-path'),
      [],
      'the cloud path probe is skipped: ' + JSON.stringify(seen),
    );
    console.log('misconfigured headline=' + JSON.stringify(shown.headline));
  } finally {
    await page.close();
  }
});

test('a missing camera password hides the password button', async () => {
  const { page } = await openStreamPage(answer(501, { ok: false, code: 'misconfigured', message: NO_PASSWORD }));
  try {
    const shown = await page.evaluate(well);
    assert.match(shown.headline, /PROVIEW_DEVICE_USER and PROVIEW_DEVICE_PASS/, 'the headline names both settings: ' + JSON.stringify(shown));
    assert.equal(shown.passwordShown, false, 'the camera password button stays hidden: ' + JSON.stringify(shown));
  } finally {
    await page.close();
  }
});

test('a refused camera password keeps the password button', async () => {
  const { page } = await openStreamPage(
    answer(502, { ok: false, message: 'device login was refused: password registration failed' }),
  );
  try {
    const shown = await page.evaluate(well);
    assert.equal(shown.passwordShown, true, 'a refused camera password still offers the button: ' + JSON.stringify(shown));
    assert.doesNotMatch(shown.headline, /not wired yet/, 'a 502 is not a 501');
  } finally {
    await page.close();
  }
});
