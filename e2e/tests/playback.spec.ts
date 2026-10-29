import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

const clip = (h: number, m: number, len: number, mode = '') => {
  const p = (n: number) => String(n).padStart(2, '0')
  const end = h * 3600 + m * 60 + len
  return {
    name: '/idea0/2026-10-27/001/' + p(h) + '.' + p(m) + '.00-x' + mode + '.h264',
    begin: '2026-10-27 ' + p(h) + ':' + p(m) + ':00',
    end: '2026-10-27 ' + p(Math.floor(end / 3600)) + ':' + p(Math.floor(end / 60) % 60) + ':' + p(end % 60),
    bytes: len * 50000,
    disk: null,
  }
}
const FILES = [clip(1, 10, 300), clip(6, 0, 600, '[M]'), clip(8, 30, 1800)]
const BODY = { ok: true, files: FILES, truncated: false, cap: 2048, elapsed_ms: 12 }

const dayOf = (url: string) => (new URL(url).searchParams.get('begin') || '').slice(0, 10)
const deviceOf = (url: string) => new URL(url).searchParams.get('device')

async function openPlayback(page: Page) {
  await page.getByTestId('pb-open').click()
  const screen = page.getByTestId('playback')
  await expect(screen).toBeVisible()
  return screen
}

async function pickDay(page: Page, day: string) {
  await page.getByTestId('pb-day').fill(day)
}

test('Playback opens on the camera on screen, keeps keys to itself, and Escape returns to Live', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  const launcher = page.getByTestId('pb-open')
  await openPlayback(page)
  await expect(page.getByTestId('pb-device')).toHaveValue('porch')
  await expect(page.locator('#pb-title')).toBeFocused()
  await page.keyboard.press('3')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('playback')).toBeHidden()
  await expect(launcher).toBeFocused()
  await openPlayback(page)
  await page.getByTestId('pb-back').click()
  await expect(page.getByTestId('playback')).toBeHidden()
  await expect(launcher).toBeFocused()
})

test('after a click on the strip or the summary, keys still stay out of Live and Escape closes Playback', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await signIn(page, 'many')
  await playing(page)
  await expect(page.getByTestId('camera-name')).toHaveText('Gate camera')
  const screen = await openPlayback(page)
  await pickDay(page, '2026-10-27')
  await expect(page.locator('.pb-row')).toHaveCount(3)
  await page.getByTestId('pb-summary').click()
  await page.keyboard.press('3')
  await page.locator('#pb-timeline').click({ position: { x: 4, y: 16 } })
  await page.keyboard.press('5')
  await expect(page.getByTestId('pb-playing')).toHaveText('2026-10-27 01:10:00 → 2026-10-27 01:15:00')
  await expect(page.getByTestId('camera-name')).toHaveText('Gate camera')
  await page.keyboard.press('Escape')
  await expect(screen).toBeHidden()
  await expect(page.getByTestId('pb-open')).toBeFocused()
  await page.keyboard.press('3')
  await expect(page.getByTestId('camera-name')).toHaveText('Garage')
})

test('#playback opens Playback after sign-in and leaving it goes back to #live', async ({ page }) => {
  const asked: string[] = []
  await page.route(/\/api\/playback\/files/, (route) => {
    asked.push(deviceOf(route.request().url()) || '')
    return route.fulfill({ json: BODY })
  })
  const base = await signIn(page, 'one')
  await playing(page)
  await page.goto(base + '/#playback')
  const screen = page.getByTestId('playback')
  await expect(screen).toBeVisible()
  await expect(page.locator('.pb-row')).toHaveCount(3)
  expect(asked).toEqual(['gate'])
  await page.getByTestId('pb-back').click()
  await expect(screen).toBeHidden()
  await expect(page).toHaveURL(/#live$/)
  await page.evaluate(() => (location.hash = 'playback'))
  await expect(screen).toBeVisible()
})

test('the day lists its recordings with camera and day, and a chosen clip is marked in the list and the strip', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-27')
  await expect(page.getByTestId('pb-summary')).toHaveText('3 files · 128.75 MB · Gate camera on 2026-10-27')
  const rows = page.locator('.pb-row')
  await expect(rows).toHaveCount(3)
  await expect(rows.nth(1)).toHaveText(/^06:00:00 – 06:10:00\s*28\.61 MB · motion$/)
  await expect(page.getByTestId('pb-seek')).toBeDisabled()
  await expect(page.locator('#pb-download')).toBeDisabled()
  await rows.nth(2).click()
  await expect(rows.nth(2)).toHaveAttribute('aria-pressed', 'true')
  await expect(page.locator('.pb-seg[data-current]')).toHaveCount(1)
  await expect(page.getByTestId('pb-playing')).toHaveText('2026-10-27 08:30:00 → 2026-10-27 09:00:00')
  await expect(page.locator('#pb-download')).toBeEnabled()
})

test('a failed file query is an alert with the device reason, and an empty day names the camera and day', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) =>
    dayOf(route.request().url()) === '2026-10-26'
      ? route.fulfill({ status: 502, json: { message: 'the device did not answer the file query' } })
      : route.fulfill({ json: { ...BODY, files: [] } }),
  )
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-26')
  const error = page.getByTestId('pb-error')
  await expect(error).toHaveText('The device did not answer the file query')
  await expect(error).toHaveAttribute('role', 'alert')
  await expect(page.getByTestId('pb-summary')).toBeHidden()
  await page.getByRole('button', { name: 'Next day' }).click()
  await expect(page.getByTestId('pb-summary')).toHaveText('No recordings of Gate camera on 2026-10-27.')
  await expect(error).toHaveCount(0)
})

test('changing the day while a list is loading shows only the day asked last', async ({ page }) => {
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route(/\/api\/playback\/files/, async (route) => {
    if (dayOf(route.request().url()) === '2026-10-26') {
      await held
      return route.fulfill({ json: BODY })
    }
    return route.fulfill({ json: { ...BODY, files: [] } })
  })
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-26')
  await expect(page.getByTestId('pb-summary')).toHaveText('Loading the recordings of Gate camera on 2026-10-26…')
  await pickDay(page, '2026-10-27')
  await expect(page.getByTestId('pb-summary')).toHaveText('No recordings of Gate camera on 2026-10-27.')
  const answered = page.waitForResponse((response) => dayOf(response.url()) === '2026-10-26')
  release()
  await (await answered).finished()
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  await expect(page.locator('.pb-row')).toHaveCount(0)
  await expect(page.getByTestId('pb-summary')).toHaveText('No recordings of Gate camera on 2026-10-27.')
})

test('Playback reuses the camera list the app loaded', async ({ page }) => {
  const asked: string[] = []
  await page.route(/\/api\/playback\/files/, (route) => {
    asked.push(deviceOf(route.request().url()) || '')
    return route.fulfill({ json: BODY })
  })
  let devices = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') devices += 1
  })
  await signIn(page, 'many')
  await playing(page)
  await openPlayback(page)
  await expect(page.getByTestId('pb-device').locator('option')).toHaveCount(5)
  await expect(page.locator('.pb-row')).toHaveCount(3)
  expect(devices).toBe(1)
  expect(asked).toEqual(['gate'])
})

test('an expired session while listing goes back to sign-in', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ status: 401, json: { message: 'signed out' } }))
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('pb-open').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(page.getByTestId('playback')).toBeHidden()
})
