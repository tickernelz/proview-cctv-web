import { readFile } from 'node:fs/promises'
import { test, expect } from '@playwright/test'
import { eventsPanelClosed, playing, signIn } from './app'

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

test('on a phone the dock is one row and More carries Record, Quality, Full screen, Recordings and cameras', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await signIn(page, 'one')
  await playing(page)
  const dock = page.getByTestId('live-controls')
  const shown = dock.locator(':scope > button:visible')
  await expect(shown).toHaveText([/Talk/, /Sound/, /Snapshot/, /Move/, /More/])
  expect(await shown.evaluateAll((nodes) => new Set(nodes.map((node) => Math.round(node.getBoundingClientRect().top))).size)).toBe(1)
  await page.getByTestId('ctl-more').click()
  const menu = page.getByTestId('more-menu')
  for (const id of ['more-record', 'more-quality', 'more-fullscreen', 'more-recordings', 'more-cloud', 'more-manage-cameras']) {
    await expect(menu.getByTestId(id)).toBeVisible()
  }
  const option = (label: string) => menu.getByTestId('more-quality-option').filter({ hasText: label })
  await expect(option('HD')).toHaveAttribute('aria-checked', 'true')
  const stream = page.waitForRequest((request) => request.url().includes('/api/stream?') && request.url().includes('quality=sub'), { timeout: 10_000 })
  await option('SD').click()
  await stream
  await expect(menu).toBeHidden()
  await expect(page.getByTestId('ctl-more')).toBeFocused()
  await playing(page)
  await page.getByTestId('ctl-more').click()
  await expect(option('SD')).toHaveAttribute('aria-checked', 'true')
  await menu.getByTestId('more-recordings').click()
  await expect(page.getByTestId('playback')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('playback')).toBeHidden()
  await expect(page.getByTestId('ctl-more')).toBeFocused()
})

for (const size of [
  { width: 1024, height: 768 },
  { width: 1440, height: 900 },
]) {
  test(`Devices, Recordings and the Events overlay leave the whole dock reachable at ${size.width}x${size.height}`, async ({ page }) => {
    await page.setViewportSize(size)
    await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: { ok: true, groups: [], devices: [] } }))
    await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { entries: [], total: 0, pollSeconds: 60 } }))
    await eventsPanelClosed(page)
    await signIn(page, 'one')
    await playing(page)
    const dock = page.getByTestId('live-controls')
    const reachable = () =>
      dock.evaluate((node) =>
        [...node.querySelectorAll(':scope > button, :scope > [aria-haspopup]')]
          .filter((button) => button.getClientRects().length)
          .flatMap((button) => {
            const box = button.getBoundingClientRect()
            const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2)
            const name = button.id + (box.right > innerWidth ? ' cut at the edge' : '')
            return button.contains(hit) && box.right <= innerWidth ? [] : [name]
          }),
      )
    const sheets: [string, () => Promise<unknown>][] = [
      ['device-admin', async () => {
        await page.getByTestId('ctl-more').click()
        await page.getByTestId('more-manage-cameras').click()
      }],
      ['recordings-panel', () => page.getByTestId('tl-recordings').click()],
      ['alarm-center', () => page.getByTestId('alarm-bell').click()],
    ]
    for (const [id, open] of sheets) {
      if (id === 'alarm-center' && size.width >= 1280) continue
      await open()
      await expect(page.getByTestId(id)).toBeVisible()
      expect(await reachable(), id).toEqual([])
      await page.getByTestId('ctl-more').click()
      await expect(page.getByTestId('more-menu')).toBeVisible()
      await page.keyboard.press('Escape')
      await expect(page.getByTestId('more-menu')).toBeHidden()
      if (id === 'alarm-center') await page.getByTestId('alarm-bell').click()
      else await page.keyboard.press('Escape')
      await expect(page.getByTestId(id)).toBeHidden()
    }
  })
}

test('in a window too short for 240 px of sheet above the dock, Devices falls back to the plain dock offset', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 340 })
  await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: { ok: true, groups: [], devices: [] } }))
  await eventsPanelClosed(page)
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('ctl-more').click()
  await page.getByTestId('more-manage-cameras').click()
  const sheet = page.getByTestId('device-admin')
  await expect(sheet).toBeVisible()
  const layout = await page.evaluate(() => {
    const root = getComputedStyle(document.documentElement)
    const sheet = document.getElementById('device-admin')!
    return {
      above: innerHeight - document.querySelector('[data-testid="app-header"]')!.getBoundingClientRect().bottom - (innerHeight - document.getElementById('controls')!.getBoundingClientRect().top),
      withoutHeader: document.getElementById('controls')!.getBoundingClientRect().top,
      clear: document.documentElement.style.getPropertyValue('--dock-clear'),
      bottom: getComputedStyle(sheet).bottom,
      fallback: parseFloat(root.getPropertyValue('--dock')) + parseFloat(root.getPropertyValue('--s4')) + 'px',
    }
  })
  expect(layout.above).toBeLessThan(240)
  expect(layout.withoutHeader).toBeGreaterThanOrEqual(240)
  expect(layout.clear).toBe('')
  expect(layout.bottom).toBe(layout.fallback)
})

test('Snapshot says where the picture went', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await Promise.all([page.waitForEvent('download'), page.getByTestId('ctl-snapshot').click()])
  await expect(page.getByText('Snapshot saved')).toBeVisible()
})

