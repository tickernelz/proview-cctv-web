import { test } from '@e2e-dev/web'
import type { Browser } from '@e2e-dev/web'
import type { App, Screen } from 'e2e'
import { expect } from 'e2e'
import { playing, signIn } from './app.ts'

const supported = (feature: string, lens: string) => ({ feature, lens, backend: 'dvrip', state: 'supported', evidence: 'e2e fixture' })

async function fakeDeviceControls(browser: Browser) {
  const calls: string[] = []
  let light = 'off'
  await browser.route(/\/api\/device\/controls/, async (route) => {
    const request = route.request
    const url = new URL(request.url)
    calls.push(request.method + ' ' + url.pathname + url.search)
    const json = (body: unknown) => route.fulfill({ json: body })
    if (url.pathname === '/api/device/controls') {
      return json({
        onvif: false,
        dvrip: true,
        capabilities: [supported('ptz move', 'bottom'), supported('ptz preset', 'bottom'), supported('white light', 'device')],
      })
    }
    if (url.pathname.endsWith('/presets')) return json([{ token: '1', name: 'Gate' }, { token: '2', name: 'Driveway' }])
    if (url.pathname.endsWith('/preset/goto')) return json({ ok: true })
    if (url.pathname.endsWith('/ptz/stop')) return json({ detail: 'Stopped.' })
    if (url.pathname.endsWith('/ptz')) return json({ lens: url.searchParams.get('lens'), backend: 'dvrip', hold_ms: 600 })
    if (url.pathname.endsWith('/light')) {
      if (request.method === 'POST') light = url.searchParams.get('mode') ?? light
      return json({ mode: light })
    }
    return route.fulfill({ status: 404, json: { message: 'not faked' } })
  })
  return calls
}

async function openLive(app: App, screen: Screen, browser: Browser) {
  const calls = await fakeDeviceControls(browser)
  await signIn(app, browser, 'one')
  await playing(screen)
  await expect(screen.getByTestId('ctl-move')).toBeEnabled()
  return calls
}

test('Move opens the pan and tilt pad and each arrow and preset reaches the device', async ({ app, screen, browser }) => {
  const calls = await openLive(app, screen, browser)
  await screen.getByTestId('ctl-move').tap()
  await expect(screen.getByTestId('ctl-move')).toHaveAttribute('aria-pressed', 'true')
  const pad = screen.getByTestId('ptz-pad')
  await expect(pad).toBeVisible()
  await pad.getByRole('button', 'Pan left').tap()
  await expect(screen.getByTestId('control-status')).toHaveText('Moved the bottom lens over dvrip for 600 ms.')
  expect(calls).toContain('POST /api/device/controls/ptz?lens=bottom&pan=-0.5&tilt=0&hold_ms=600')
  await expect(pad.getByTestId('ptz-preset')).toHaveCount(2)
  await pad.getByTestId('ptz-preset').filter({ hasText: 'Driveway' }).tap()
  await expect(screen.getByTestId('control-status')).toHaveText('At Driveway.')
  expect(calls).toContain('POST /api/device/controls/preset/goto?token=2')
  await screen.getByTestId('ctl-move').tap()
  await expect(pad).toBeHidden()
})

test('Record saves a clip where the browser can record, and says why where it cannot', async ({ app, screen, browser }) => {
  await openLive(app, screen, browser)
  const record = screen.getByTestId('ctl-record')
  await expect(record).toBeEnabled()
  const canRecord = await browser.evaluate(
    () => 'captureStream' in HTMLVideoElement.prototype && typeof MediaRecorder !== 'undefined',
  )
  if (!canRecord) {
    await record.tap()
    await expect(screen.getByTestId('control-status')).toHaveText('Recording needs a playing picture and a browser with captureStream.')
    await expect(record).toHaveAttribute('aria-pressed', 'false')
    return
  }
  await record.tap()
  await expect(record).toHaveAttribute('aria-pressed', 'true')
  await expect(record).toContainText(/\d+:\d{2}/)
  const file = await browser.waitForDownload(() => record.tap())
  await expect(record).toHaveAttribute('aria-pressed', 'false')
  expect(file.suggestedFilename).toMatch(/\.(webm|mp4)$/)
})

test('the white light switch and the upside-down flip apply from Settings', async ({ app, screen, browser }) => {
  const calls = await openLive(app, screen, browser)
  await screen.getByTestId('open-settings').tap()
  const settings = screen.getByTestId('settings')
  await expect(settings).toBeVisible()
  await settings.getByRole('link', 'This camera').tap()
  await screen.getByTestId('ctl-light').tap()
  await expect(screen.getByTestId('light-status')).toHaveText('White light: always (it was off).')
  expect(calls).toContain('POST /api/device/controls/light?mode=always')
  await settings.getByRole('link', 'Playback').tap()
  await screen.getByTestId('set-flip').check()
  await screen.getByTestId('settings-back').tap()
  await expect(screen.getByTestId('live')).toBeVisible()
  await expect(screen.getByTestId('live-frame')).toHaveAttribute('data-flipped', 'true')
})
