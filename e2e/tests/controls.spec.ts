import { test, expect } from '@playwright/test'
import { openLive, playing } from './app'

test('Move opens the pan and tilt pad and each arrow and preset reaches the device', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ctl-move')).toHaveAttribute('aria-pressed', 'true')
  const pad = page.getByTestId('ptz-pad')
  await expect(pad).toBeVisible()
  const left = pad.getByRole('button', { name: 'Pan left', exact: true })
  const box = (await left.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await expect(page.getByTestId('control-status')).toContainText('release to stop')
  await expect.poll(() => calls).toContain('POST /api/device/controls/ptz/press?action=left&speed=normal&device=gate')
  await expect(page.getByTestId('control-status')).toContainText('ms to start')
  expect(calls.filter((call) => call.includes('/ptz/release'))).toEqual([])
  await page.waitForTimeout(700)
  await page.mouse.up()
  await expect(page.getByTestId('control-status')).toContainText('Stopped left after')
  const release = calls.find((call) => call.includes('/ptz/release')) || ''
  expect(Number(new URL('http://x' + release.split(' ')[1]).searchParams.get('pressed_ms'))).toBeGreaterThanOrEqual(600)
  expect(calls.filter((call) => call.startsWith('POST /api/device/controls/ptz?'))).toEqual([])
  await expect(pad.getByTestId('ptz-preset')).toHaveCount(2)
  await pad.getByTestId('ptz-preset').filter({ hasText: 'Driveway' }).click()
  await expect(page.getByTestId('control-status')).toHaveText('At Driveway.')
  expect(calls).toContain('POST /api/device/controls/preset/goto?token=2&device=gate')
  await page.getByTestId('ctl-move').click()
  await expect(pad).toBeHidden()
})

test('arrow keys hold the move until the key comes up', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ptz-pad')).toBeVisible()
  await page.keyboard.down('ArrowUp')
  await expect.poll(() => calls).toContain('POST /api/device/controls/ptz/press?action=up&speed=normal&device=gate')
  await page.keyboard.down('ArrowUp')
  await page.waitForTimeout(300)
  expect(calls.filter((call) => call.includes('/ptz/press')).length).toBe(1)
  await page.keyboard.up('ArrowUp')
  await expect.poll(() => calls.filter((call) => call.includes('/ptz/release')).length).toBe(1)
  await expect(page.getByTestId('control-status')).toContainText('Stopped up after')
})

test('the speed pick rides on every press and a turned picture swaps the keys', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('ctl-move').click()
  await page.getByTestId('ptz-speed').selectOption('fast')
  await page.keyboard.down('ArrowRight')
  await expect.poll(() => calls).toContain('POST /api/device/controls/ptz/press?action=right&speed=fast&device=gate')
  await page.keyboard.up('ArrowRight')
  await expect.poll(() => calls.filter((call) => call.includes('/ptz/release')).length).toBe(1)
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('pvw_prefs_v2') || '{}').ptzSpeed)).toEqual({ gate: 'fast' })
  await page.evaluate(() => {
    const prefs = JSON.parse(localStorage.getItem('pvw_prefs_v2') || '{}')
    localStorage.setItem('pvw_prefs_v2', JSON.stringify({ ...prefs, flip: true }))
  })
  await page.reload()
  await playing(page)
  await expect(page.getByTestId('live-frame')).toHaveAttribute('data-flipped', 'true')
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ptz-speed')).toHaveValue('fast')
  await page.keyboard.down('ArrowLeft')
  await expect.poll(() => calls).toContain('POST /api/device/controls/ptz/press?action=right&speed=fast&device=gate')
  await page.keyboard.up('ArrowLeft')
  await expect.poll(() => calls.filter((call) => call.includes('/ptz/release')).length).toBe(2)
  expect(calls.filter((call) => call.includes('action=left'))).toEqual([])
})

test('a click on the top lens aims the moving lens while the pad is open', async ({ page }) => {
  const calls = await openLive(page)
  const frame = page.getByTestId('live-frame')
  const box = (await frame.boundingBox())!
  await frame.click({ position: { x: box.width * 0.25, y: box.height * 0.125 } })
  expect(calls.filter((call) => call.includes('/ptz/locate'))).toEqual([])
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ptz-aim-hint')).toBeVisible()
  await frame.click({ position: { x: box.width * 0.25, y: box.height * 0.125 } })
  await expect.poll(() => calls.some((call) => call.includes('/ptz/locate'))).toBe(true)
  const sent = new URL('http://x' + calls.find((call) => call.includes('/ptz/locate'))!.split(' ')[1]).searchParams
  expect(Math.abs(Number(sent.get('x')) - 0.25)).toBeLessThan(0.01)
  expect(Math.abs(Number(sent.get('y')) - 0.25)).toBeLessThan(0.01)
  expect(sent.get('device')).toBe('gate')
  await expect(page.getByTestId('control-status')).toContainText('Aiming at 25% across, 25% down')
  await frame.click({ position: { x: box.width * 0.5, y: box.height * 0.75 } })
  await expect(page.getByTestId('control-status')).toHaveText('Click inside the top lens picture to aim the moving lens there.')
  expect(calls.filter((call) => call.includes('/ptz/locate')).length).toBe(1)
})

test('the preset editor saves, renames and deletes a preset and edits the tour', async ({ page }) => {
  const calls = await openLive(page)
  page.on('dialog', (dialog) => dialog.accept())
  await page.getByTestId('ctl-move').click()
  await page.getByTestId('ptz-edit').click()
  const editor = page.getByTestId('ptz-editor')
  await expect(editor).toBeVisible()
  await expect(editor.getByTestId('preset-new-name')).toHaveAttribute('placeholder', 'Preset3')
  await editor.getByTestId('preset-add').click()
  await expect(page.getByTestId('control-status')).toHaveText('Saved this position as Preset3 (preset 3).')
  expect(calls).toContain('POST /api/device/controls/preset/add?name=&device=gate')
  const row = editor.locator('[data-testid="preset-row"][data-token="3"]')
  await row.getByRole('textbox').fill('Porch')
  await row.getByTestId('preset-rename').click()
  await expect(page.getByTestId('control-status')).toHaveText('Renamed preset 3 to Porch.')
  await expect(page.getByTestId('ptz-preset').filter({ hasText: 'Porch' })).toHaveCount(1)
  await editor.getByTestId('tour-stops').getByTestId('tour-stop-remove').first().click()
  await expect.poll(() => calls).toContain('POST /api/device/controls/tour/remove?tour=1&preset=1&device=gate')
  await row.getByTestId('preset-delete').click()
  await expect(page.getByTestId('control-status')).toHaveText('Deleted Porch.')
  await expect(editor.getByTestId('preset-row')).toHaveCount(2)
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
  expect(calls).toContain('POST /api/device/controls/light?mode=always&device=gate')
  await settings.getByRole('link', { name: 'Playback', exact: true }).click()
  await page.getByTestId('set-flip').check()
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect(page.getByTestId('live-frame')).toHaveAttribute('data-flipped', 'true')
})

test('with ONVIF and DVRIP both present the editor works on DVRIP ids and the chips keep their ONVIF tokens', async ({ page }) => {
  const calls = await openLive(page, { onvif: true, extra: ['ptz zoom'] })
  let release = () => {}
  const held = new Promise<void>((done) => (release = done))
  await page.route(/\/api\/device\/controls\/tour\?/, async (route) => {
    await held
    await route.fallback()
  })
  const dialogs: string[] = []
  page.on('dialog', (dialog) => {
    dialogs.push(dialog.message())
    dialog.accept()
  })
  await page.getByTestId('ctl-move').click()
  const pad = page.getByTestId('ptz-pad')
  await expect(pad.getByTestId('ptz-preset')).toHaveCount(3)
  await page.getByTestId('ptz-edit').click()
  const editor = page.getByTestId('ptz-editor')
  await expect(editor.getByTestId('ptz-editor-loading')).toBeVisible()
  await expect(editor.getByTestId('preset-row')).toHaveCount(0)
  release()
  await expect(editor.getByTestId('preset-row')).toHaveCount(3)
  await expect(editor.locator('[data-testid="preset-row"][data-token="250"] input')).toHaveValue('Home')
  await editor.locator('[data-testid="preset-row"][data-token="2"]').getByTestId('preset-delete').click()
  await expect(page.getByTestId('control-status')).toHaveText('Deleted Driveway.')
  expect(dialogs).toEqual(['Are you sure you want to delete this preset? (Driveway)'])
  expect(calls).toContain('POST /api/device/controls/preset/delete?token=2&device=gate')
  expect(calls.filter((call) => call.includes('/preset/delete'))).toHaveLength(1)
  await expect(editor.getByTestId('preset-row')).toHaveCount(2)
  await expect(pad.getByTestId('ptz-preset')).toHaveCount(3)
  await pad.getByTestId('ptz-preset').filter({ hasText: 'Driveway' }).click()
  await expect(page.getByTestId('control-status')).toHaveText('At Driveway.')
  expect(calls).toContain('POST /api/device/controls/preset/goto?token=1&device=gate')
  await pad.getByTestId('ptz-zoom-in').click()
  await expect.poll(() => calls).toContain('POST /api/device/controls/ptz/press?action=zoom-in&speed=normal&device=gate')
  await expect.poll(() => calls.some((call) => call.startsWith('POST /api/device/controls/ptz/release'))).toBe(true)
  expect(calls.filter((call) => call.startsWith('POST /api/device/controls/ptz?'))).toEqual([])
})

test('leaving the page mid-hold sends the release as a beacon', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('ctl-move').click()
  const left = page.getByTestId('ptz-pad').getByRole('button', { name: 'Pan left', exact: true })
  const box = (await left.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await expect.poll(() => calls).toContain('POST /api/device/controls/ptz/press?action=left&speed=normal&device=gate')
  const beacons = await page.evaluate(() => {
    const sent: string[] = []
    navigator.sendBeacon = (url: string | URL) => {
      sent.push(String(url))
      return true
    }
    window.dispatchEvent(new Event('pagehide'))
    return sent
  })
  expect(beacons).toHaveLength(1)
  expect(beacons[0]).toMatch(/^\/api\/device\/controls\/ptz\/release\?pressed_ms=\d+&device=gate$/)
})

test('calibration is reachable from the pad without the preset editor', async ({ page }) => {
  const calls = await openLive(page, { extra: ['ptz calibrate'], drop: ['ptz preset edit'] })
  page.on('dialog', (dialog) => dialog.accept())
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ptz-edit')).toHaveCount(0)
  await page.getByTestId('ptz-calibrate').click()
  await expect(page.getByTestId('control-status')).toHaveText('Calibration finished in 1200 ms.')
  expect(calls).toContain('POST /api/device/controls/ptz/calibrate?device=gate')
})
