import { test, expect, type Page } from '@playwright/test'
import { playing, scenarioUrl, signIn } from './app'

test('the address opens the screen it names, and a hash change moves between them', async ({ page }) => {
  const base = await signIn(page, 'many')
  await page.goto(base + '/?signed-in&boot=settings#settings/server')
  await expect(page.getByTestId('settings')).toBeVisible()
  await expect(page.locator('#h-server')).toBeVisible()
  await page.evaluate(() => (location.hash = '#grid'))
  await expect(page.getByTestId('grid')).toBeVisible()
  await expect(page.getByTestId('live-well')).toBeHidden()
  await page.evaluate(() => (location.hash = '#live'))
  await expect(page.getByTestId('live-well')).toBeVisible()
  await expect(page.getByTestId('grid')).toBeHidden()
})

test('a theme chosen with the toggle survives a reload and drives every toggle label', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' })
  await signIn(page, 'one')
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  await page.getByTestId('live').getByTestId('theme-toggle').click()
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  await page.reload()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  await expect(page.getByTestId('live').getByTestId('theme-toggle')).toHaveAttribute('aria-label', 'Switch to the dark theme')
})

test('Sign out from Settings returns to sign-in, keeps no camera, and a fresh sign-in opens Live again', async ({ page }) => {
  const base = await signIn(page, 'one')
  await page.goto(base + '/?signed-in&boot=connection#settings/connection')
  await page.getByTestId('settings-signout').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(page.getByTestId('settings')).toBeHidden()
  const me = await page.evaluate(() => fetch('/api/me').then((reply) => reply.status))
  expect(me).toBe(401)
  await page.goto(scenarioUrl('one'))
  await expect(page.getByTestId('signin')).toBeVisible()
})

test('a 401 on a signed-in call returns to sign-in to resume the same camera', async ({ page }) => {
  const base = await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await page.route('**/api/devices', (route) => route.fulfill({ status: 401, json: { message: 'session expired' } }))
  await page.getByTestId('camera-switcher').click()
  await page.getByTestId('camera-menu').getByRole('menuitem', { name: 'Refresh the list', exact: true }).click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(page.getByTestId('signin-submit')).toHaveText('Sign in to resume')
  await page.unroute('**/api/devices')
  await page.getByTestId('signin-email').fill('fixture@example.invalid')
  await page.getByTestId('signin-password').fill('fixture-password')
  await page.getByTestId('signin-submit').click()
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  expect(base).toBeTruthy()
})

const CLIP = { name: '/idea0/2026-10-27/001/01.10.00-01.15.00[R][@1][0].h264', begin: '2026-10-27 01:10:00', end: '2026-10-27 01:15:00', bytes: 15000000, disk: null }

async function playingClip(page: Page) {
  const socket = { opened: 0, closed: 0 }
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: { ok: true, files: [CLIP], truncated: false, cap: 2048, elapsed_ms: 3 } }))
  await page.routeWebSocket(/\/ws\/playback/, (ws) => {
    socket.opened += 1
    ws.onClose(() => {
      socket.closed += 1
    })
  })
  const base = await signIn(page, 'many')
  await page.goto(base + '/?signed-in&boot=playback#playback')
  await expect(page.getByTestId('playback')).toBeVisible()
  await page.getByTestId('pb-day').fill('2026-10-27')
  await page.locator('.pb-row').first().click()
  await expect.poll(() => socket.opened).toBe(1)
  return socket
}

const LEAVE: [string, (page: Page) => Promise<unknown>][] = [
  ['the desktop tray Live item', (page) => page.evaluate(() => window.dispatchEvent(new CustomEvent('desktop-live')))],
  ['the Settings address', (page) => page.evaluate(() => (location.hash = '#settings/server'))],
  ['the grid address', (page) => page.evaluate(() => (location.hash = '#grid'))],
]

for (const [label, leave] of LEAVE) {
  test('leaving a playing clip through ' + label + ' closes its playback socket', async ({ page }) => {
    const socket = await playingClip(page)
    await leave(page)
    await expect(page.getByTestId('playback')).toBeHidden()
    await expect.poll(() => socket.closed).toBe(1)
  })
}

test('the address of another screen closes the Playback overlay', async ({ page }) => {
  const base = await signIn(page, 'many')
  await page.goto(base + '/?signed-in&boot=playback#playback')
  await expect(page.getByTestId('playback')).toBeVisible()
  await page.evaluate(() => (location.hash = '#settings/server'))
  await expect(page.locator('#h-server')).toBeVisible()
  await expect(page.getByTestId('playback')).toBeHidden()
})

test('the desktop tray Live item closes the Playback overlay and gives the digit keys back', async ({ page }) => {
  const base = await signIn(page, 'many')
  await page.goto(base + '/?signed-in&boot=playback#playback')
  await expect(page.getByTestId('playback')).toBeVisible()
  await page.evaluate(() => window.dispatchEvent(new CustomEvent('desktop-live')))
  await expect(page.getByTestId('playback')).toBeHidden()
  await playing(page)
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
})
