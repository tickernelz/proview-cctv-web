import { test, expect, type Page, type Locator } from '@playwright/test'
import { playing, scenarioUrl, signIn } from './app'
import { overflowFindings, smallTargets, walkScreens } from './layout'

const SCREENS: Record<string, (page: Page) => Promise<Locator[]>> = {
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
  devices: async (page) => {
    await signIn(page, 'many')
    await playing(page)
    await page.getByTestId('camera-switcher').click()
    await expect(page.getByTestId('camera-option')).toHaveCount(5)
    return [page.getByTestId('live-frame'), page.getByTestId('live-state')]
  },
  settings: async (page) => {
    await signIn(page, 'one')
    await page.getByTestId('open-settings').click()
    await expect(page.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
    return []
  },
}

test('a coarse pointer gets 44 px targets and no overflow on every screen and overlay', async ({ page }) => {
  test.setTimeout(120_000)
  expect(await page.evaluate(() => matchMedia('(pointer: coarse)').matches)).toBe(true)
  const findings: string[] = []
  await walkScreens(page, async (state) => {
    const found = [...(await smallTargets(page)).map((line) => 'small ' + line), ...(await overflowFindings(page))]
    console.log(`RESULT ${test.info().project.name} | ${state} | ${found.length ? 'fail: ' + found.join('; ') : 'pass'}`)
    findings.push(...found.map((line) => state + ': ' + line))
  })
  expect(findings).toEqual([])
})

for (const [name, open] of Object.entries(SCREENS)) {
  for (const theme of ['dark', 'light'] as const) {
    test(`${name} on a touch screen in the ${theme} theme`, async ({ page }, info) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' })
      const mask = await open(page)
      expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe(theme)
      const root = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth])
      expect(root[0]).toBeLessThanOrEqual(root[1])
      await page.evaluate(() => document.fonts.ready.then(() => true))
      await expect(page).toHaveScreenshot(`${name}-${info.project.name}-${theme}.png`, {
        mask,
      })
    })
  }
}
