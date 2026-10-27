import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, copyFileSync, symlinkSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { randomBytes } from 'node:crypto';

const ROOT = resolve(import.meta.dirname, '..', '..');

const KEYS = {
  ArrowDown: 40, ArrowUp: 38, ArrowLeft: 37, ArrowRight: 39, Escape: 27, Tab: 9, Enter: 13, ' ': 32, Home: 36, End: 35,
};

function freePort() {
  return new Promise((done, fail) => {
    const probe = createServer();
    probe.once('error', fail);
    probe.listen(0, '127.0.0.1', () => {
      const { port } = probe.address();
      probe.close(() => done(port));
    });
  });
}

async function waitFor(check, timeout = 15000, what = 'condition') {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await check();
      if (last) return last;
    } catch (error) {
      last = error;
    }
    await sleep(50);
  }
  throw new Error('timed out waiting for ' + what + (last instanceof Error ? ': ' + last.message : ''));
}

function ended(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve();
  }
  return new Promise((done) => {
    const force = setTimeout(() => child.kill('SIGKILL'), 5000);
    child.once('exit', () => {
      clearTimeout(force);
      done();
    });
    child.kill('SIGTERM');
  });
}

export const DUAL_LENS_CLIP = join(ROOT, 'fixtures', 'dual-lens', 'dual-lens-stacked-640x720.h264');

export async function startServer(devices, { clips = {}, env = {} } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'proview-ui-'));
  const fixtures = join(dir, 'fixtures');
  const data = join(dir, 'data');
  mkdirSync(fixtures);
  mkdirSync(data);
  copyFileSync(join(ROOT, 'fixtures', 'account.json'), join(fixtures, 'account.json'));
  writeFileSync(join(fixtures, 'devices.json'), JSON.stringify(devices));
  for (const [name, source] of Object.entries(clips)) {
    symlinkSync(source, join(fixtures, name));
  }
  const port = await freePort();
  const binary = process.env.PROVIEW_SERVER_BIN || join(ROOT, 'target', 'debug', 'proview-server');
  const child = spawn(binary, [], {
    cwd: ROOT,
    env: {
      ...process.env,
      PROVIEW_PROVIDER: 'fixture',
      PROVIEW_FIXTURE_DIR: fixtures,
      PROVIEW_DATA_DIR: data,
      PROVIEW_LISTEN: '127.0.0.1:' + port,
      ROCKET_SECRET_KEY: randomBytes(32).toString('base64'),
      ...env,
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  let log = '';
  child.stderr.on('data', (chunk) => {
    log += chunk;
  });
  const base = 'http://127.0.0.1:' + port;
  await waitFor(async () => (await fetch(base + '/api/health')).ok, 30000, 'the server at ' + base + '\n' + log);
  return {
    base,
    async stop() {
      await ended(child);
      rmSync(dir, { recursive: true, force: true, maxRetries: 5 });
    },
  };
}

export async function startBrowser(extraArgs = []) {
  const profile = mkdtempSync(join(tmpdir(), 'proview-chrome-'));
  const binary = process.env.CHROME_BIN || 'google-chrome';
  const child = spawn(
    binary,
    [
      '--headless=new',
      '--remote-debugging-port=0',
      '--user-data-dir=' + profile,
      '--no-first-run',
      '--no-default-browser-check',
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      '--autoplay-policy=no-user-gesture-required',
      ...extraArgs,
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] },
  );
  let output = '';
  const url = await new Promise((done, fail) => {
    child.stderr.on('data', (chunk) => {
      output += chunk;
      const found = output.match(/DevTools listening on (ws:\/\/\S+)/);
      if (found) done(found[1]);
    });
    child.once('exit', (code) => fail(new Error('chrome exited ' + code + '\n' + output)));
  });
  const socket = new WebSocket(url);
  await new Promise((done, fail) => {
    socket.onopen = done;
    socket.onerror = fail;
  });
  let next = 0;
  const pending = new Map();
  const listeners = new Set();
  socket.onmessage = (message) => {
    const data = JSON.parse(message.data);
    if (data.id && pending.has(data.id)) {
      const { done, fail } = pending.get(data.id);
      pending.delete(data.id);
      if (data.error) fail(new Error(data.error.message));
      else done(data.result);
      return;
    }
    for (const listener of listeners) listener(data);
  };
  const send = (method, params = {}, sessionId) =>
    new Promise((done, fail) => {
      next += 1;
      pending.set(next, { done, fail });
      socket.send(JSON.stringify({ id: next, method, params, sessionId }));
    });
  return {
    async page({ width = 1440, height = 900, init = [], intercept = null } = {}) {
      const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
      const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
      const call = (method, params) => send(method, params, sessionId);
      await call('Page.enable');
      await call('Runtime.enable');
      await call('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
      for (const source of init) {
        await call('Page.addScriptToEvaluateOnNewDocument', { source });
      }
      if (intercept) {
        await call('Fetch.enable', { patterns: intercept.patterns });
        listeners.add(async (data) => {
          if (data.sessionId !== sessionId || data.method !== 'Fetch.requestPaused') return;
          const body = await intercept.respond(data.params.request.url);
          await call('Fetch.fulfillRequest', {
            requestId: data.params.requestId,
            responseCode: 200,
            responseHeaders: [{ name: 'content-type', value: 'application/json' }],
            body: Buffer.from(JSON.stringify(body)).toString('base64'),
          });
        });
      }
      const evaluate = async (expression) => {
        const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
        if (result.exceptionDetails) {
          throw new Error(result.exceptionDetails.exception ? result.exceptionDetails.exception.description : result.exceptionDetails.text);
        }
        return result.result.value;
      };
      const key = async (name, type = 'both') => {
        const text = name === 'Enter' ? '\r' : name.length === 1 ? name : undefined;
        const params = {
          key: name,
          code: name.length === 1 ? (/[a-z]/i.test(name) ? 'Key' + name.toUpperCase() : /\d/.test(name) ? 'Digit' + name : '') : name,
          windowsVirtualKeyCode: KEYS[name] || name.toUpperCase().charCodeAt(0),
          text,
        };
        if (type !== 'up') await call('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', ...params });
        if (type !== 'down') await call('Input.dispatchKeyEvent', { type: 'keyUp', ...params });
      };
      return {
        call,
        evaluate,
        key,
        wait: (expression, what, timeout) => waitFor(() => evaluate(expression), timeout, what || expression),
        async goto(url) {
          await call('Page.navigate', { url });
          await waitFor(() => evaluate('document.readyState === "complete"'), 15000, 'load of ' + url);
        },
        async screenshot(path) {
          const { data } = await call('Page.captureScreenshot', { format: 'png' });
          writeFileSync(path, Buffer.from(data, 'base64'));
        },
        close: () => send('Target.closeTarget', { targetId }),
      };
    },
    async close() {
      socket.close();
      await ended(child);
      rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
    },
  };
}

export async function signIn(page, base) {
  await page.goto(base + '/');
  await page.evaluate(
    "fetch('/api/login', { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email: 'fixture@example.invalid', password: 'fixture-password' }) }).then((r) => r.status)",
  );
  await page.goto(base + '/?signed-in#live');
  await page.wait("!document.getElementById('watch').hidden", 'the Live screen');
}
