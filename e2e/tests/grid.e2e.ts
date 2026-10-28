import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { signIn } from './app.ts'

test('the grid shows every camera as a tile and the layout picks 1 to 3 columns', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'grid')
  await screen.getByTestId('grid-toggle').tap()
  const grid = screen.getByTestId('grid')
  await expect(grid).toBeVisible()
  const tiles = grid.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect(tiles.filter({ hasText: 'Shed' })).toHaveAttribute('data-state', 'offline')
  const layout = grid.getByTestId('grid-layout')
  for (const columns of ['1', '2', '3']) {
    await layout.getByRole('button', columns).tap()
    await expect(layout.getByRole('button', columns)).toHaveAttribute('aria-pressed', 'true')
    await expect
      .poll(() =>
        browser.evaluate(
          () => getComputedStyle(document.querySelector('[data-testid="grid-tiles"]')!).gridTemplateColumns.split(' ').length,
        ),
      )
      .toBe(Number(columns))
  }
  await tiles.filter({ hasText: 'Porch' }).getByRole('button', 'Open Porch in Live').tap()
  await expect(screen.getByTestId('camera-name')).toHaveText('Porch')
})
