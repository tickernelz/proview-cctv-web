import { expect, type Page } from '@playwright/test'
import { PER_BROWSER, type Scenario } from '../scenarios.mjs'

export const ACCOUNT = { email: 'fixture@example.invalid', password: 'fixture-password' }

const servers = JSON.parse(process.env.PROVIEW_E2E_SERVERS ?? '{}') as Record<string, string>

export const serverFor0 = (scenario: Scenario) => servers[scenario] + '/'

async function serverFor(scenario: Scenario, page: Page): Promise<string> {
  let name: string = scenario
  if (PER_BROWSER.includes(scenario)) {
    const agent = await page.evaluate(() => navigator.userAgent)
    name = scenario + '-' + (/Chrome\//.test(agent) ? 'chromium' : 'webkit')
  }
  const base = servers[name]
  if (!base) throw new Error('no fixture server named ' + name + '; the global setup did not run')
  return base
}

export async function signIn(page: Page, scenario: Scenario): Promise<string> {
  const base = await serverFor(scenario, page)
  await page.goto(base + '/')
  const status = await page.evaluate(
    (account) =>
      fetch('/api/login', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(account),
      }).then((reply) => reply.status),
    ACCOUNT,
  )
  expect(status).toBe(200)
  await page.goto(base + '/?signed-in#live')
  return base
}

export async function playing(page: Page) {
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-state', 'playing', { timeout: 20_000 })
}
