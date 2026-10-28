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
  codeShown: !document.getElementById('well-code').hidden,
  passwordShown: !document.getElementById('check-password').hidden,
}))()`;

const passwordField = '#settings-fields [data-key="PROVIEW_DEVICE_PASS"]';

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
    assert.equal(shown.codeShown, false, 'the reason is not repeated in the code line: ' + JSON.stringify(shown));
    assert.equal(shown.passwordShown, false, 'no camera password to check is offered for a serial gap: ' + JSON.stringify(shown));
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

test('a missing camera password offers the button that reaches the field', async () => {
  const { page } = await openStreamPage(answer(501, { ok: false, code: 'misconfigured', message: NO_PASSWORD }));
  try {
    const shown = await page.evaluate(well);
    assert.match(shown.headline, /PROVIEW_DEVICE_USER and PROVIEW_DEVICE_PASS/, 'the headline names both settings: ' + JSON.stringify(shown));
    assert.equal(shown.passwordShown, true, 'the camera password button stays: ' + JSON.stringify(shown));
    await page.evaluate("document.getElementById('check-password').click()");
    await page.wait(
      'document.activeElement === document.querySelector(' + JSON.stringify(passwordField) + ')',
      'the focus on the camera password field',
      15000,
    );
    const opened = await page.evaluate("({ hash: location.hash, key: document.activeElement.dataset.key })");
    assert.equal(opened.hash, '#settings/config');
    assert.equal(opened.key, 'PROVIEW_DEVICE_PASS');
    console.log('password button -> ' + JSON.stringify(opened));
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

test('a grid tile carries the same misconfigured reason', async () => {
  const { page } = await openStreamPage(answer(501, { ok: false, code: 'misconfigured', message: NO_SERIAL }));
  try {
    await page.key('g');
    await page.wait(
      "!document.getElementById('grid').hidden && document.querySelector('.tile') && document.querySelector('.tile').dataset.state === 'error'",
      'the grid tile to fail closed',
      20000,
    );
    const tile = await page.evaluate(`(() => {
      const fail = document.querySelector('.tile .tile-fail span');
      return { state: document.querySelector('.tile').dataset.state, text: fail.textContent, title: fail.title };
    })()`);
    assert.match(tile.text, /PROVIEW_DEVICE_SERIAL/, 'the tile carries the reason: ' + JSON.stringify(tile));
    assert.doesNotMatch(tile.text, /not wired/, 'the tile drops the 501 placeholder');
    assert.equal(tile.title, NO_SERIAL, 'the tooltip keeps the whole server message');
    console.log('grid tile=' + JSON.stringify(tile));
  } finally {
    await page.close();
  }
});
