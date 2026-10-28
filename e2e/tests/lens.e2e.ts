import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { playing, signIn } from './app.ts'

test('the lens menu switches from both lenses to the top lens and to the bottom lens', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'one')
  await playing(screen)
  await expect(screen.getByTestId('lens-label')).toHaveText('Both lenses')
  const menu = screen.getByTestId('lens-menu')
  for (const lens of ['Top lens', 'Bottom lens']) {
    await screen.getByTestId('lens-pick').tap()
    await expect(menu.getByTestId('lens-option')).toHaveCount(3)
    await menu.getByTestId('lens-option').filter({ hasText: lens }).tap()
    await expect(menu).toBeHidden()
    await expect(screen.getByTestId('lens-label')).toHaveText(lens)
    await expect(screen.getByTestId('live-frame')).toBeVisible()
    await screen.getByTestId('lens-pick').tap()
    await expect(menu.getByTestId('lens-option').filter({ hasText: lens })).toHaveAttribute('aria-checked', 'true')
    await browser.keyboard.press('Escape')
    await expect(menu).toBeHidden()
  }
})
