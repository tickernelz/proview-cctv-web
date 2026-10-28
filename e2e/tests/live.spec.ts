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
