import { test, expect } from '@playwright/test'
import { signIn } from './app'

test('the grid shows every camera as a tile and the layout picks 1 to 3 columns', async ({ page }) => {
  await signIn(page, 'grid')
  await page.getByTestId('grid-toggle').click()
  const grid = page.getByTestId('grid')
  await expect(grid).toBeVisible()
  const tiles = grid.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect(tiles.filter({ hasText: 'Shed' })).toHaveAttribute('data-state', 'offline')
  const layout = grid.getByTestId('grid-layout')
  await expect(layout.getByRole('button', { name: 'Auto', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(grid.getByTestId('grid-tiles')).toBeVisible()
  await expect
    .poll(() =>
      page.evaluate(() => getComputedStyle(document.querySelector('[data-testid="grid-tiles"]')!).gridTemplateColumns.split(' ').length),
    )
    .toBe(4)
  for (const columns of ['1', '2', '3']) {
    await layout.getByRole('button', { name: columns, exact: true }).click()
    await expect(layout.getByRole('button', { name: columns, exact: true })).toHaveAttribute('aria-pressed', 'true')
    await expect
      .poll(() =>
        page.evaluate(
          () => getComputedStyle(document.querySelector('[data-testid="grid-tiles"]')!).gridTemplateColumns.split(' ').length,
        ),
      )
      .toBe(Number(columns))
  }
  await tiles.filter({ hasText: 'Porch' }).getByRole('button', { name: 'Open Porch in Live', exact: true }).click()
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
})
