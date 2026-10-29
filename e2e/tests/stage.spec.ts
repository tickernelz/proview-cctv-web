import { test, expect } from '@playwright/test'
import { playing, signIn } from './app'

const ATTEMPT = {
  message: 'camera did not answer',
  stage: 'login',
  elapsed_ms: 4012,
  detail: 'no reply in 4 s',
  stages: [
    { stage: 'ticket', ok: true, elapsed_ms: 120, detail: 'ok' },
    { stage: 'login', ok: false, elapsed_ms: 4012, detail: 'no reply in 4 s' },
  ],
}

test('a failed stream is announced with a warning icon, names its stages, and copies them', async ({ page, browserName }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.route('**/api/stream?**', (route) => route.fulfill({ status: 502, json: ATTEMPT }))
  await page.keyboard.press('2')
  const well = page.getByTestId('live-well')
  await expect(well).toHaveAttribute('data-state', 'error')
  const message = page.locator('#well-msg')
  await expect(message).toHaveAttribute('role', 'alert')
  await expect(page.getByTestId('live-title')).toHaveText('No picture from Porch')
  await expect(page.getByTestId('live-title').locator('use')).toHaveAttribute('href', '#i-warning')
  await expect(page.locator('#well-text')).toHaveText('Failed at login after 4012 ms: no reply in 4 s')
  await expect(page.locator('#well-stage')).toHaveText('Stages: ticket ok 120 ms · login failed 4012 ms')
  const copy = page.locator('#well-stage-copy')
  await expect(copy).toBeVisible()
  if (browserName === 'chromium') {
    await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
    await copy.click()
    await expect(copy).toHaveText('Copied')
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
      'Failed at login after 4012 ms: no reply in 4 s\nticket ok in 120 ms: ok\nlogin failed in 4012 ms: no reply in 4 s',
    )
    await expect(copy).toHaveText('Copy the stage detail', { timeout: 4000 })
  }
  await page.unroute('**/api/stream?**')
  await page.locator('#retry').click()
  await playing(page)
  await expect(message).toBeHidden()
  await expect(page.locator('#well-stage')).toBeHidden()
})

test('a 502 without stages points at the reason it shows, not at a tooltip', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.route('**/api/stream?**', (route) => route.fulfill({ status: 502, json: { message: 'device login failed: wrong password' } }))
  await page.keyboard.press('3')
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-state', 'error')
  await expect(page.locator('#well-text')).toHaveText('The camera or the recording did not answer, so the server has no playable stream for this channel. Its reason is below.')
  await expect(page.locator('#well-code')).toHaveText('device login failed: wrong password')
  await expect(page.locator('#well-stage-copy')).toBeHidden()
  await page.locator('#check-password').click()
  await expect(page.getByTestId('settings')).toBeVisible()
  await expect(page).toHaveURL(/#settings\/config$/)
})

test('an offline camera says when the picture comes back without repeating the banner', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('5')
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-state', 'offline')
  await expect(page.getByTestId('live-title')).toHaveText('Camera offline')
  await expect(page.locator('#well-text')).toHaveText('The picture comes back once Shed answers and the list is refreshed.')
  await expect(page.locator('#well-msg')).not.toHaveAttribute('role', 'alert')
  await expect(page.getByTestId('live-frame')).toBeHidden()
})

test('an account without cameras shows only the empty state, and Refresh asks for the list once', async ({ page }) => {
  await signIn(page, 'empty')
  const empty = page.getByTestId('devices-empty')
  await expect(empty).toBeVisible()
  await expect(page.locator('#live')).toBeHidden()
  await expect(page.getByTestId('grid')).toBeHidden()
  let lists = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') lists += 1
  })
  await page.getByTestId('devices-refresh').click()
  await expect.poll(() => lists).toBe(1)
  await expect(empty).toBeVisible()
})
