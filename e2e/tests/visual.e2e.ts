import { test } from '@e2e-dev/web'
import type { Browser } from '@e2e-dev/web'
import type { App, Locator, Screen } from 'e2e'
import { expect } from 'e2e'
import { playing, signIn } from './app.ts'

const VIEWPORTS = [
  { width: 1440, height: 900 },
  { width: 1280, height: 800 },
]
const THEMES = ['dark', 'light'] as const

type Open = (app: App, screen: Screen, browser: Browser) => Promise<Locator[]>

const SCREENS: Record<string, Open> = {
  signin: async (app, screen) => {
    await app.open('/')
    await expect(screen.getByTestId('signin')).toBeVisible()
    return []
  },
  live: async (app, screen, browser) => {
    await signIn(app, browser, 'one')
    await playing(screen)
    return [screen.getByTestId('live-frame'), screen.getByTestId('live-state')]
  },
  'devices-empty': async (app, screen, browser) => {
    await signIn(app, browser, 'empty')
    await expect(screen.getByTestId('devices-empty')).toBeVisible()
    return []
  },
  grid: async (app, screen, browser) => {
    await signIn(app, browser, 'many')
    await screen.getByTestId('grid-toggle').tap()
    await expect(screen.getByTestId('grid-tile')).toHaveCount(5)
    return [screen.getByTestId('grid-tile-video'), screen.getByTestId('grid-tile-clock'), screen.getByTestId('grid-count')]
  },
  settings: async (app, screen, browser) => {
    await signIn(app, browser, 'one')
    await screen.getByTestId('open-settings').tap()
    await expect(screen.getByTestId('settings')).toBeVisible()
    await expect(screen.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
    return []
  },
  alarms: async (app, screen, browser) => {
    await browser.route(/\/api\/cloud\/alarms\?/, (route) =>
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
    await signIn(app, browser, 'one')
    await screen.getByTestId('alarm-bell').tap()
    await expect(screen.getByTestId('alarm-row')).toHaveCount(2)
    return [screen.getByTestId('live-frame'), screen.getByTestId('live-state'), screen.getByTestId('alarm-status')]
  },
}

for (const [name, open] of Object.entries(SCREENS)) {
  for (const viewport of VIEWPORTS) {
    for (const theme of THEMES) {
      const shot = name + '-' + viewport.width + 'x' + viewport.height + '-' + theme
      test(shot, { tags: ['visual'] }, async ({ app, screen, browser }) => {
        await browser.setViewport(viewport)
        await browser.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' })
        const mask = await open(app, screen, browser)
        expect(await browser.evaluate(() => document.documentElement.dataset.theme)).toBe(theme)
        await browser.evaluate(() => document.fonts.ready.then(() => true))
        await expect(screen).toHaveScreenshot(shot, {
          maxDiffPixelRatio: 0.002,
          mask: [...mask, screen.getByTestId('live-clock')],
        })
      })
    }
  }
}
