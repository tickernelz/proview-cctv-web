import type { Locator } from '@playwright/test'
import { test, expect, type Page } from '@playwright/test'
import { playing, scenarioUrl, signIn } from './app'

const VIEWPORTS = [
  { width: 1440, height: 900 },
  { width: 1280, height: 800 },
]
const THEMES = ['dark', 'light'] as const

type Open = (page: Page) => Promise<Locator[]>

const SCREENS: Record<string, Open> = {
  signin: async (page) => {
    await page.goto(scenarioUrl('one'))
    await expect(page.getByTestId('signin')).toBeVisible()
    return []
  },
  live: async (page) => {
    await signIn(page, 'one')
    await playing(page)
    return [page.getByTestId('live-frame'), page.getByTestId('live-state')]
  },
  'devices-empty': async (page) => {
    await signIn(page, 'empty')
    await expect(page.getByTestId('devices-empty')).toBeVisible()
    return []
  },
  grid: async (page) => {
    await signIn(page, 'many')
    await page.getByTestId('grid-toggle').click()
    await expect(page.getByTestId('grid-tile')).toHaveCount(5)
    return [page.getByTestId('grid-tile-video'), page.getByTestId('grid-count')]
  },
  settings: async (page) => {
    await signIn(page, 'one')
    await page.getByTestId('open-settings').click()
    await expect(page.getByTestId('settings')).toBeVisible()
    await expect(page.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
    return []
  },
  alarms: async (page) => {
    await page.route(/\/api\/cloud\/alarms\?/, (route) =>
      route.fulfill({
        json: {
          serial: 'gate',
          total: 2,
          finished: true,
          pollSeconds: 60,
          entries: [0, 1].map((index) => ({
            id: 'visual' + index,
            name: 'human detect',
            time: '2026-10-27 21:0' + index + ':10',
            utcTime: 1793112000 + index,
            channel: '0',
          })),
        },
      }),
    )
    await signIn(page, 'one')
    await page.getByTestId('alarm-bell').click()
    await expect(page.getByTestId('alarm-row')).toHaveCount(2)
    return [page.getByTestId('live-frame'), page.getByTestId('live-state')]
  },
}

for (const [name, open] of Object.entries(SCREENS)) {
  for (const viewport of VIEWPORTS) {
    for (const theme of THEMES) {
      const shot = name + '-' + viewport.width + 'x' + viewport.height + '-' + theme
      test(shot, async ({ page }) => {
        await page.setViewportSize(viewport)
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' })
        const mask = await open(page)
        expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe(theme)
        await page.evaluate(() => document.fonts.ready.then(() => true))
        await expect(page).toHaveScreenshot(shot + '.png', {
          mask,
        })
      })
    }
  }
}
