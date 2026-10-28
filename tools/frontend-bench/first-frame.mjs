import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, normalize } from 'node:path';

const [engine, dir, runsArg, readyArg] = process.argv.slice(2);
const runs = Number(runsArg ?? 5);
const readyCount = Number(readyArg ?? 9);
const port = Number(process.env.PORT ?? 18450);
const devices = Array.from({ length: 8 }, (_, i) => ({ id: 'd' + i, name: 'Camera ' + i, online: i % 2 === 0 }));
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.woff2': 'font/woff2', '.json': 'application/json' };
const probe = `<script>
(() => {
  const out = { fp: null, fcp: null, ready: null };
  const send = () => fetch('/__result', { method: 'POST', body: JSON.stringify(out) });
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) { if (e.name === 'first-paint') out.fp = e.startTime; if (e.name === 'first-contentful-paint') out.fcp = e.startTime; }
  }).observe({ type: 'paint', buffered: true });
  const target = ${readyCount};
  const check = () => {
    if (out.ready !== null || target === 0) return;
    if (document.querySelectorAll('.rail button').length >= target) {
      requestAnimationFrame(() => { out.ready = performance.now(); });
    }
  };
  new MutationObserver(check).observe(document, { childList: true, subtree: true });
  setTimeout(send, 3000);
})();
</script>`;
let waiting = null;
const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  if (req.method === 'POST' && url.pathname === '/__result') {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => { res.end('ok'); waiting?.(JSON.parse(body)); });
    return;
  }
  if (url.pathname === '/api/devices') {
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify(devices));
    return;
  }
  let path = normalize(join(dir, url.pathname));
  try { if (statSync(path).isDirectory()) path = join(path, 'index.html'); } catch { path = join(dir, 'index.html'); }
  let body;
  try { body = readFileSync(path); } catch { path = join(dir, 'index.html'); body = readFileSync(path); }
  if (path.endsWith('.html')) body = Buffer.from(body.toString().replace('<head>', '<head>' + probe).replace(/<head ([^>]*)>/, '<head $1>' + probe));
  res.setHeader('content-type', types[extname(path)] ?? 'application/octet-stream');
  res.setHeader('cache-control', 'no-store');
  res.end(body);
});
await new Promise((ok) => server.listen(port, '127.0.0.1', ok));
const results = [];
for (let i = 0; i < runs; i++) {
  const unit = 'zha4133-first-frame-' + process.pid + '-' + i;
  const profile = mkdtempSync(join(tmpdir(), 'zha4133-ff-'));
  const url = 'http://127.0.0.1:' + port + '/';
  const cmd = engine === 'chrome'
    ? ['google-chrome', '--headless=new', '--no-sandbox', '--no-first-run', '--user-data-dir=' + profile, url]
    : ['env', 'GDK_BACKEND=x11', 'xvfb-run', '-a', '/usr/libexec/webkit2gtk-4.1/MiniBrowser', url];
  const started = performance.now();
  const result = new Promise((ok) => { waiting = ok; });
  spawn('systemd-run', ['--user', '--scope', '-q', '--collect', '-u', unit, ...cmd], { stdio: 'ignore', env: { ...process.env, HOME: profile } });
  const timer = new Promise((ok) => setTimeout(() => ok({ timeout: true }), 30000));
  const r = await Promise.race([result, timer]);
  r.launch_to_report_ms = performance.now() - started;
  spawnSync('systemctl', ['--user', 'stop', unit + '.scope']);
  rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  results.push(r);
}
server.close();
const median = (xs) => { const v = xs.filter((x) => typeof x === 'number').sort((a, b) => a - b); return v.length ? Math.round(v[(v.length - 1) >> 1]) : null; };
if (process.env.DEBUG) console.error(JSON.stringify(results));
console.log(JSON.stringify({ engine, dir, runs, fp_ms_median: median(results.map((r) => r.fp)), fcp_ms_median: median(results.map((r) => r.fcp)), ready_ms_median: median(results.map((r) => r.ready)), fcp_ms_all: results.map((r) => r.fcp && Math.round(r.fcp)), ready_ms_all: results.map((r) => r.ready && Math.round(r.ready)) }));
