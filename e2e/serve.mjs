import { spawn } from 'node:child_process'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, copyFileSync, symlinkSync, renameSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { randomBytes } from 'node:crypto'
import { SCENARIOS, SERVERS_FILE, serverNames } from './scenarios.mjs'

const ROOT = resolve(import.meta.dirname, '..')
const CLIP = join(ROOT, 'fixtures', 'dual-lens', 'dual-lens-stacked-640x720.h264')
const BINARY = process.env.PROVIEW_SERVER_BIN || join(ROOT, 'target', 'debug', 'proview-server')

function freePort() {
  return new Promise((done, fail) => {
    const probe = createServer()
    probe.once('error', fail)
    probe.listen(0, '127.0.0.1', () => {
      const { port } = probe.address()
      probe.close(() => done(port))
    })
  })
}

async function start(name, dir) {
  const devices = SCENARIOS[name.split('-')[0]]
  const fixtures = join(dir, name, 'fixtures')
  const data = join(dir, name, 'data')
  mkdirSync(fixtures, { recursive: true })
  mkdirSync(data, { recursive: true })
  copyFileSync(join(ROOT, 'fixtures', 'account.json'), join(fixtures, 'account.json'))
  writeFileSync(join(fixtures, 'devices.json'), JSON.stringify(devices))
  for (const device of devices) {
    for (const quality of ['main', 'sub']) symlinkSync(CLIP, join(fixtures, device.id + '_ch1_' + quality + '.h264'))
  }
  const port = await freePort()
  const child = spawn(BINARY, [], {
    cwd: ROOT,
    env: {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      LD_LIBRARY_PATH: process.env.LD_LIBRARY_PATH ?? '',
      RUST_LOG: process.env.RUST_LOG ?? 'warn',
      PROVIEW_PROVIDER: 'fixture',
      PROVIEW_FIXTURE_DIR: fixtures,
      PROVIEW_DATA_DIR: data,
      PROVIEW_LISTEN: '127.0.0.1:' + port,
      ROCKET_SECRET_KEY: randomBytes(32).toString('base64'),
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  })
  let log = ''
  child.stderr.on('data', (chunk) => {
    log = (log + chunk).slice(-8000)
  })
  const base = 'http://127.0.0.1:' + port
  const end = Date.now() + 30_000
  for (;;) {
    if (child.exitCode !== null) throw new Error(name + ': proview-server exited ' + child.exitCode + '\n' + log)
    if (await fetch(base + '/api/health').then((reply) => reply.ok, () => false)) return { name, base, child }
    if (Date.now() > end) throw new Error(name + ': no answer on ' + base + '/api/health in 30 s\n' + log)
    await new Promise((done) => setTimeout(done, 100))
  }
}

const dir = mkdtempSync(join(tmpdir(), 'proview-e2e-'))
const running = []
const stop = (code) => {
  for (const { child } of running) child.kill('SIGKILL')
  rmSync(SERVERS_FILE, { force: true })
  rmSync(dir, { recursive: true, force: true, maxRetries: 5 })
  process.exit(code)
}
process.on('SIGINT', () => stop(0))
process.on('SIGTERM', () => stop(0))

try {
  running.push(...(await Promise.all(serverNames().map((name) => start(name, dir)))))
} catch (error) {
  console.error(String(error))
  stop(1)
}
for (const server of running) {
  server.child.once('exit', (code) => {
    console.error(server.name + ': proview-server exited ' + code)
    stop(1)
  })
}
const partial = SERVERS_FILE + '.partial'
writeFileSync(partial, JSON.stringify(Object.fromEntries(running.map(({ name, base }) => [name, base])), null, 2))
renameSync(partial, SERVERS_FILE)
console.log('serving ' + running.length + ' fixture servers; URLs in ' + SERVERS_FILE)
