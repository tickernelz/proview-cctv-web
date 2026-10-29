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

const supported = (feature: string, lens: string) => ({ feature, lens, backend: 'dvrip', state: 'supported', evidence: 'e2e fixture' })

export async function fakeDeviceControls(page: Page) {
  const calls: string[] = []
  let light = 'off'
  await page.route(/\/api\/device\/controls/, async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    calls.push(request.method() + ' ' + url.pathname + url.search)
    const json = (body: unknown) => route.fulfill({ json: body })
    if (url.pathname === '/api/device/controls') {
      return json({
        onvif: false,
        dvrip: true,
        capabilities: [supported('ptz move', 'bottom'), supported('ptz preset', 'bottom'), supported('white light', 'device'), supported('two-way audio', 'device')],
      })
    }
    if (url.pathname.endsWith('/presets')) return json([{ token: '1', name: 'Gate' }, { token: '2', name: 'Driveway' }])
    if (url.pathname.endsWith('/preset/goto')) return json({ ok: true })
    if (url.pathname.endsWith('/ptz/stop')) return json({ detail: 'Stopped.' })
    if (url.pathname.endsWith('/ptz')) return json({ lens: url.searchParams.get('lens'), backend: 'dvrip', hold_ms: 600 })
    if (url.pathname.endsWith('/light')) {
      if (request.method() === 'POST') light = url.searchParams.get('mode') ?? light
      return json({ mode: light })
    }
    return route.fulfill({ status: 404, json: { message: 'not faked' } })
  })
  return calls
}

export async function openLive(page: Page) {
  const calls = await fakeDeviceControls(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('ctl-move')).toBeEnabled()
  return calls
}
