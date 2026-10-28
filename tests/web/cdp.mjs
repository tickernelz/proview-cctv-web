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

function signalGroup(child, signal) {
  try {
    process.kill(-child.pid, signal);
  } catch (error) {
    if (error.code !== 'ESRCH') throw error;
  }
}

async function ended(child) {
  if (child.exitCode === null && child.signalCode === null) {
    await new Promise((done) => {
      const force = setTimeout(() => signalGroup(child, 'SIGKILL'), 5000);
      child.once('exit', () => {
        clearTimeout(force);
        done();
      });
      signalGroup(child, 'SIGTERM');
    });
  }
  signalGroup(child, 'SIGKILL');
  await waitFor(() => {
    try {
      process.kill(-child.pid, 0);
      return false;
    } catch (error) {
      return error.code === 'ESRCH';
    }
  }, 10000, 'every process of ' + child.spawnfile + ' to exit');
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
    detached: true,
  });
  let log = '';
  child.stderr.on('data', (chunk) => {
    log += chunk;
  });
  const base = 'http://127.0.0.1:' + port;
  await waitFor(async () => (await fetch(base + '/api/health')).ok, 30000, 'the server at ' + base + '\n' + log);
  return {
    base,
    dataDir: data,
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
    { stdio: ['ignore', 'ignore', 'pipe'], detached: true },
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
    async page({ width = 1440, height = 900, init = [], intercept = null, downloads = null } = {}) {
      const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
      const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
      const call = (method, params) => send(method, params, sessionId);
      await call('Page.enable');
      await call('Runtime.enable');
      await call('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
      if (downloads) {
        await send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, eventsEnabled: true });
      }
      for (const source of init) {
        await call('Page.addScriptToEvaluateOnNewDocument', { source });
      }
      if (intercept) {
        await call('Fetch.enable', { patterns: intercept.patterns });
        listeners.add(async (data) => {
          if (data.sessionId !== sessionId || data.method !== 'Fetch.requestPaused') return;
          const answer = await intercept.respond(data.params.request.url, data.params.request);
          const raw = answer && answer.raw;
          const headers = raw
            ? [
                { name: 'content-type', value: raw.type || 'application/octet-stream' },
                ...(raw.cache ? [{ name: 'cache-control', value: raw.cache }] : []),
              ]
            : [{ name: 'content-type', value: 'application/json' }];
          await call('Fetch.fulfillRequest', {
            requestId: data.params.requestId,
            responseCode: raw && raw.status ? raw.status : 200,
            responseHeaders: headers,
            body: raw
              ? Buffer.from(raw.body, 'base64').toString('base64')
              : Buffer.from(JSON.stringify(answer)).toString('base64'),
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
  await page.goto(base + '/api/health');
  await page.evaluate(
    "fetch('/api/login', { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email: 'fixture@example.invalid', password: 'fixture-password' }) }).then((r) => r.status)",
  );
  await page.goto(base + '/?signed-in#live');
  await page.wait("!document.getElementById('watch').hidden", 'the Live screen');
}

export async function fakeTalkDevice() {
  const frames = [];
  const server = createServer((socket) => {
    let buffer = Buffer.alloc(0);
    let reply = 0;
    const answer = (id, body) => {
      const payload = Buffer.concat([Buffer.from(JSON.stringify(body), 'utf8'), Buffer.from([0])]);
      const header = Buffer.alloc(20);
      header[0] = 0xff;
      header.writeUInt32LE(0x141, 4);
      header.writeUInt32LE(++reply, 8);
      header.writeUInt16LE(id, 14);
      header.writeUInt32LE(payload.length, 16);
      socket.write(Buffer.concat([header, payload]));
    };
    socket.on('data', (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      for (;;) {
        if (buffer.length < 20) return;
        const length = buffer.readUInt32LE(16);
        if (buffer.length < 20 + length) return;
        const id = buffer.readUInt16LE(14);
        const body = buffer.subarray(20, 20 + length);
        buffer = buffer.subarray(20 + length);
        if (id === 1432) {
          frames.push({
            id,
            header: body.subarray(0, 8).toString('hex'),
            silent: body.subarray(8).every((byte) => byte === 0xd5),
          });
        }
        if (id === 1000) answer(1000, { Ret: 100, SessionID: '0x00000141' });
        if (id === 1434) answer(1434, { Ret: 100, Name: 'OPTalk' });
      }
    });
  });
  await new Promise((done) => server.listen(0, '127.0.0.1', done));
  return {
    port: server.address().port,
    frames: () => frames,
    close: () => server.close(),
  };
}

export function toneWave(path, { seconds = 3, hz = 1000, rate = 48000 } = {}) {
  const samples = rate * seconds;
  const data = Buffer.alloc(samples * 2);
  for (let index = 0; index < samples; index += 1) {
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * hz * index) / rate) * 16384), index * 2);
  }
  const header = Buffer.alloc(44);
  header.write('RIFF', 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write('WAVE', 8);
  header.write('fmt ', 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write('data', 36);
  header.writeUInt32LE(data.length, 40);
  writeFileSync(path, Buffer.concat([header, data]));
  return path;
}
