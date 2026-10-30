import { readFile } from 'node:fs/promises'
import { test, expect } from '@playwright/test'
import { playing, signIn } from './app'

test('the live tile shows a decoded frame from the camera', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-video')).toBeVisible()
  const size = await page.evaluate(() => {
    const video = document.querySelector('[data-testid="live-video"]') as HTMLVideoElement
    return [video.videoWidth, video.videoHeight]
  })
  expect(size[0]).toBeGreaterThan(0)
  expect(size[1]).toBeGreaterThan(0)
})

test('Snapshot downloads a PNG of the live frame', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  const file = await Promise.all([page.waitForEvent('download'), page.getByTestId('ctl-snapshot').click()]).then(([d]) => d)
  expect(file.suggestedFilename()).toMatch(/\.png$/)
  const bytes = await readFile((await file.path())!)
  expect(bytes.subarray(1, 4).toString()).toBe('PNG')
})

test('Full screen puts the player in fullscreen and leaving it returns to Live', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('ctl-fullscreen').click()
  await expect.poll(() => page.evaluate(() => document.fullscreenElement !== null)).toBe(true)
  await page.evaluate(() => document.exitFullscreen().then(() => true))
  await expect.poll(() => page.evaluate(() => document.fullscreenElement !== null)).toBe(false)
  await expect(page.getByTestId('live')).toBeVisible()
})

test('the status plate on the picture says LIVE with the clock and the quality', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  const plate = page.getByTestId('live-plate')
  await expect(plate).toBeVisible()
  await expect(plate).toContainText('LIVE')
  await expect(plate.getByTestId('live-clock')).toHaveText(/^\d{2}:\d{2}:\d{2}$/)
  await expect(plate).toContainText('HD')
  await page.keyboard.press('q')
  await expect(plate).toContainText('SD')
  await expect(page.getByTestId('quality-value')).toHaveText('SD')
  await playing(page)
})

test('the dock keeps the main actions labelled and More turns the picture over', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  const dock = page.getByTestId('live-controls')
  for (const label of ['Talk', 'Sound', 'Snapshot', 'Record', 'Move', 'Lens', 'Quality', 'More', 'Full screen']) {
    await expect(dock.getByRole('button', { name: label, exact: false }).first()).toBeVisible()
  }
  await page.getByTestId('ctl-more').click()
  const menu = page.getByTestId('more-menu')
  await expect(menu).toBeVisible()
  await expect(page.getByTestId('more-flip')).toContainText('Off')
  await page.getByTestId('more-flip').click()
  await expect(menu).toBeHidden()
  await expect(page.getByTestId('ctl-more')).toBeFocused()
  await expect(page.getByTestId('live-frame')).toHaveAttribute('data-flipped', 'true')
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('pvw_prefs_v2') || '{}').flip)).toBe(true)
  await page.getByTestId('ctl-more').click()
  await page.getByTestId('more-camera-settings').click()
  await expect(page.getByTestId('settings')).toBeVisible()
})

test('Snapshot says where the picture went', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await Promise.all([page.waitForEvent('download'), page.getByTestId('ctl-snapshot').click()])
  await expect(page.getByText('Snapshot saved')).toBeVisible()
})

