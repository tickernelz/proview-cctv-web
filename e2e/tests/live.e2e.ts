import { readFile } from 'node:fs/promises'
import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { playing, signIn } from './app.ts'

test('the live tile shows a decoded frame from the camera', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'one')
  await playing(screen)
  await expect(screen.getByTestId('live-video')).toBeVisible()
  const size = await browser.evaluate(() => {
    const video = document.querySelector('[data-testid="live-video"]') as HTMLVideoElement
    return [video.videoWidth, video.videoHeight]
  })
  expect(size[0]).toBeGreaterThan(0)
  expect(size[1]).toBeGreaterThan(0)
})

test('Snapshot downloads a PNG of the live frame', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'one')
  await playing(screen)
  const file = await browser.waitForDownload(() => screen.getByTestId('ctl-snapshot').tap())
  expect(file.suggestedFilename).toMatch(/\.png$/)
  const bytes = await readFile(file.absolutePath)
  expect(bytes.subarray(1, 4).toString()).toBe('PNG')
})

test('Full screen puts the player in fullscreen and leaving it returns to Live', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'one')
  await playing(screen)
  await screen.getByTestId('ctl-fullscreen').tap()
  await expect.poll(() => browser.evaluate(() => document.fullscreenElement !== null)).toBe(true)
  await browser.evaluate(() => document.exitFullscreen().then(() => true))
  await expect.poll(() => browser.evaluate(() => document.fullscreenElement !== null)).toBe(false)
  await expect(screen.getByTestId('live')).toBeVisible()
})
