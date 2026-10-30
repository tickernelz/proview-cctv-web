import { readFileSync } from 'node:fs'
import { test, expect, type Page, type WebSocketRoute } from '@playwright/test'
import { playing, signIn } from './app'

const MP4 = readFileSync(new URL('./fixtures/playback-640x720.mp4', import.meta.url))
const NOW = new Date('2026-10-27T20:00:00')

const pad = (n: number) => String(n).padStart(2, '0')
const clip = (h: number, m: number, len: number, mode = '', day = '2026-10-27') => {
  const end = h * 3600 + m * 60 + len
  return {
    name: '/idea0/' + day + '/001/' + pad(h) + '.' + pad(m) + '.00-x' + mode + '.h264',
    begin: day + ' ' + pad(h) + ':' + pad(m) + ':00',
    end: day + ' ' + pad(Math.floor(end / 3600)) + ':' + pad(Math.floor(end / 60) % 60) + ':' + pad(end % 60),
    bytes: len * 50000,
    disk: null,
  }
}
const FILES = [clip(1, 10, 300), clip(6, 0, 600, '[M]'), clip(8, 30, 1800)]
const BODY = { ok: true, files: FILES, truncated: false, cap: 2048, elapsed_ms: 12 }
const ALARMS = {
  entries: [
    { id: 'm1', event: 'VideoMotion', time: '2026-10-27 06:05:00', utcTime: 0 },
    { id: 'a1', event: 'appEventHumanDetectAlarm', time: '2026-10-27 08:40:00', utcTime: 0 },
  ],
  total: 2,
}
const PIXEL = 'data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q=='

const dayOf = (url: string) => (new URL(url).searchParams.get('begin') || '').slice(0, 10)
const param = (url: string, name: string) => new URL(url).searchParams.get(name)

type Sockets = { urls: string[]; closed: number }

async function prepare(page: Page, files: unknown = BODY) {
  await page.clock.setFixedTime(NOW)
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  if (files) await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: files }))
}

async function fakePlayback(page: Page, frames = true): Promise<Sockets> {
  const sockets: Sockets = { urls: [], closed: 0 }
  await page.routeWebSocket(/\/ws\/playback/, (ws: WebSocketRoute) => {
    sockets.urls.push(ws.url())
    ws.onClose(() => {
      sockets.closed += 1
    })
    ws.onMessage(() => {})
    if (!frames) return
    ws.send(JSON.stringify({ type: 'status', fps: 25, openMs: 300, primeMs: 120, codec: 'h264', windowSec: 3600 }))
    ws.send(JSON.stringify({ type: 'clock', frame: 0, at: param(ws.url(), 'begin') }))
    ws.send(MP4)
  })
  return sockets
}

type Live = { opened: number; closed: number }

async function trackLive(page: Page): Promise<Live> {
  const live: Live = { opened: 0, closed: 0 }
  await page.routeWebSocket(/\/ws\/live/, (ws: WebSocketRoute) => {
    live.opened += 1
    const server = ws.connectToServer()
    let gone = false
    const done = () => {
      if (gone) return
      gone = true
      live.closed += 1
    }
    ws.onClose((code, reason) => {
      done()
      server.close({ code, reason })
    })
    server.onClose((code, reason) => {
      done()
      ws.close({ code, reason })
    })
  })
  return live
}

const openSockets = (live: Live, playback: Sockets) => ({ live: live.opened - live.closed, playback: playback.urls.length - playback.closed })

async function trackPoint(page: Page, time: string) {
  const box = await page.getByTestId('tl-track').boundingBox()
  if (!box) throw new Error('the timeline track has no box')
  const [h, m, s] = time.split(':').map(Number)
  return { x: box.x + (box.width * (h * 3600 + m * 60 + (s || 0))) / 86400, y: box.y + box.height / 2 }
}

async function clickAt(page: Page, time: string) {
  const point = await trackPoint(page, time)
  await page.mouse.click(point.x, point.y)
}

test('the timeline shows the day of the camera on screen: recorded spans, motion and alarm markers, the live pill', async ({ page }) => {
  const asked: string[] = []
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, (route) => {
    asked.push(param(route.request().url(), 'device') + ' ' + param(route.request().url(), 'begin') + ' ' + param(route.request().url(), 'end'))
    return route.fulfill({ json: BODY })
  })
  await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await expect.poll(() => asked.at(-1)).toBe('porch 2026-10-27 00:00:00 2026-10-27 23:59:59')
  const timeline = page.getByTestId('timeline')
  await expect(timeline.getByTestId('tl-bar')).toHaveCount(3)
  await expect(timeline.getByTestId('tl-mark')).toHaveCount(2)
  await expect(timeline.locator('[data-testid="tl-mark"][data-kind="motion"]')).toHaveCount(1)
  await expect(timeline.locator('[data-testid="tl-mark"][data-kind="alarm"]')).toHaveCount(1)
  await expect(page.getByTestId('tl-live')).toHaveText('LIVE')
  await expect(page.getByTestId('tl-pill')).toHaveText('20:00:00')
  await expect(page.getByTestId('tl-day')).toContainText('Today, 27 Oct')
  await expect(page.getByTestId('tl-next-day')).toBeDisabled()
  await expect(page.getByTestId('tl-track')).toHaveAttribute('aria-valuetext', '20:00:00, Today, 27 Oct, live')
})

test('a click on a recorded point plays from that time inside Live, and Back to live returns to the live stream', async ({ page }) => {
  await prepare(page)
  const sockets = await fakePlayback(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await clickAt(page, '08:52:00')
  await expect.poll(() => sockets.urls.length).toBe(1)
  const url = sockets.urls[0]
  expect(param(url, 'mode')).toBe('time')
  expect(param(url, 'device')).toBe('gate')
  expect(param(url, 'end')).toBe('2026-10-27 23:59:59')
  const begin = new Date(String(param(url, 'begin')).replace(' ', 'T')).getTime()
  expect(Math.abs(begin - new Date('2026-10-27T08:52:00').getTime())).toBeLessThan(3 * 60_000)
  await expect(page.getByTestId('playback-controls')).toBeVisible()
  await expect(page.getByTestId('ctl-talk')).toBeDisabled()
  await expect(page.getByTestId('ctl-move')).toBeDisabled()
  await expect(page.getByTestId('playback-note')).toBeVisible()
  await expect(page.getByTestId('ctl-record')).toHaveCount(0)
  await expect(page.getByTestId('tl-live')).toHaveCount(0)
  await expect(page.getByTestId('live-plate')).toHaveAttribute('data-tone', 'rec')
  await expect(page.getByTestId('live-plate')).toContainText(/RECs*08:5d:dd/)
  await expect(page.getByTestId('pb-snapshot')).toBeEnabled()
  await expect(page.getByTestId('pb-frame-next')).toBeEnabled()
  await expect(page.getByTestId('tl-pill')).toHaveText(/^08:5\d:\d\d$/)
  await page.getByTestId('tl-back-live').click()
  await expect.poll(() => sockets.closed).toBe(1)
  await expect(page.getByTestId('playback-controls')).toHaveCount(0)
  await expect(page.getByTestId('ctl-talk')).toBeVisible()
  await expect(page.getByTestId('tl-live')).toHaveText('LIVE')
  await playing(page)
})

test('a click on an event marker starts playback 5 s before the event', async ({ page }) => {
  await prepare(page)
  const sockets = await fakePlayback(page, false)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-mark')).toHaveCount(2)
  await clickAt(page, '08:40:00')
  await expect.poll(() => sockets.urls.length).toBe(1)
  expect(param(sockets.urls[0], 'begin')).toBe('2026-10-27 08:39:55')
  await expect(page.getByTestId('pb-snapshot')).toBeDisabled()
  await expect(page.getByTestId('pb-snapshot-note')).toHaveText(/Snapshot and frame step wait for the first picture/)
})

test('the track takes the keyboard: arrows move a cursor, Enter plays the next recording, Escape goes back to live', async ({ page }) => {
  await prepare(page)
  const sockets = await fakePlayback(page, false)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  const track = page.getByTestId('tl-track')
  await track.focus()
  await page.keyboard.press('Home')
  for (let hour = 0; hour < 7; hour += 1) await page.keyboard.press('Shift+ArrowRight')
  await expect(track).toHaveAttribute('aria-valuetext', '07:00:00, Today, 27 Oct, live')
  await page.keyboard.press('Enter')
  await expect.poll(() => sockets.urls.length).toBe(1)
  expect(param(sockets.urls[0], 'begin')).toBe('2026-10-27 08:30:00')
  await page.getByTestId('pb-pause').focus()
  await page.keyboard.press('Escape')
  await expect.poll(() => sockets.closed).toBe(1)
  await expect(page.getByTestId('tl-live')).toBeVisible()
})

test('#/live/playback and the Playback button open the day of recordings, and a row plays from its start', async ({ page }) => {
  await prepare(page)
  const sockets = await fakePlayback(page, false)
  const base = await signIn(page, 'many')
  await playing(page)
  await page.goto(base + '/#/live/playback')
  const list = page.getByTestId('playback')
  await expect(list).toBeVisible()
  await expect(page).toHaveURL(/#\/live$/)
  await expect(list.locator('.pb-row')).toHaveCount(3)
  await page.keyboard.press('Escape')
  await expect(list).toBeHidden()
  await page.evaluate(() => (location.hash = '/live/playback'))
  await expect(list).toBeVisible()
  await expect(page).toHaveURL(/#\/live$/)
  await page.keyboard.press('Escape')
  await expect(list).toBeHidden()
  const launcher = page.getByTestId('pb-open')
  await launcher.click()
  await expect(list).toBeVisible()
  await page.keyboard.press('3')
  await expect(page.getByTestId('camera-name')).toHaveText('Gate camera')
  await page.keyboard.press('Escape')
  await expect(list).toBeHidden()
  await expect(launcher).toBeFocused()
  await launcher.click()
  await list.locator('.pb-row').nth(1).click()
  await expect(list).toBeHidden()
  await expect.poll(() => sockets.urls.length).toBe(1)
  expect(param(sockets.urls[0], 'begin')).toBe('2026-10-27 06:00:00')
  await expect(launcher).toHaveAttribute('aria-pressed', 'true')
  await launcher.click()
  await expect(list.locator('.pb-row').nth(1)).toHaveAttribute('aria-pressed', 'true')
})

test('the recording list names camera and day and downloads a file by name', async ({ page }) => {
  await prepare(page)
  await page.route(/\/api\/playback\/download\?/, (route) =>
    route.fulfill({ status: 200, body: 'clip', headers: { 'content-type': 'video/mp4', 'content-disposition': 'attachment; filename="clip.mp4"' } }),
  )
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('tl-recordings').click()
  await expect(page.getByTestId('pb-summary')).toHaveText('3 files · 128.75 MB · Gate camera on 2026-10-27')
  const rows = page.locator('.pb-row')
  await expect(rows).toHaveCount(3)
  await expect(rows.nth(1)).toHaveText(/06:00:00 – 06:10:00\s*28\.61 MB · motion/)
  const saved = page.waitForEvent('download')
  await page.getByRole('button', { name: 'Download 08:30:00 – 09:00:00' }).click()
  const url = (await saved).url()
  expect(param(url, 'device')).toBe('gate')
  expect(param(url, 'name')).toBe(FILES[2].name)
  expect(param(url, 'begin')).toBe(FILES[2].begin)
  expect(param(url, 'end')).toBe(FILES[2].end)
  expect(param(url, 'bytes')).toBe(String(FILES[2].bytes))
})

test('Download clip in the dock saves the file under the playhead', async ({ page }) => {
  await prepare(page)
  await page.route(/\/api\/playback\/download\?/, (route) =>
    route.fulfill({ status: 200, body: 'clip', headers: { 'content-type': 'video/mp4', 'content-disposition': 'attachment; filename="clip.mp4"' } }),
  )
  await fakePlayback(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await clickAt(page, '06:04:00')
  await expect(page.getByTestId('pb-download')).toBeEnabled()
  const saved = page.waitForEvent('download')
  await page.getByTestId('pb-download').click()
  expect(param((await saved).url(), 'name')).toBe(FILES[1].name)
})

test('a failed file query says why on the track and in the list, and an empty day names the day', async ({ page }) => {
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, (route) =>
    dayOf(route.request().url()) === '2026-10-26'
      ? route.fulfill({ status: 502, json: { message: 'the device did not answer the file query' } })
      : route.fulfill({ json: { ...BODY, files: [] } }),
  )
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-note')).toHaveText('No recording on 27 Oct.')
  await expect(page.getByTestId('tl-bar')).toHaveCount(0)
  await page.getByTestId('tl-prev-day').click()
  await expect(page.getByTestId('tl-note')).toHaveText('The recordings of 26 Oct did not load: The device did not answer the file query')
  await expect(page.getByTestId('tl-back-live')).toBeVisible()
  await page.getByTestId('tl-recordings').click()
  const error = page.getByTestId('pb-error')
  await expect(error).toHaveText('The device did not answer the file query')
  await expect(error).toHaveAttribute('role', 'alert')
  await page.getByTestId('playback').getByRole('button', { name: 'Next day' }).click()
  await expect(page.getByTestId('pb-summary')).toHaveText('No recordings of Gate camera on 2026-10-27.')
  await expect(error).toHaveCount(0)
})

test('changing the day while a list is loading shows only the day asked last', async ({ page }) => {
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, async (route) => {
    if (dayOf(route.request().url()) === '2026-10-26') {
      await held
      return route.fulfill({ json: { ...BODY, files: FILES.map((file) => ({ ...file, begin: file.begin.replace('10-27', '10-26'), end: file.end.replace('10-27', '10-26') })) } })
    }
    return route.fulfill({ json: { ...BODY, files: [] } })
  })
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('tl-prev-day').click()
  await expect(page.getByTestId('tl-note')).toHaveText('Loading the recordings of 26 Oct…')
  await page.getByTestId('tl-next-day').click()
  await expect(page.getByTestId('tl-note')).toHaveText('No recording on 27 Oct.')
  const answered = page.waitForResponse((response) => dayOf(response.url()) === '2026-10-26')
  release()
  await (await answered).finished()
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  await expect(page.getByTestId('tl-bar')).toHaveCount(0)
  await expect(page.getByTestId('tl-note')).toHaveText('No recording on 27 Oct.')
})

test('the timeline reuses the camera list the app loaded', async ({ page }) => {
  const asked: string[] = []
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, (route) => {
    asked.push(param(route.request().url(), 'device') || '')
    return route.fulfill({ json: BODY })
  })
  let devices = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') devices += 1
  })
  await signIn(page, 'many')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await page.getByTestId('tl-recordings').click()
  await expect(page.locator('.pb-row')).toHaveCount(3)
  expect(devices).toBe(1)
  expect(asked).toEqual(['gate'])
})

test('an expired session while listing goes back to sign-in', async ({ page }) => {
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ status: 401, json: { message: 'signed out' } }))
  await signIn(page, 'one')
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(page.getByTestId('timeline')).toBeHidden()
})

test('the calendar marks the days the camera reports and a marked day opens it on the timeline', async ({ page }) => {
  const months: string[] = []
  const days: string[] = []
  await prepare(page, null)
  await page.route(/\/api\/playback\/calendar/, (route) => {
    const url = new URL(route.request().url())
    months.push(url.searchParams.get('year') + '-' + url.searchParams.get('month'))
    return route.fulfill({ json: { ok: true, days: url.searchParams.get('month') === '10' ? [25, 27] : [] } })
  })
  await page.route(/\/api\/playback\/files/, (route) => {
    days.push(dayOf(route.request().url()))
    return route.fulfill({ json: BODY })
  })
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('tl-prev-day').click()
  await page.getByTestId('tl-day').click()
  const calendar = page.getByTestId('pb-calendar')
  await expect(calendar).toBeVisible()
  await expect(page.getByTestId('pb-cal-note')).toHaveText('2 days with recordings in October 2026.')
  await expect(calendar.locator('.pb-cal-day[data-has]')).toHaveText(['25', '27'])
  await expect(calendar.getByRole('button', { name: '2026-10-26, no recordings' })).toHaveAttribute('aria-pressed', 'true')
  await calendar.getByRole('button', { name: 'Previous month' }).click()
  await expect(page.getByTestId('pb-cal-note')).toHaveText('No recordings on the camera in September 2026.')
  await calendar.getByRole('button', { name: 'Next month' }).click()
  await calendar.getByRole('button', { name: '2026-10-25, has recordings' }).click()
  await expect(calendar).toBeHidden()
  await expect(page.getByTestId('tl-day')).toContainText('25 Oct')
  expect(months).toEqual(['2026-10', '2026-9'])
  expect(days.at(-1)).toBe('2026-10-25')
})

test('a calendar the device refuses says so without hiding the recordings', async ({ page }) => {
  await prepare(page)
  await page.route(/\/api\/playback\/calendar/, (route) => route.fulfill({ status: 504, json: { message: 'playback stage=calendar: the device did not answer inside 20.0 s' } }))
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('tl-day').click()
  await expect(page.getByTestId('pb-cal-note')).toHaveText('The calendar did not load: playback stage=calendar: the device did not answer inside 20.0 s')
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
})

test('the record type asks the device for regular or alarm files only and the timeline follows', async ({ page }) => {
  const asked: (string | null)[] = []
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, (route) => {
    const event = param(route.request().url(), 'event')
    asked.push(event)
    return route.fulfill({ json: event === 'alarm' ? { ...BODY, files: [FILES[1]] } : BODY })
  })
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('tl-recordings').click()
  await expect(page.locator('.pb-row')).toHaveCount(3)
  await page.getByTestId('pb-kind-alarm').click()
  await expect(page.locator('.pb-row')).toHaveCount(1)
  await expect(page.getByTestId('pb-summary')).toHaveText('1 files · 28.61 MB · Gate camera on 2026-10-27 (alarm only)')
  await expect(page.getByTestId('tl-bar')).toHaveCount(1)
  await page.getByTestId('pb-kind-regular').click()
  await expect(page.getByTestId('pb-summary')).toHaveText('3 files · 128.75 MB · Gate camera on 2026-10-27 (regular only)')
  await page.getByTestId('pb-kind-all').click()
  await expect(page.getByTestId('pb-summary')).toHaveText('3 files · 128.75 MB · Gate camera on 2026-10-27')
  expect(asked).toEqual([null, 'alarm', 'regular', null])
})

test('rows on screen ask for thumbnails in one batch, and hovering the track shows them as a filmstrip', async ({ page }) => {
  const batches: string[][] = []
  await prepare(page)
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
  await page.getByTestId('tl-recordings').click()
  await expect(page.locator('.pb-thumb[data-state="ok"]')).toHaveCount(2)
  await expect(page.locator('.pb-thumb[data-state="none"]')).toHaveCount(1)
  await expect(page.locator('.pb-thumb[data-state="none"]')).toHaveAttribute('title', 'keyframe: no keyframe inside 8 s')
  expect(batches).toEqual([FILES.map((file) => file.name)])
  await page.keyboard.press('Escape')
  const point = await trackPoint(page, '06:05:00')
  await page.mouse.move(point.x, point.y)
  const film = page.getByTestId('tl-film')
  await expect(film).toBeVisible()
  await expect(film.locator('.tl-frame')).toHaveCount(3)
  await expect(film.locator('.tl-frame.current .tl-frame-time')).toHaveText('06:00')
  await expect(film.locator('.tl-thumb[data-state="ok"]')).toHaveCount(2)
  expect(batches.length).toBe(1)
})

test('the player buttons wait for a picture and sound waits for 1x speed', async ({ page }) => {
  await prepare(page)
  const sockets = await fakePlayback(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await clickAt(page, '08:52:00')
  await expect.poll(() => sockets.urls.length).toBe(1)
  await expect(page.getByTestId('pb-snapshot')).toBeEnabled()
  await expect(page.getByTestId('pb-sound')).toBeEnabled()
  await page.getByTestId('pb-speed-2').click()
  await expect(page.getByTestId('pb-sound')).toBeDisabled()
  await expect(page.getByTestId('pb-sound-note')).toHaveText('Sound plays at 1x only.')
  await expect(page.locator('#video')).toHaveJSProperty('playbackRate', 2)
  await page.getByTestId('pb-speed-0.5').click()
  await expect(page.locator('#video')).toHaveJSProperty('playbackRate', 0.5)
  await page.getByTestId('pb-speed-1').click()
  await expect(page.getByTestId('pb-sound-note')).toHaveCount(0)
  const video = page.locator('#video')
  const pause = page.getByTestId('pb-pause')
  await video.evaluate(async (element: HTMLVideoElement) => {
    element.currentTime = 0.05
    await element.play()
  })
  await pause.click()
  await expect(video).toHaveJSProperty('paused', true)
  await expect(pause).toHaveAttribute('aria-pressed', 'true')
  await video.evaluate((element: HTMLVideoElement) => (element.currentTime = 0.4))
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.currentTime)).toBeCloseTo(0.4, 1)
  await page.getByTestId('pb-frame-next').click()
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.currentTime)).toBeGreaterThan(0.43)
  const stepped = await video.evaluate((element: HTMLVideoElement) => element.currentTime)
  await page.getByTestId('pb-frame-back').click()
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.currentTime)).toBeLessThan(stepped - 0.03)
})

test('the zoom buttons and the wheel narrow the span the ruler shows', async ({ page }) => {
  await prepare(page)
  await signIn(page, 'one')
  await playing(page)
  const hours = page.locator('.tl-hour')
  await expect(hours.first()).toHaveText('00')
  await expect(page.getByTestId('tl-zoom-out')).toBeDisabled()
  await page.getByTestId('tl-zoom-in').click()
  await expect(page.getByTestId('tl-zoom-6h')).toHaveAttribute('aria-pressed', 'true')
  await expect(hours.first()).toHaveText(/^\d\d:00$/)
  await page.getByTestId('tl-zoom-10m').click()
  await expect(page.getByTestId('tl-zoom-in')).toBeDisabled()
  await expect(page.getByTestId('tl-pill')).toHaveText('20:00:00')
  await page.getByTestId('tl-zoom-24h').click()
  const box = await page.getByTestId('tl-track').boundingBox()
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2)
  await page.mouse.wheel(0, -120)
  await expect(page.getByTestId('tl-zoom-6h')).toHaveAttribute('aria-pressed', 'true')
})

test('changing the day while a thumbnail batch is in flight sends the next batch only after it returns', async ({ page }) => {
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  const asked: string[][] = []
  let open = 0
  let most = 0
  const other = FILES.map((file) => ({ ...file, name: file.name.replace('2026-10-27', '2026-10-26'), begin: file.begin.replace('10-27', '10-26'), end: file.end.replace('10-27', '10-26') }))
  await prepare(page, null)
  await page.route(/\/api\/playback\/files/, (route) => route.fulfill({ json: dayOf(route.request().url()) === '2026-10-26' ? { ...BODY, files: other } : BODY }))
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
  await page.getByTestId('tl-recordings').click()
  await expect.poll(() => asked.length).toBe(1)
  await page.getByTestId('pb-day').fill('2026-10-26')
  await page.getByTestId('pb-day').blur()
  await expect(page.locator('.pb-row')).toHaveCount(3)
  await page.waitForTimeout(500)
  expect(asked.length).toBe(1)
  release()
  await expect(page.locator('.pb-thumb[data-state="ok"]')).toHaveCount(3)
  expect(asked[1]).toEqual(other.map((file) => file.name))
  expect(most).toBe(1)
})

test('in playback the live keys stay out: T, R, V and Q do nothing live, S saves the playback frame, Space pauses, arrows step', async ({ page }) => {
  await prepare(page)
  const live = await trackLive(page)
  const sockets = await fakePlayback(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await clickAt(page, '08:52:00')
  await expect(page.getByTestId('pb-snapshot')).toBeEnabled()
  const quality = await page.evaluate(() => JSON.parse(localStorage.getItem('pvw_prefs_v2') || '{}').quality ?? 'main')
  for (const key of ['t', 'r', 'v', 'q']) await page.keyboard.press(key)
  await expect(page.getByTestId('playback-controls')).toBeVisible()
  await expect(page.getByTestId('ptz-pad')).toBeHidden()
  await expect(page.getByTestId('ctl-record')).toHaveCount(0)
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('pvw_prefs_v2') || '{}').quality ?? 'main')).toBe(quality)
  expect(openSockets(live, sockets)).toEqual({ live: 0, playback: 1 })
  expect(sockets.urls.length).toBe(1)
  const saved = page.waitForEvent('download')
  await page.keyboard.press('s')
  expect((await saved).suggestedFilename()).toMatch(/^gate_playback_2026-10-27-08-5\d-\d\d\.png$/)
  const video = page.locator('#video')
  await video.evaluate(async (element: HTMLVideoElement) => {
    element.currentTime = 0.05
    await element.play()
  })
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press(' ')
  await expect(video).toHaveJSProperty('paused', true)
  await expect(page.getByTestId('pb-pause')).toHaveAttribute('aria-pressed', 'true')
  await video.evaluate((element: HTMLVideoElement) => (element.currentTime = 0.4))
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.currentTime)).toBeCloseTo(0.4, 1)
  await page.keyboard.press('ArrowRight')
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.currentTime)).toBeGreaterThan(0.43)
  expect(openSockets(live, sockets)).toEqual({ live: 0, playback: 1 })
})

test('a live route during playback keeps playing the recording and opens no live stream beside it', async ({ page }) => {
  await prepare(page)
  const live = await trackLive(page)
  const sockets = await fakePlayback(page)
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await clickAt(page, '08:52:00')
  await expect(page.getByTestId('pb-snapshot')).toBeEnabled()
  expect(openSockets(live, sockets)).toEqual({ live: 0, playback: 1 })
  await page.evaluate(() => {
    history.replaceState(null, '', '#/live/single')
    location.hash = '#/live'
  })
  await page.waitForTimeout(1500)
  expect(openSockets(live, sockets)).toEqual({ live: 0, playback: 1 })
  await expect(page.getByTestId('playback-controls')).toBeVisible()
  await expect(page.getByTestId('live-plate')).toHaveAttribute('data-tone', 'rec')
  await page.getByTestId('tl-back-live').click()
  await expect.poll(() => sockets.closed).toBe(1)
  await playing(page)
  await expect.poll(() => openSockets(live, sockets)).toEqual({ live: 1, playback: 0 })
  await expect(page.getByTestId('live-plate')).toHaveAttribute('data-tone', 'live')
})

test('switching camera and clicking the track while the new stream loads plays that camera once the stream settles', async ({ page }) => {
  await prepare(page)
  const live = await trackLive(page)
  const sockets = await fakePlayback(page)
  await signIn(page, 'many')
  await playing(page)
  await expect(page.getByTestId('tl-bar')).toHaveCount(3)
  await page.keyboard.press('2')
  await clickAt(page, '08:52:00')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await expect(page.getByTestId('playback-controls')).toBeVisible()
  await expect.poll(() => sockets.urls.length).toBe(1)
  expect(param(sockets.urls[0], 'device')).toBe('porch')
  await expect(page.getByTestId('pb-snapshot')).toBeEnabled()
  await page.waitForTimeout(1500)
  expect(openSockets(live, sockets)).toEqual({ live: 0, playback: 1 })
  await expect(page.getByTestId('playback-controls')).toBeVisible()
})

test('Play on an event of another camera switches camera, then plays 5 s before it with one socket', async ({ page }) => {
  await prepare(page)
  const live = await trackLive(page)
  const sockets = await fakePlayback(page)
  await signIn(page, 'many')
  await playing(page)
  await page.evaluate(() => (location.hash = '#/events'))
  await expect(page.getByTestId('alarm-center')).toHaveAttribute('data-mode', 'page')
  await page.getByTestId('alarm-camera').click()
  await page.getByRole('option', { name: 'Porch', exact: true }).click()
  await expect(page.getByTestId('alarm-row')).toHaveCount(2)
  await page.getByTestId('alarm-row').filter({ hasText: '08:40' }).locator('.ev-hit').click()
  await expect(page.getByTestId('alarm-play')).toBeEnabled()
  await page.getByTestId('alarm-play').click()
  await expect(page).toHaveURL(/#\/live$/)
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await expect.poll(() => sockets.urls.length).toBe(1)
  expect(param(sockets.urls[0], 'device')).toBe('porch')
  expect(param(sockets.urls[0], 'begin')).toBe('2026-10-27 08:39:55')
  await expect(page.getByTestId('pb-snapshot')).toBeEnabled()
  await page.waitForTimeout(1500)
  expect(openSockets(live, sockets)).toEqual({ live: 0, playback: 1 })
  await expect(page.getByTestId('playback-controls')).toBeVisible()
})

test('leaving the recordings opened from the grid puts #/live/grid back', async ({ page }) => {
  await prepare(page)
  await signIn(page, 'many')
  await playing(page)
  await page.getByTestId('view-grid').click()
  await expect(page).toHaveURL(/#\/live\/grid$/)
  await page.evaluate(() => (location.hash = '/live/playback'))
  const list = page.getByTestId('playback')
  await expect(list).toBeVisible()
  await expect(page).toHaveURL(/#\/live\/grid$/)
  await page.keyboard.press('Escape')
  await expect(list).toBeHidden()
  await expect(page).toHaveURL(/#\/live\/grid$/)
  await expect(page.getByTestId('grid')).toBeVisible()
})

for (const size of [{ width: 1024, height: 768 }, { width: 768, height: 1024 }, { width: 390, height: 844 }]) {
  test('the reasons for disabled playback controls are on screen at ' + size.width + 'x' + size.height, async ({ page }) => {
    await page.setViewportSize(size)
    await prepare(page)
    await fakePlayback(page, false)
    await signIn(page, 'one')
    await playing(page)
    await expect(page.getByTestId('tl-bar')).toHaveCount(3)
    await clickAt(page, '08:52:00')
    await expect(page.getByTestId('playback-controls')).toBeVisible()
    await expect(page.getByTestId('pb-snapshot')).toBeDisabled()
    await expect(page.getByTestId('pb-snapshot-note')).toBeVisible()
    await expect(page.getByTestId('pb-snapshot-note')).toHaveText(/Snapshot and frame step wait for the first picture/)
    const speed = size.width > 880 ? page.getByTestId('pb-speed-2') : page.getByTestId('pb-speed-cycle')
    await speed.click()
    await expect(page.getByTestId('pb-sound')).toBeDisabled()
    await expect(page.getByTestId('pb-sound-note')).toBeVisible()
    await expect(page.getByTestId('pb-sound-note')).toHaveText('Sound plays at 1x only.')
  })
}
