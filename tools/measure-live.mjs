#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';

const base = process.env.PROVIEW_BASE || 'http://127.0.0.1:8787/';
const email = process.env.PROVIEW_MEASURE_EMAIL || 'fixture@example.invalid';
const password = process.env.PROVIEW_MEASURE_PASSWORD || 'fixture-password';
const port = Number(process.env.PROVIEW_CDP_PORT || 9333);
const profile = process.env.PROVIEW_CDP_PROFILE || '/tmp/proview-measure-profile';
const mode = process.env.PROVIEW_TRANSPORT === 'hevc' ? 'hevc' : 'avc';
const viewers = Math.max(1, Number(process.env.PROVIEW_VIEWERS || 1));
const sampleMs = Math.max(1000, Number(process.env.PROVIEW_SAMPLE_MS || 8000));
const sampleSeconds = Number(process.env.PROVIEW_SAMPLE_SECONDS || 8);
const out = process.env.PROVIEW_OUT || '';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const chrome = spawn('google-chrome', [
  '--headless=new', '--no-sandbox', '--disable-gpu',
  '--autoplay-policy=no-user-gesture-required',
  '--remote-debugging-port=' + port, '--window-size=1440,900',
  '--user-data-dir=' + profile, 'about:blank',
], { stdio: 'ignore' });

const call = async (path, init) => (await fetch('http://127.0.0.1:' + port + path, init)).json();

let up = false;
for (let i = 0; i < 80 && !up; i++) {
  try { await call('/json/version'); up = true; } catch { await sleep(250); }
}
if (!up) {
  console.error('chrome did not expose the debugging port');
  process.exit(1);
}

class Tab {
  constructor(socket, targetId) {
    this.targetId = targetId;
    this.socket = socket;
    this.id = 0;
    this.pending = new Map();
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.id && this.pending.has(message.id)) {
        this.pending.get(message.id)(message);
        this.pending.delete(message.id);
      }
    };
  }
  static async open(url) {
    const target = await call('/json/new?' + encodeURIComponent(url), { method: 'PUT' });
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve) => { socket.onopen = resolve; });
    const tab = new Tab(socket, target.id);
    await tab.send('Page.enable');
    await tab.send('Runtime.enable');
    return tab;
  }
  send(method, params) {
    return new Promise((resolve) => {
      const messageId = ++this.id;
      this.pending.set(messageId, resolve);
      this.socket.send(JSON.stringify({ id: messageId, method, params: params || {} }));
    });
  }
  async evaluate(expression) {
    const reply = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    const result = reply.result || {};
    if (result.exceptionDetails) return null;
    return result.result ? result.result.value : null;
  }
  async waitFor(expression, tries, gap) {
    for (let i = 0; i < tries; i++) {
      if (await this.evaluate(expression)) return true;
      await sleep(gap);
    }
    return false;
  }
  async activate() {
    if (this.targetId) await fetch('http://127.0.0.1:' + port + '/json/activate/' + this.targetId);
  }
  close() {
    this.socket.close();
  }
}

const login = (tab) => tab.evaluate(
  "(function(){var e=document.querySelector('#login-form input[type=email]');var p=document.querySelector('#login-form input[type=password]');" +
  "if(!e)return 0;e.value=" + JSON.stringify(email) + ";p.value=" + JSON.stringify(password) +
  ";document.querySelector('#login-submit').click();return 1;})()",
);

const devicesReady = "document.querySelectorAll('#devices li button').length > 0";
const clickFirstDevice = "(function(){var b=document.querySelector('#devices li button');if(!b)return 0;b.click();return 1;})()";
const playing = "(function(){var v=document.querySelector('video');return !!(v && v.readyState>=2 && v.currentTime>0);})()";
const videoState = "(function(){var v=document.querySelector('video');if(!v)return null;return {width:v.videoWidth,height:v.videoHeight,time:v.currentTime,readyState:v.readyState};})()";

function processTree(rootPid) {
  const parents = new Map();
  for (const entry of readdirSync('/proc')) {
    if (!/^\d+$/.test(entry)) continue;
    let stat;
    try { stat = readFileSync('/proc/' + entry + '/stat', 'utf8'); } catch { continue; }
    const tail = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    parents.set(Number(entry), Number(tail[1]));
  }
  const tree = new Set([rootPid]);
  let added = true;
  while (added) {
    added = false;
    for (const [pid, parent] of parents) {
      if (tree.has(parent) && !tree.has(pid)) { tree.add(pid); added = true; }
    }
  }
  return [...tree];
}

function sampleTree(rootPid) {
  let ticks = 0;
  let rss = 0;
  let processes = 0;
  for (const pid of processTree(rootPid)) {
    let stat;
    let status;
    try {
      stat = readFileSync('/proc/' + pid + '/stat', 'utf8');
      status = readFileSync('/proc/' + pid + '/status', 'utf8');
    } catch { continue; }
    processes += 1;
    const tail = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    ticks += Number(tail[11]) + Number(tail[12]);
    const match = status.match(/VmRSS:\s+(\d+) kB/);
    if (match) rss += Number(match[1]);
  }
  return { ticks, rss_kb: rss, processes };
}

function processRss(pid) {
  try {
    const status = readFileSync('/proc/' + pid + '/status', 'utf8');
    const match = status.match(/VmRSS:\s+(\d+) kB/);
    return match ? Number(match[1]) : 0;
  } catch {
    return 0;
  }
}

async function statsSnapshot(cookie) {
  const reply = await fetch(base.replace(/\/$/, '') + '/api/live/stats', { headers: { cookie } });
  if (!reply.ok) throw new Error('live stats answered ' + reply.status);
  return reply.json();
}

function transcodePublishers(stats) {
  const streams = Array.isArray(stats.streams) ? stats.streams : [];
  return streams
    .filter((stream) => stream.mode === 'transcode' && stream.publishers > 0)
    .reduce((total, stream) => total + stream.publishers, 0);
}

function findServer() {
  if (process.env.PROVIEW_SERVER_PID) return Number(process.env.PROVIEW_SERVER_PID);
  const port = new URL(base).port || '80';
  let fallback = 0;
  for (const entry of readdirSync('/proc')) {
    if (!/^\d+$/.test(entry)) continue;
    let cmd;
    try { cmd = readFileSync('/proc/' + entry + '/cmdline', 'utf8'); } catch { continue; }
    if (!cmd.includes('proview-server')) continue;
    if (cmd.includes('--listen') && cmd.includes(':' + port)) return Number(entry);
    if (!fallback) fallback = Number(entry);
  }
  return fallback;
}

const cookies = async () => {
  const tab = await Tab.open(base);
  await sleep(1200);
  await login(tab);
  if (!(await tab.waitFor(devicesReady, 60, 200))) {
    throw new Error('the device list never appeared; check the login fixture');
  }
  const reply = await tab.send('Storage.getCookies');
  const all = (reply.result && reply.result.cookies) || [];
  tab.close();
  return all.map((cookie) => cookie.name + '=' + cookie.value).join('; ');
};

async function measureAvc() {
  const first = await Tab.open(base);
  await sleep(1200);
  await login(first);
  if (!(await first.waitFor(devicesReady, 60, 200))) {
    throw new Error('the device list never appeared; check the login fixture');
  }
  const extra = [];
  for (let i = 1; i < viewers; i++) {
    const tab = await Tab.open(base);
    if (!(await tab.waitFor(devicesReady, 60, 200))) {
      throw new Error('a second viewer never reached the device list');
    }
    extra.push(tab);
  }
  const tabs = [first, ...extra];
  const started = Date.now();
  const frames = [];
  for (const tab of tabs) {
    await tab.activate();
    await sleep(150);
    const clicked = Date.now();
    await tab.evaluate(clickFirstDevice);
    let elapsed = -1;
    for (let i = 0; i < 400; i++) {
      if (await tab.evaluate(playing)) {
        elapsed = Date.now() - clicked;
        break;
      }
      await sleep(50);
    }
    frames.push(elapsed);
  }
  const before = sampleTree(serverPid);
  const wall = Date.now();
  await sleep(sampleMs);
  const after = sampleTree(serverPid);
  const elapsed = (Date.now() - wall) / 1000;
  const states = await Promise.all(tabs.map((tab) => tab.evaluate(videoState)));
  for (const tab of tabs) tab.close();
  const cpuPercent = ((after.ticks - before.ticks) / 100 / elapsed) * 100;
  return {
    mode: 'avc-transcode',
    viewers,
    started_at: new Date(started).toISOString(),
    click_to_first_frame_ms: frames,
    video: states,
    cpu_percent_of_one_core: Number(cpuPercent.toFixed(1)),
    cpu_percent_per_viewer: Number((cpuPercent / viewers).toFixed(1)),
    rss_mib: Number((after.rss_kb / 1024).toFixed(1)),
    server_rss_mib: Number((processRss(serverPid) / 1024).toFixed(1)),
    ffmpeg_processes: Math.max(0, after.processes - 1),
    processes: after.processes,
    sample_seconds: elapsed,
  };
}

async function measureHevc() {
  const cookie = await cookies();
  const device = process.env.PROVIEW_DEVICE || 'fixture-dev-0001';
  const path = '/ws/live?device=' + encodeURIComponent(device) + '&channel=1&quality=main&codec=hevc';
  const target = base.replace(/^http/, 'ws').replace(/\/$/, '') + path;
  const warm = transcodePublishers(await statsSnapshot(cookie));
  if (warm > 0) {
    throw new Error(
      'a transcode source for this camera is still warm (' + warm + ' publishers); ' +
      'wait out the 60 s warm window before sampling passthrough',
    );
  }
  const before = sampleTree(serverPid);
  const wall = Date.now();
  const sockets = [];
  for (let index = 0; index < viewers; index++) {
    sockets.push(new WebSocket(target, { headers: { cookie } }));
  }
  const socket = sockets[0];
  socket.binaryType = 'arraybuffer';
  let hello = null;
  let init = null;
  let firstKeyframe = -1;
  let keyframes = 0;
  let deltas = 0;
  let bytes = 0;
  let otherBytes = 0;
  for (const extra of sockets.slice(1)) {
    extra.binaryType = 'arraybuffer';
    extra.onmessage = (event) => {
      if (typeof event.data !== 'string') otherBytes += event.data.byteLength;
    };
  }
  const started = Date.now();
  const decoder = spawn('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-f', 'mp4', '-i', 'pipe:0', '-fps_mode', 'passthrough', '-f', 'null', '-'], { stdio: ['pipe', 'ignore', 'pipe'] });
  let stderr = '';
  decoder.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
  const finished = new Promise((resolve) => decoder.on('exit', resolve));
  socket.onmessage = (event) => {
    if (typeof event.data === 'string') {
      hello = JSON.parse(event.data);
      return;
    }
    const view = Buffer.from(event.data);
    bytes += view.length;
    decoder.stdin.write(view);
    if (init === null) { init = view.length; return; }
    const trun = view.indexOf('trun');
    if (trun < 0) return;
    if (trun + 32 > view.length) return;
    if (view.readUInt32BE(trun + 28) & 0x02000000) {
      keyframes += 1;
      if (firstKeyframe < 0) firstKeyframe = Date.now() - started;
    } else {
      deltas += 1;
    }
  };
  await new Promise((resolve, reject) => {
    let ready = 0;
    for (const item of sockets) {
      item.onopen = () => {
        ready += 1;
        if (ready === sockets.length) resolve();
      };
      item.onerror = () => reject(new Error('the live socket failed to connect'));
    }
  });
  await sleep(sampleSeconds * 1000);
  const after = sampleTree(serverPid);
  const elapsed = (Date.now() - wall) / 1000;
  socket.onmessage = null;
  for (const item of sockets) item.close();
  decoder.stdin.end();
  await Promise.race([finished, sleep(5000)]);
  const decodeErrors = stderr.split('\n').filter((line) => line.trim() && line.indexOf('[null @') === -1);
  if (decodeErrors.length) {
    throw new Error('the passthrough bytes did not decode: ' + decodeErrors.join('\n').slice(0, 300));
  }
  if (after.processes !== 1) {
    throw new Error(
      'the passthrough sample ran ' + after.processes + ' processes; ffmpeg must not be involved',
    );
  }
  const cpuPercent = ((after.ticks - before.ticks) / 100 / elapsed) * 100;
  return {
    mode: 'hevc-passthrough',
    viewers,
    other_viewer_bytes: otherBytes,
    hello,
    init_bytes: init,
    first_keyframe_ms: firstKeyframe,
    keyframes,
    deltas,
    media_bytes: bytes,
    bytes_per_second: Math.round(bytes / elapsed),
    decoded_without_error: stderr.trim() === '',
    cpu_percent_of_one_core: Number(cpuPercent.toFixed(1)),
    cpu_percent_per_viewer: Number((cpuPercent / viewers).toFixed(2)),
    rss_mib: Number((after.rss_kb / 1024).toFixed(1)),
    server_rss_mib: Number((processRss(serverPid) / 1024).toFixed(1)),
    processes: after.processes,
    sample_seconds: elapsed,
  };
}

const serverPid = findServer();
if (!serverPid) {
  console.error('proview-server is not running; set PROVIEW_SERVER_PID');
  process.exit(1);
}

let report;
try {
  report = mode === 'hevc' ? await measureHevc() : await measureAvc();
} catch (error) {
  console.error(error.message);
  chrome.kill();
  process.exit(1);
}
report.base = base;
report.server_pid = serverPid;
console.log(JSON.stringify(report, null, 2));
if (out) writeFileSync(out, JSON.stringify(report, null, 2));
chrome.kill();
