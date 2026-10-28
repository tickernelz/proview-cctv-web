import { createServer } from 'node:http';
import { readFileSync, writeFileSync } from 'node:fs';

const [port, out] = [Number(process.argv[2]), process.argv[3]];
const page = readFileSync(new URL('./probe.html', import.meta.url));
const server = createServer((req, res) => {
  if (req.method === 'POST' && req.url === '/result') {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      writeFileSync(out, body + '\n');
      res.end('ok');
      server.close();
      process.exit(0);
    });
    return;
  }
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.end(page);
});
server.listen(port, '0.0.0.0');
setTimeout(() => process.exit(2), Number(process.env.PROBE_WAIT_MS ?? 60000));
