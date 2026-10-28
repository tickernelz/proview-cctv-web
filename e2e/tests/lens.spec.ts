import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

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
