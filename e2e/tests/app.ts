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

export async function eventsPanelClosed(page: Page) {
  await page.addInitScript(() => {
    if (localStorage.getItem('pvw_events_panel') === null) localStorage.setItem('pvw_events_panel', 'closed')
  })
}

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
  await page.goto(base + '/?signed-in#/live')
  return base
}

export async function playing(page: Page) {
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-state', 'playing', { timeout: 20_000 })
}

const supported = (feature: string, lens: string) => ({ feature, lens, backend: 'dvrip', state: 'supported', evidence: 'e2e fixture' })

export type ControlsFixture = { onvif?: boolean; extra?: string[]; drop?: string[] }

const ONVIF_PRESETS = [
  { token: '0', name: 'Gate' },
  { token: '1', name: 'Driveway' },
  { token: '2', name: 'Home' },
]

export async function fakeDeviceControls(page: Page, fixture: ControlsFixture = {}) {
  const calls: string[] = []
  let light = 'off'
  const presets = [
    { channel: 0, id: 1, name: 'Gate' },
    { channel: 0, id: 2, name: 'Driveway' },
    ...(fixture.onvif ? [{ channel: 0, id: 250, name: 'Home' }] : []),
  ]
  const features = ['ptz move', 'ptz preset', 'ptz preset edit', ...(fixture.extra ?? [])].filter((feature) => !(fixture.drop ?? []).includes(feature))
  const tour = [1, 2]
  const lines = () => [{ channel: 0, id: 1, name: 'Yard', points: tour.map((preset) => ({ preset, seconds: 5 })) }]
  await page.route(/\/api\/device\/controls/, async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    calls.push(request.method() + ' ' + url.pathname + url.search)
    const json = (body: unknown) => route.fulfill({ json: body })
    if (url.pathname === '/api/device/controls') {
      return json({
        onvif: fixture.onvif === true,
        dvrip: true,
        capabilities: [
          ...features.map((feature) => supported(feature, 'bottom')),
          supported('ptz aim', 'top'),
          supported('white light', 'device'),
          supported('two-way audio', 'device'),
        ],
      })
    }
    if (url.pathname.endsWith('/presets') && fixture.onvif) return json(ONVIF_PRESETS)
    if (url.pathname.endsWith('/presets')) return json(presets.map((preset) => ({ token: String(preset.id), name: preset.name })))
    if (url.pathname.endsWith('/ptz/press')) return json({ action: url.searchParams.get('action'), command: 'DirectionLeft', session_ms: 12, press_ms: 4 })
    if (url.pathname.endsWith('/ptz/release')) return json({ held: true, times: { held_ms: Number(url.searchParams.get('pressed_ms')), stop_ms: 3 } })
    if (url.pathname.endsWith('/ptz/calibrate')) return json({ session_ms: 12, calibrate_ms: 1200 })
    if (url.pathname.endsWith('/ptz/locate')) return json({ xoffset: 0, yoffset: 0, session_ms: 12, locate_ms: 4 })
    if (url.pathname.endsWith('/tour')) return json({ presets, lines: lines() })
    if (url.pathname.endsWith('/tour/remove')) {
      const stop = tour.indexOf(Number(url.searchParams.get('preset')))
      if (stop >= 0) tour.splice(stop, 1)
      return json({ tour: 1, lines: lines() })
    }
    if (url.pathname.endsWith('/preset/add')) {
      const id = [1, 2, 3, 4].find((candidate) => !presets.some((preset) => preset.id === candidate)) ?? 9
      const name = url.searchParams.get('name') || 'Preset' + id
      presets.push({ channel: 0, id, name })
      return json({ id, name, presets, lines: lines() })
    }
    if (url.pathname.endsWith('/preset/rename')) {
      const entry = presets.find((preset) => String(preset.id) === url.searchParams.get('token'))
      if (entry) entry.name = url.searchParams.get('name') ?? entry.name
      return json({ id: entry?.id, name: entry?.name, presets, lines: lines() })
    }
    if (url.pathname.endsWith('/preset/delete')) {
      const index = presets.findIndex((preset) => String(preset.id) === url.searchParams.get('token'))
      const [gone] = presets.splice(index, 1)
      return json({ id: gone.id, name: gone.name, presets, lines: lines() })
    }
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

export async function openLive(page: Page, fixture: ControlsFixture = {}) {
  const calls = await fakeDeviceControls(page, fixture)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('ctl-move')).toBeEnabled()
  return calls
}
