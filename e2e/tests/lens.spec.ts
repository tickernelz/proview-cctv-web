import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

async function frameBoxes(page: Page) {
  return page.evaluate(() => {
    const frame = document.getElementById('lens-crop')!.getBoundingClientRect()
    const video = document.getElementById('video')!.getBoundingClientRect()
    return { frame: { w: frame.width, h: frame.height }, video: { dy: video.top - frame.top, h: video.height, w: video.width } }
  })
}

test('one lens fills the frame at its own shape: the other half of the stacked picture sits outside it', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  const both = await frameBoxes(page)
  expect(both.video.dy).toBe(0)
  expect(Math.abs(both.frame.w / both.frame.h - 640 / 720)).toBeLessThan(0.02)
  for (const [lens, shift] of [['Bottom lens', -1], ['Top lens', 0]] as const) {
    await page.getByTestId('lens-pick').click()
    await page.getByTestId('lens-menu').getByTestId('lens-option').filter({ hasText: lens }).click()
    await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', lens === 'Top lens' ? '0' : '1')
    const one = await frameBoxes(page)
    expect(Math.abs(one.frame.w / one.frame.h - 640 / 360)).toBeLessThan(0.02)
    expect(Math.abs(one.video.h - 2 * one.frame.h)).toBeLessThan(2)
    expect(Math.abs(one.video.dy - shift * one.frame.h)).toBeLessThan(2)
  }
})

test('the lens menu switches from both lenses to the top lens and to the bottom lens', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('lens-label')).toHaveText('Both lenses')
  const menu = page.getByTestId('lens-menu')
  for (const lens of ['Top lens', 'Bottom lens']) {
    await page.getByTestId('lens-pick').click()
    await expect(menu.getByTestId('lens-option')).toHaveCount(3)
    await menu.getByTestId('lens-option').filter({ hasText: lens }).click()
    await expect(menu).toBeHidden()
    await expect(page.getByTestId('lens-label')).toHaveText(lens)
    await expect(page.getByTestId('live-frame')).toBeVisible()
    await page.getByTestId('lens-pick').click()
    await expect(menu.getByTestId('lens-option').filter({ hasText: lens })).toHaveAttribute('aria-checked', 'true')
    await page.keyboard.press('Escape')
    await expect(menu).toBeHidden()
  }
})
