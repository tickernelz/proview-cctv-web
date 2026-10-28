import { test, expect, type Page } from '@playwright/test'
import { serverFor0, signIn, playing } from './app'

const VIEWPORTS = [
  { width: 320, height: 568 },
  { width: 568, height: 320 },
  { width: 360, height: 640 },
  { width: 640, height: 360 },
  { width: 390, height: 844 },
  { width: 844, height: 390 },
  { width: 768, height: 1024 },
  { width: 1024, height: 768 },
  { width: 1280, height: 800 },
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 },
]

const SCREENS: Record<string, (page: Page) => Promise<void>> = {
  signin: async (page) => {
    await page.goto(serverFor0('one'))
    await expect(page.getByTestId('signin')).toBeVisible()
  },
  live: async (page) => {
    await signIn(page, 'one')
    await playing(page)
  },
  devices: async (page) => {
    await signIn(page, 'many')
    await expect(page.getByTestId('live')).toBeVisible()
  },
  grid: async (page) => {
    await signIn(page, 'many')
    await page.getByTestId('grid-toggle').click()
    await expect(page.getByTestId('grid-tile')).toHaveCount(5)
  },
  settings: async (page) => {
    await signIn(page, 'one')
    await page.getByTestId('open-settings').click()
    await expect(page.getByTestId('settings')).toBeVisible()
  },
}

async function overflow(page: Page) {
  return page.evaluate(() => {
    const root = document.scrollingElement as HTMLElement
    const width = root.clientWidth
    const wide = [...document.querySelectorAll<HTMLElement>('body *')]
      .filter((node) => {
        if (!node.offsetParent && getComputedStyle(node).position !== 'fixed') return false
        const box = node.getBoundingClientRect()
        return box.width > 0 && (box.right > width + 1 || box.left < -1)
      })
      .slice(0, 5)
      .map((node) => (node.dataset.testid || node.id || node.className || node.tagName) + '@' + Math.round(node.getBoundingClientRect().right))
    return { scroll: root.scrollWidth, client: width, wide }
  })
}

for (const [name, open] of Object.entries(SCREENS)) {
  for (const viewport of VIEWPORTS) {
    test(`${name} has no horizontal scroll at ${viewport.width}x${viewport.height}`, async ({ page }) => {
      await page.setViewportSize(viewport)
      await open(page)
      const seen = await overflow(page)
      expect(seen.scroll, JSON.stringify(seen)).toBeLessThanOrEqual(seen.client)
      expect(seen.wide, JSON.stringify(seen)).toEqual([])
    })
  }
}

for (const [width, columns] of [
  [390, 1],
  [800, 2],
  [1280, 4],
] as const) {
  test(`the auto grid uses ${columns} column(s) at ${width} px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    await signIn(page, 'many')
    await page.getByTestId('grid-toggle').click()
    await expect(page.getByTestId('grid-tile')).toHaveCount(5)
    await expect
      .poll(() => page.getByTestId('grid-tiles').evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(' ').length))
      .toBe(columns)
  })
}
