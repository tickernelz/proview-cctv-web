import { test, expect } from '@playwright/test'
import { eventsPanelClosed, signIn } from './app'

test.beforeEach(({ page }) => eventsPanelClosed(page))

test('the grid shows every camera as a tile and the layout picks 1 to 3 columns', async ({ page }) => {
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  const grid = page.getByTestId('grid')
  await expect(grid).toBeVisible()
  const tiles = grid.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect(tiles.filter({ hasText: 'Shed' })).toHaveAttribute('data-state', 'offline')
  const layout = grid.getByTestId('grid-layout')
  await layout.getByRole('button', { name: 'Auto', exact: true }).click()
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

test('on a phone each tile keeps its name and stream under its own picture, whatever layout was saved', async ({ page }) => {
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  await page.getByTestId('grid-layout').getByRole('button', { name: '3', exact: true }).click()
  await expect(page.getByTestId('grid-note')).toHaveText(/saved at/)
  await page.setViewportSize({ width: 390, height: 844 })
  const tiles = page.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect
    .poll(() => page.getByTestId('grid-tiles').evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(' ').length))
    .toBe(1)
  const boxes = await tiles.evaluateAll((nodes) =>
    nodes.map((tile) => {
      const well = tile.querySelector('.well')!.getBoundingClientRect()
      const meta = tile.querySelector('.tile-meta')!.getBoundingClientRect()
      return { wellBottom: well.bottom, wellHeight: well.height, wellWidth: well.width, metaTop: meta.top, metaBottom: meta.bottom, top: tile.getBoundingClientRect().top }
    }),
  )
  boxes.forEach((box, index) => {
    expect(box.metaTop).toBeGreaterThanOrEqual(box.wellBottom - 0.5)
    expect(Math.abs(box.wellHeight - (box.wellWidth * 9) / 16)).toBeLessThan(2)
    if (index + 1 < boxes.length) expect(boxes[index + 1].top).toBeGreaterThanOrEqual(box.metaBottom - 0.5)
  })
})

test('a tile that cannot play says why, centred, and Try again plays it', async ({ page }) => {
  let fail = true
  let asked = 0
  await page.route(/\/api\/stream\?device=porch/, (route) => {
    asked += 1
    return fail ? route.fulfill({ status: 502, json: { message: 'the camera did not answer the stream request in 8 s' } }) : route.continue()
  })
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  const porch = page.getByTestId('grid-tile').filter({ hasText: 'Porch' })
  await expect(porch).toHaveAttribute('data-state', 'error')
  const failBox = porch.getByTestId('grid-tile-fail')
  await expect(failBox).toContainText('No picture')
  await expect(failBox).toHaveCSS('text-align', 'center')
  const retry = porch.getByTestId('grid-tile-retry')
  await expect(retry).toHaveClass(/btn/)
  fail = false
  await retry.click()
  await expect(porch).toHaveAttribute('data-state', 'playing', { timeout: 20_000 })
  expect(asked).toBe(2)
})

test('a connecting tile says so inside the picture, and an offline tile says offline once in its caption', async ({ page }) => {
  await page.route(/\/api\/stream\?/, () => new Promise(() => {}))
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  const gate = page.getByTestId('grid-tile').filter({ hasText: 'Gate camera' })
  await expect(gate.getByTestId('grid-tile-loading')).toHaveText('Connecting…')
  const shed = page.getByTestId('grid-tile').filter({ hasText: 'Shed' })
  await expect(shed).toHaveAttribute('data-state', 'offline')
  await expect(shed.locator('.tile-meta')).toHaveText(/^Shed\s*offline\s*SD$/)
  await expect(page.getByTestId('grid-count')).toHaveText('5 cameras · 4 connecting · 1 offline')
})

test('a layout the server refuses to save is an alert, and the next save clears it', async ({ page }) => {
  let refuse = true
  await page.route(/\/api\/grid$/, (route) =>
    route.request().method() === 'POST' && refuse ? route.fulfill({ status: 500, json: { message: 'disk full' } }) : route.continue(),
  )
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  const layout = page.getByTestId('grid-layout')
  await layout.getByRole('button', { name: '2', exact: true }).click()
  const note = page.getByTestId('grid-note')
  await expect(note).toHaveText('The grid was not saved: disk full')
  await expect(note).toHaveAttribute('role', 'alert')
  await expect(note.locator('svg')).toHaveCount(1)
  refuse = false
  await layout.getByRole('button', { name: '3', exact: true }).click()
  await expect(note).toHaveText(/^Layout and streams saved at \d\d:\d\d$/)
  await expect(note).toHaveAttribute('role', 'status')
})

test('arrow keys move focus between tiles and the focused tile shows a ring', async ({ page }) => {
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  await expect(page.getByTestId('grid-tile')).toHaveCount(5)
  const open = page.locator('#tiles .tile-open')
  await open.first().focus()
  await page.keyboard.press('ArrowRight')
  await expect(open.nth(1)).toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(open.nth(4)).toBeFocused()
  await expect(open.nth(4)).toHaveCSS('outline-style', 'solid')
})

test('the grid rings the camera that was on screen, also after opening another tile and coming back', async ({ page }) => {
  await signIn(page, 'grid')
  await expect(page.getByTestId('camera-name')).not.toHaveText('No camera')
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await page.getByTestId('view-grid').click()
  const tiles = page.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect(page.locator('#tiles .tile.sel')).toHaveCount(1)
  await expect(tiles.filter({ hasText: 'Porch' })).toHaveClass(/\bsel\b/)
  await tiles.filter({ hasText: 'Garage' }).getByRole('button', { name: 'Open Garage in Live', exact: true }).click()
  await expect(page.getByTestId('camera-name')).toHaveText('Garage')
  await page.getByTestId('view-grid').click()
  await expect(tiles.filter({ hasText: 'Garage' })).toHaveClass(/\bsel\b/)
  await expect(page.locator('#tiles .tile.sel')).toHaveCount(1)
})

test('on a phone two tiles get rows the height of their picture, with no gap under the first', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await signIn(page, 'grid')
  await page.getByTestId('view-grid').click()
  await expect(page.getByTestId('grid-tile')).toHaveCount(5)
  await page.getByTestId('grid-tiles').evaluate((node) => node.setAttribute('data-count', '2'))
  const rows = await page.getByTestId('grid-tiles').evaluate((node) => getComputedStyle(node).gridTemplateRows.split(' ').map(parseFloat))
  expect(Math.max(...rows) - Math.min(...rows)).toBeLessThan(2)
})
