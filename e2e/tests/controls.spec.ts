import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

const supported = (feature: string, lens: string) => ({ feature, lens, backend: 'dvrip', state: 'supported', evidence: 'e2e fixture' })

async function fakeDeviceControls(page: Page) {
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
        capabilities: [supported('ptz move', 'bottom'), supported('ptz preset', 'bottom'), supported('white light', 'device')],
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

async function openLive(page: Page) {
  const calls = await fakeDeviceControls(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('ctl-move')).toBeEnabled()
  return calls
}

test('Move opens the pan and tilt pad and each arrow and preset reaches the device', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ctl-move')).toHaveAttribute('aria-pressed', 'true')
  const pad = page.getByTestId('ptz-pad')
  await expect(pad).toBeVisible()
  await pad.getByRole('button', { name: 'Pan left', exact: true }).click()
  await expect(page.getByTestId('control-status')).toHaveText('Moved the bottom lens over dvrip for 600 ms.')
  expect(calls).toContain('POST /api/device/controls/ptz?lens=bottom&pan=-0.5&tilt=0&hold_ms=600')
  await expect(pad.getByTestId('ptz-preset')).toHaveCount(2)
  await pad.getByTestId('ptz-preset').filter({ hasText: 'Driveway' }).click()
  await expect(page.getByTestId('control-status')).toHaveText('At Driveway.')
  expect(calls).toContain('POST /api/device/controls/preset/goto?token=2')
  await page.getByTestId('ctl-move').click()
  await expect(pad).toBeHidden()
})

test('Record saves a clip where the browser can record, and says why where it cannot', async ({ page }) => {
  await openLive(page)
  const record = page.getByTestId('ctl-record')
  await expect(record).toBeEnabled()
  const canRecord = await page.evaluate(
    () => 'captureStream' in HTMLVideoElement.prototype && typeof MediaRecorder !== 'undefined',
  )
  if (!canRecord) {
    await record.click()
    await expect(page.getByTestId('control-status')).toHaveText('Recording needs a playing picture and a browser with captureStream.')
    await expect(record).toHaveAttribute('aria-pressed', 'false')
    return
  }
  await record.click()
  await expect(record).toHaveAttribute('aria-pressed', 'true')
  await expect(record).toContainText(/\d+:\d{2}/)
  const [file] = await Promise.all([page.waitForEvent('download'), record.click()])
  await expect(record).toHaveAttribute('aria-pressed', 'false')
  expect(file.suggestedFilename()).toMatch(/\.(webm|mp4)$/)
})

test('the white light switch and the upside-down flip apply from Settings', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('open-settings').click()
  const settings = page.getByTestId('settings')
  await expect(settings).toBeVisible()
  await settings.getByRole('link', { name: 'This camera', exact: true }).click()
  await page.getByTestId('ctl-light').click()
  await expect(page.getByTestId('light-status')).toHaveText('White light: always (it was off).')
  expect(calls).toContain('POST /api/device/controls/light?mode=always')
  await settings.getByRole('link', { name: 'Playback', exact: true }).click()
  await page.getByTestId('set-flip').check()
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect(page.getByTestId('live-frame')).toHaveAttribute('data-flipped', 'true')
})
