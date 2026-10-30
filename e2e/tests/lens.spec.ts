import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

async function frameBoxes(page: Page) {
  return page.evaluate(() => {
    const frame = document.getElementById('lens-crop')!.getBoundingClientRect()
    const video = document.getElementById('video')!.getBoundingClientRect()
    return { frame: { w: frame.width, h: frame.height }, video: { dy: video.top - frame.top, h: video.height, w: video.width } }
  })
}

async function pickLens(page: Page, lens: string) {
  await page.getByTestId('lens-pick').click()
  await page.getByTestId('lens-menu').getByTestId('lens-option').filter({ hasText: lens }).click()
  await expect(page.getByTestId('lens-menu')).toBeHidden()
}

test('picture in picture is the default: the top lens fills the frame and the bottom lens sits in the corner', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'pip')
  const main = await frameBoxes(page)
  expect(Math.abs(main.frame.w / main.frame.h - 640 / 360)).toBeLessThan(0.02)
  expect(Math.abs(main.video.h - 2 * main.frame.h)).toBeLessThan(2)
  expect(Math.abs(main.video.dy)).toBeLessThan(2)
  const inset = page.getByTestId('pip-inset')
  await expect(inset).toBeVisible()
  await expect(inset).toHaveAttribute('aria-label', 'Swap lenses: show the bottom lens large')
  const box = (await inset.boundingBox())!
  expect(Math.abs(box.width / box.height - 640 / 360)).toBeLessThan(0.05)
  expect(box.width).toBeLessThan(main.frame.w / 2)
  const painted = await page.waitForFunction(() => {
    const canvas = document.querySelector('[data-testid="pip-inset"] canvas') as HTMLCanvasElement | null
    if (!canvas || !canvas.width) return false
    const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data
    for (let index = 0; index < pixels.length; index += 4) if (pixels[index] + pixels[index + 1] + pixels[index + 2] > 30) return true
    return false
  })
  expect(await painted.jsonValue()).toBe(true)
  await inset.click()
  await expect(inset).toHaveAttribute('aria-label', 'Swap lenses: show the top lens large')
  const swapped = await frameBoxes(page)
  expect(Math.abs(swapped.video.dy + swapped.frame.h)).toBeLessThan(2)
  await inset.click()
  await expect(inset).toHaveAttribute('aria-label', 'Swap lenses: show the bottom lens large')
})

test('one lens fills the frame at its own shape: the other half of the stacked picture sits outside it', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await pickLens(page, 'Both lenses')
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
  await expect(page.getByTestId('pip-inset')).toHaveCount(0)
  const both = await frameBoxes(page)
  expect(both.video.dy).toBe(0)
  expect(Math.abs(both.frame.w / both.frame.h - 640 / 720)).toBeLessThan(0.02)
  for (const [lens, shift] of [['Bottom lens', -1], ['Top lens', 0]] as const) {
    await pickLens(page, lens)
    await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', lens === 'Top lens' ? '0' : '1')
    const one = await frameBoxes(page)
    expect(Math.abs(one.frame.w / one.frame.h - 640 / 360)).toBeLessThan(0.02)
    expect(Math.abs(one.video.h - 2 * one.frame.h)).toBeLessThan(2)
    expect(Math.abs(one.video.dy - shift * one.frame.h)).toBeLessThan(2)
  }
})

test('the lens menu offers picture in picture, both lenses, the top lens and the bottom lens', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  const menu = page.getByTestId('lens-menu')
  await page.getByTestId('lens-pick').click()
  await expect(menu.getByTestId('lens-option')).toHaveText([/Picture in picture/, /Both lenses/, /Top lens/, /Bottom lens/])
  await expect(menu.getByTestId('lens-option').filter({ hasText: 'Picture in picture' })).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.press('Escape')
  for (const lens of ['Top lens', 'Bottom lens', 'Picture in picture']) {
    await pickLens(page, lens)
    await expect(page.getByTestId('live-frame')).toBeVisible()
    await page.getByTestId('lens-pick').click()
    await expect(menu.getByTestId('lens-option').filter({ hasText: lens })).toHaveAttribute('aria-checked', 'true')
    await page.keyboard.press('Escape')
    await expect(menu).toBeHidden()
  }
  await page.keyboard.press('l')
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
})

test('More holds the lens modes and the swap, and a saved choice of both lenses still loads', async ({ page }) => {
  await page.addInitScript(() => {
    if (!sessionStorage.getItem('seeded')) {
      sessionStorage.setItem('seeded', '1')
      localStorage.setItem('pvw_prefs_v2', JSON.stringify({ lens: 'combined' }))
    }
  })
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
  await page.getByTestId('ctl-more').click()
  await expect(page.getByTestId('more-swap')).toHaveCount(0)
  await page.getByTestId('more-lens').click()
  await page.getByTestId('more-lens-menu').getByTestId('more-lens-option').filter({ hasText: 'Picture in picture' }).click()
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'pip')
  await page.getByTestId('ctl-more').click()
  await page.getByTestId('more-swap').click()
  await expect(page.getByTestId('pip-label')).toHaveText('Top lens')
})
