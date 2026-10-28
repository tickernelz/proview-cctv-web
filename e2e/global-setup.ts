import { existsSync, readFileSync } from 'node:fs'
import { SERVERS_FILE } from './scenarios.mjs'
import { startServers } from './serve.mjs'

export default async function globalSetup() {
  if (process.env.PROVIEW_E2E_REUSE === '1') {
    if (!existsSync(SERVERS_FILE)) throw new Error('PROVIEW_E2E_REUSE=1 but ' + SERVERS_FILE + ' is missing: start `npm run serve` first')
    process.env.PROVIEW_E2E_SERVERS = readFileSync(SERVERS_FILE, 'utf8')
    return
  }
  const { urls, stop } = await startServers()
  process.env.PROVIEW_E2E_SERVERS = JSON.stringify(urls)
  return stop
}
