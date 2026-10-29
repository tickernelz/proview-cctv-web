import { test, expect } from '@playwright/test'
import { openLive } from './app'

test('Move opens the pan and tilt pad and each arrow and preset reaches the device', async ({ page }) => {
  const calls = await openLive(page)
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ctl-move')).toHaveAttribute('aria-pressed', 'true')
  const pad = page.getByTestId('ptz-pad')
  await expect(pad).toBeVisible()
  await pad.getByRole('button', { name: 'Pan left', exact: true }).click()
  await expect(page.getByTestId('control-status')).toHaveText('Moved the bottom lens over dvrip for 600 ms.')
  expect(calls).toContain('POST /api/device/controls/ptz?lens=bottom&pan=-0.5&tilt=0&hold_ms=600&device=gate')
  await expect(pad.getByTestId('ptz-preset')).toHaveCount(2)
  await pad.getByTestId('ptz-preset').filter({ hasText: 'Driveway' }).click()
  await expect(page.getByTestId('control-status')).toHaveText('At Driveway.')
  expect(calls).toContain('POST /api/device/controls/preset/goto?token=2&device=gate')
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
  expect(calls).toContain('POST /api/device/controls/light?mode=always&device=gate')
  await settings.getByRole('link', { name: 'Playback', exact: true }).click()
  await page.getByTestId('set-flip').check()
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect(page.getByTestId('live-frame')).toHaveAttribute('data-flipped', 'true')
})
