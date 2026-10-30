import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

const CLIPS = {
  recordings: [
    { start: '2026-10-27 08:00:00', duration_secs: 95, size_bytes: 3_250_000 },
    { start: '2026-10-27 09:30:12', duration_secs: 40, size_bytes: 900_000 },
  ],
}

async function openPanel(page: Page) {
  await page.getByTestId('recordings-launcher').click()
  const panel = page.getByTestId('recordings-panel')
  await expect(panel).toBeVisible()
  await expect(page.getByTestId('recordings-camera')).toBeEnabled()
  return panel
}

test('Cloud opens on the camera on screen, takes focus, and Escape or Close hands focus back', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  const launcher = page.getByTestId('recordings-launcher')
  const panel = await openPanel(page)
  await expect(launcher).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('recordings-day')).toBeFocused()
  await expect(page.getByTestId('recordings-camera')).toHaveValue('porch')
  await page.keyboard.press('Escape')
  await expect(panel).toBeHidden()
  await expect(launcher).toBeFocused()
  await expect(launcher).toHaveAttribute('aria-pressed', 'false')
  await expect(page.getByTestId('live-well')).toBeVisible()
  await openPanel(page)
  await page.getByTestId('recordings-panel').getByRole('button', { name: 'Close', exact: true }).click()
  await expect(panel).toBeHidden()
  await expect(launcher).toBeFocused()
})

test('Load shows that it is loading, lists the clips, and sends one request', async ({ page }) => {
  const requests: string[] = []
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route(/\/api\/recordings\?/, async (route) => {
    requests.push(new URL(route.request().url()).search)
    await held
    await route.fulfill({ json: CLIPS })
  })
  await signIn(page, 'one')
  await playing(page)
  const panel = await openPanel(page)
  await page.getByTestId('recordings-day').fill('2026-10-27')
  const load = page.getByTestId('recordings-load')
  await load.focus()
  await page.keyboard.press('Enter')
  await expect(load).toHaveText('Loading…')
  await expect(load).toHaveAttribute('aria-busy', 'true')
  await expect(load).toBeFocused()
  await page.keyboard.press('Enter')
  release()
  await expect(panel.getByTestId('recordings-item')).toHaveCount(2)
  await expect(panel.getByTestId('recordings-item').first()).toHaveText('2026-10-27 08:00:001m 35s · 3.1 MB')
  await expect(page.getByTestId('recordings-status')).toHaveText('2 clips · Gate camera · 2026-10-27')
  await expect(load).toHaveAttribute('aria-busy', 'false')
  await expect(load).toHaveText('Load')
  await expect(load).toBeFocused()
  expect(requests).toEqual(['?day=2026-10-27&channel=0&device=gate'])
})

test('an empty day says so for the chosen camera and day', async ({ page }) => {
  await page.route(/\/api\/recordings\?/, (route) => route.fulfill({ json: { recordings: [] } }))
  await signIn(page, 'one')
  await playing(page)
  await openPanel(page)
  await page.getByTestId('recordings-day').fill('2026-10-26')
  await page.getByTestId('recordings-load').click()
  await expect(page.getByTestId('recordings-status')).toHaveText('No cloud clips for Gate camera on 2026-10-26.')
  await expect(page.getByTestId('recordings-list')).toHaveCount(0)
})

test('the answer names the camera and day that were asked, even when the form changed while loading', async ({ page }) => {
  const requests: string[] = []
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route(/\/api\/recordings\?/, async (route) => {
    requests.push(new URL(route.request().url()).search)
    await held
    await route.fulfill({ json: { recordings: [] } })
  })
  await signIn(page, 'many')
  await playing(page)
  await openPanel(page)
  await expect(page.getByTestId('recordings-camera')).toHaveValue('gate')
  await page.getByTestId('recordings-day').fill('2026-10-26')
  await page.getByTestId('recordings-load').click()
  await expect(page.getByTestId('recordings-load')).toHaveAttribute('aria-busy', 'true')
  await page.getByTestId('recordings-camera').selectOption('porch')
  await page.getByTestId('recordings-day').fill('2026-10-20')
  release()
  await expect(page.getByTestId('recordings-status')).toHaveText('No cloud clips for Gate camera on 2026-10-26.')
  expect(requests).toEqual(['?day=2026-10-26&channel=0&device=gate'])
})

test('opening Cloud again does not ask the server for the camera list', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  let devices = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') devices += 1
  })
  for (let round = 0; round < 3; round += 1) {
    await openPanel(page)
    await expect(page.getByTestId('recordings-camera').locator('option')).toHaveCount(5)
    await page.getByTestId('recordings-panel').getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.getByTestId('recordings-panel')).toBeHidden()
  }
  expect(devices).toBe(0)
})

test('a failed load is an error with the server reason, not a grey note', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await openPanel(page)
  await page.getByTestId('recordings-load').click()
  const error = page.getByTestId('recordings-error')
  await expect(error).toHaveAttribute('role', 'alert')
  await expect(error).toHaveText(/^This session has no cloud token/)
  const colours = await error.evaluate((node) => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--bad)'
    document.body.append(probe)
    const bad = getComputedStyle(probe).color
    probe.remove()
    return { text: getComputedStyle(node).color, bad }
  })
  expect(colours.text).toBe(colours.bad)
})

test('an expired session during Load goes back to sign-in', async ({ page }) => {
  await page.route(/\/api\/recordings\?/, (route) => route.fulfill({ status: 401, json: { message: 'signed out' } }))
  await signIn(page, 'one')
  await playing(page)
  const panel = await openPanel(page)
  await page.getByTestId('recordings-load').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(panel).toBeHidden()
})
