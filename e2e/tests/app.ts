import { expect, test, type Page } from '@playwright/test'
import { PER_BROWSER, type Scenario } from '../scenarios.mjs'

export const ACCOUNT = { email: 'fixture@example.invalid', password: 'fixture-password' }

const servers = JSON.parse(process.env.PROVIEW_E2E_SERVERS ?? '{}') as Record<string, string>

function serverBase(scenario: Scenario): string {
  const name = PER_BROWSER.includes(scenario) ? scenario + '-' + test.info().project.name : scenario
  const base = servers[name]
  if (!base) throw new Error('no fixture server named ' + name + '; the global setup did not run')
  return base
}

export const scenarioUrl = (scenario: Scenario) => serverBase(scenario) + '/'

export async function signIn(page: Page, scenario: Scenario): Promise<string> {
  const base = serverBase(scenario)
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
