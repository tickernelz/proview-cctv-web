import { test, expect } from '@playwright/test'
import { eventsPanelClosed, signIn } from './app'
import { overflowFindings, walkScreens } from './layout'

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

for (const viewport of VIEWPORTS) {
  test(`every screen and overlay stays inside ${viewport.width}x${viewport.height}`, async ({ page }) => {
    test.setTimeout(120_000)
    await page.setViewportSize(viewport)
    const findings: string[] = []
    await walkScreens(page, async (state) => {
      const found = await overflowFindings(page)
      console.log(`RESULT ${test.info().project.name} ${viewport.width}x${viewport.height} | ${state} | ${found.length ? 'fail: ' + found.join('; ') : 'pass'}`)
      findings.push(...found.map((line) => state + ': ' + line))
    })
    expect(findings).toEqual([])
  })
}

for (const [width, columns] of [
  [390, 1],
  [800, 2],
  [1280, 4],
] as const) {
  test(`the auto grid uses ${columns} column(s) at ${width} px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    await eventsPanelClosed(page)
    await signIn(page, 'many')
    await page.getByTestId('view-grid').click()
    await expect(page.getByTestId('grid-tile')).toHaveCount(5)
    await expect
      .poll(() => page.getByTestId('grid-tiles').evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(' ').length))
      .toBe(columns)
  })
}
