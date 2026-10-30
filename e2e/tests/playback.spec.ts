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

test('#/live/playback opens Playback after sign-in and leaving it goes back to #/live', async ({ page }) => {
  const asked: string[] = []
  await page.route(/\/api\/playback\/files/, (route) => {
    asked.push(deviceOf(route.request().url()) || '')
    return route.fulfill({ json: BODY })
  })
  const base = await signIn(page, 'one')
  await playing(page)
  await page.goto(base + '/#/live/playback')
  const screen = page.getByTestId('playback')
  await expect(screen).toBeVisible()
  await expect(page.locator('.pb-row')).toHaveCount(3)
  expect(asked).toEqual(['gate'])
  await page.getByTestId('pb-back').click()
  await expect(screen).toBeHidden()
  await expect(page).toHaveURL(/#\/live$/)
  await page.evaluate(() => (location.hash = '/live/playback'))
  await expect(screen).toBeVisible()
})

test('leaving Playback opened from the grid puts #/live/grid back, not #/live', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await signIn(page, 'many')
  await playing(page)
  await page.getByTestId('view-grid').click()
  await expect(page).toHaveURL(/#\/live\/grid$/)
  await page.evaluate(() => (location.hash = '/live/playback'))
  await expect(page.getByTestId('playback')).toBeVisible()
  await page.getByTestId('pb-back').click()
  await expect(page.getByTestId('playback')).toBeHidden()
  await expect(page).toHaveURL(/#\/live\/grid$/)
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

const PIXEL = 'data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q=='

test('the calendar marks the days the camera reports and a marked day opens its list', async ({ page }) => {
  const months: string[] = []
  await page.route(/\/api\/playback\/calendar/, (route) => {
    const url = new URL(route.request().url())
    months.push(url.searchParams.get('year') + '-' + url.searchParams.get('month'))
    return route.fulfill({ json: { ok: true, days: url.searchParams.get('month') === '10' ? [25, 27] : [] } })
  })
  const days: string[] = []
  await page.route(/\/api\/playback\/files/, (route) => {
    days.push(dayOf(route.request().url()))
    return route.fulfill({ json: BODY })
  })
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-26')
  await page.getByTestId('pb-cal-open').click()
  const calendar = page.getByTestId('pb-calendar')
  await expect(calendar).toBeVisible()
  await expect(page.getByTestId('pb-cal-note')).toHaveText('2 days with recordings in October 2026.')
  await expect(calendar.locator('.pb-cal-day[data-has]')).toHaveText(['25', '27'])
  await expect(calendar.getByRole('button', { name: '2026-10-26, no recordings' })).toHaveAttribute('aria-pressed', 'true')
  await page.getByRole('button', { name: 'Previous month' }).click()
  await expect(page.getByTestId('pb-cal-note')).toHaveText('No recordings on the camera in September 2026.')
  await page.getByRole('button', { name: 'Next month' }).click()
  await calendar.getByRole('button', { name: '2026-10-27, has recordings' }).click()
  await expect(calendar).toBeHidden()
  await expect(page.getByTestId('pb-day')).toHaveValue('2026-10-27')
  await expect(page.locator('.pb-row')).toHaveCount(3)
  expect(months).toEqual(['2026-10', '2026-9'])
  expect(days.at(-1)).toBe('2026-10-27')
})

test('a calendar the device refuses says so without hiding the list', async ({ page }) => {
  await page.route(/\/api\/playback\/calendar/, (route) => route.fulfill({ status: 504, json: { message: 'playback stage=calendar: the device did not answer inside 20.0 s' } }))
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await page.getByTestId('pb-cal-open').click()
  await expect(page.getByTestId('pb-cal-note')).toHaveText('The calendar did not load: playback stage=calendar: the device did not answer inside 20.0 s')
  await expect(page.locator('.pb-row')).toHaveCount(3)
  await expect(page.getByTestId('pb-error')).toHaveCount(0)
})

test('the record type boxes ask the device for regular or alarm files only, never neither', async ({ page }) => {
  const asked: (string | null)[] = []
  await page.route(/\/api\/playback\/files/, (route) => {
    const event = new URL(route.request().url()).searchParams.get('event')
    asked.push(event)
    return route.fulfill({ json: event === 'alarm' ? { ...BODY, files: [FILES[1]] } : BODY })
  })
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-27')
  await expect(page.locator('.pb-row')).toHaveCount(3)
  await page.getByTestId('pb-kind-regular').uncheck()
  await expect(page.locator('.pb-row')).toHaveCount(1)
  await expect(page.getByTestId('pb-summary')).toHaveText('1 files · 28.61 MB · Gate camera on 2026-10-27 (alarm only)')
  await page.getByTestId('pb-kind-alarm').click()
  await expect(page.getByTestId('pb-kind-regular')).toBeChecked()
  await expect(page.getByTestId('pb-kind-alarm')).not.toBeChecked()
  await expect(page.getByTestId('pb-summary')).toHaveText('3 files · 128.75 MB · Gate camera on 2026-10-27 (regular only)')
  expect(asked.slice(-2)).toEqual(['alarm', 'regular'])
  await expect(page.locator('.pb-legend')).toHaveText('ContinuousMotionAlarm')
})

test('rows on screen ask for thumbnails in one batch and show the pictures the device returns', async ({ page }) => {
  const batches: string[][] = []
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await page.route(/\/api\/playback\/thumbnails/, (route) => {
    const body = route.request().postDataJSON() as { files: { name: string }[] }
    batches.push(body.files.map((file) => file.name))
    return route.fulfill({
      json: {
        ok: true,
        thumbs: body.files.map((file, index) =>
          index === 2 ? { name: file.name, image: null, error: 'keyframe: no keyframe inside 8 s' } : { name: file.name, source: 'picture', image: PIXEL },
        ),
      },
    })
  })
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-27')
  await expect(page.locator('.pb-thumb[data-state="ok"]')).toHaveCount(2)
  await expect(page.locator('.pb-thumb[data-state="none"]')).toHaveCount(1)
  await expect(page.locator('.pb-thumb[data-state="none"]')).toHaveAttribute('title', 'keyframe: no keyframe inside 8 s')
  expect(batches).toEqual([FILES.map((file) => file.name)])
})

test('the player buttons wait for a recording and sound waits for 1x speed', async ({ page }) => {
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: BODY }))
  await page.route(/\/api\/playback\/thumbnails/, (route) => route.fulfill({ json: { ok: true, thumbs: [] } }))
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-27')
  await expect(page.getByTestId('pb-pause')).toBeDisabled()
  await expect(page.getByTestId('pb-sound')).toBeDisabled()
  await expect(page.getByTestId('pb-snapshot')).toBeDisabled()
  await expect(page.getByTestId('pb-playing')).toHaveText('Pick a recording from the list or the strip.')
  await page.locator('.pb-row').nth(0).click()
  await expect(page.getByTestId('pb-pause')).toBeEnabled()
  await expect(page.getByTestId('pb-sound')).toBeEnabled()
  await expect(page.getByTestId('pb-snapshot-note')).toHaveText('Snapshot waits for the first picture.')
  await page.getByTestId('pb-speed').selectOption('2')
  await expect(page.getByTestId('pb-sound')).toBeDisabled()
  await expect(page.getByTestId('pb-sound-note')).toHaveText('Sound plays at 1x only.')
  await expect(page.getByTestId('pb-snapshot-note')).toHaveText('Snapshot waits for the first picture.')
  await expect(page.locator('#pb-video')).toHaveJSProperty('playbackRate', 2)
  await page.getByTestId('pb-speed').selectOption('0.25')
  await expect(page.locator('#pb-video')).toHaveJSProperty('playbackRate', 0.25)
})

test('changing the day while a thumbnail batch is in flight sends the next batch only after it returns', async ({ page }) => {
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  const asked: string[][] = []
  let open = 0
  let most = 0
  const other = FILES.map((file) => ({ ...file, name: file.name.replace('2026-10-27', '2026-10-26'), begin: file.begin.replace('10-27', '10-26'), end: file.end.replace('10-27', '10-26') }))
  await page.route(/\/api\/playback\/files/, (route) =>
    route.fulfill({ json: dayOf(route.request().url()) === '2026-10-26' ? { ...BODY, files: other } : BODY }),
  )
  await page.route(/\/api\/playback\/thumbnails/, async (route) => {
    const names = (route.request().postDataJSON() as { files: { name: string }[] }).files.map((file) => file.name)
    asked.push(names)
    open += 1
    most = Math.max(most, open)
    if (asked.length === 1) await held
    open -= 1
    return route.fulfill({ json: { ok: true, thumbs: names.map((name) => ({ name, source: 'picture', image: PIXEL })) } })
  })
  await signIn(page, 'one')
  await playing(page)
  await openPlayback(page)
  await pickDay(page, '2026-10-27')
  await expect.poll(() => asked.length).toBe(1)
  await pickDay(page, '2026-10-26')
  await expect(page.locator('.pb-row')).toHaveCount(3)
  await page.waitForTimeout(500)
  expect(asked.length).toBe(1)
  release()
  await expect(page.locator('.pb-thumb[data-state="ok"]')).toHaveCount(3)
  expect(asked[1]).toEqual(other.map((file) => file.name))
  expect(most).toBe(1)
})
