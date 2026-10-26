#!/usr/bin/env node
import { spawn } from 'node:child_process';

const base = process.env.PROVIEW_BASE || 'http://127.0.0.1:8787/';
const email = process.env.PROVIEW_MEASURE_EMAIL || 'fixture@example.invalid';
const password = process.env.PROVIEW_MEASURE_PASSWORD || 'fixture-password';
const port = Number(process.env.PROVIEW_CDP_PORT || 9333);
const profile = process.env.PROVIEW_CDP_PROFILE || '/tmp/proview-measure-profile';

const chrome = spawn('google-chrome', [
  '--headless=new', '--no-sandbox', '--disable-gpu',
  '--autoplay-policy=no-user-gesture-required',
  '--remote-debugging-port=' + port, '--window-size=1440,900',
  '--user-data-dir=' + profile, 'about:blank',
], { stdio: 'ignore' });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const call = async (path, init) => (await fetch('http://127.0.0.1:' + port + path, init)).json();

let up = false;
for (let i = 0; i < 80 && !up; i++) {
  try { await call('/json/version'); up = true; } catch { await sleep(250); }
}
if (!up) {
  console.error('chrome did not expose the debugging port');
  process.exit(1);
}

const target = await call('/json/new?' + encodeURIComponent(base), { method: 'PUT' });
const socket = new WebSocket(target.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
socket.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  }
};
await new Promise((resolve) => { socket.onopen = resolve; });

const send = (method, params) => new Promise((resolve) => {
  const messageId = ++id;
  pending.set(messageId, resolve);
  socket.send(JSON.stringify({ id: messageId, method, params: params || {} }));
});

const evaluate = async (expression) => {
  const reply = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  const result = reply.result || {};
  if (result.exceptionDetails) return null;
  return result.result ? result.result.value : null;
};

await send('Page.enable');
await send('Runtime.enable');
await sleep(1200);

await evaluate("(function(){var e=document.querySelector('#login-form input[type=email]');var p=document.querySelector('#login-form input[type=password]');if(!e)return 0;e.value=" + JSON.stringify(email) + ";p.value=" + JSON.stringify(password) + ";document.querySelector('#login-submit').click();return 1;})()");

for (let i = 0; i < 60; i++) {
  const count = await evaluate("document.querySelectorAll('#devices li button').length");
  if (typeof count === 'number' && count > 0) break;
  await sleep(200);
}

const started = Date.now();
await evaluate("(function(){var b=document.querySelector('#devices li button');if(!b)return 0;b.click();return 1;})()");

let firstFrame = -1;
for (let i = 0; i < 400; i++) {
  const playing = await evaluate("(function(){var v=document.querySelector('video');return !!(v && v.readyState>=2 && v.currentTime>0);})()");
  if (playing === true) { firstFrame = Date.now() - started; break; }
  await sleep(50);
}

const state = await evaluate("(function(){var v=document.querySelector('video');if(!v)return null;return {width:v.videoWidth,height:v.videoHeight,time:v.currentTime,readyState:v.readyState};})()");
console.log(JSON.stringify({ base, click_to_first_frame_ms: firstFrame, video: state }, null, 2));

socket.close();
chrome.kill();
