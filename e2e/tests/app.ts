import { readFileSync } from 'node:fs'
import { expect } from 'e2e'
import type { App, Screen } from 'e2e'
import type { Browser } from '@e2e-dev/web'
import { PER_BROWSER, SERVERS_FILE, type Scenario } from '../scenarios.mjs'

export const ACCOUNT = { email: 'fixture@example.invalid', password: 'fixture-password' }

const servers = JSON.parse(readFileSync(SERVERS_FILE, 'utf8')) as Record<string, string>

export async function serverFor(scenario: Scenario, browser: Browser): Promise<string> {
  let name: string = scenario
  if (PER_BROWSER.includes(scenario)) {
    const agent = await browser.evaluate(() => navigator.userAgent)
    name = scenario + '-' + (/Chrome\//.test(agent) ? 'chromium' : 'webkit')
  }
  const base = servers[name]
  if (!base) throw new Error('no fixture server named ' + name + ' in ' + SERVERS_FILE)
  return base
}

export async function signIn(app: App, browser: Browser, scenario: Scenario): Promise<string> {
  await app.open('about:blank')
  const base = await serverFor(scenario, browser)
  await app.open(base + '/')
  const status = await browser.evaluate(
    (account: { email: string; password: string }) =>
      fetch('/api/login', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(account),
      }).then((reply) => reply.status),
    ACCOUNT,
  )
  expect(status).toBe(200)
  await app.open(base + '/?signed-in#live')
  return base
}

export async function playing(screen: Screen) {
  await expect(screen.getByTestId('live-well')).toHaveAttribute('data-state', 'playing', { timeout: 20_000 })
}
