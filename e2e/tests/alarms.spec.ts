import { test, expect, type Page } from '@playwright/test'
import { ACCOUNT, eventsPanelClosed, playing, signIn } from './app'

test.beforeEach(({ page }) => eventsPanelClosed(page))

const ALARMS = {
  serial: 'gate',
  name: 'Gate camera',
  total: 3,
  finished: true,
  latencyMs: 4,
  pollSeconds: 60,
  entries: [0, 1, 2].map((index) => ({
    id: '26102721463' + index,
    event: 'appEventHumanDetectAlarm:2',
    eventName: 'human',
    name: 'human detect',
    time: '2026-10-27 21:0' + index + ':10',
    utcTime: 1793112000 + index,
    channel: '0',
    message: '',
  })),
}

const EMPTY = { ...ALARMS, total: 0, entries: [] }

const PICTURED = {
  ...ALARMS,
  entries: ALARMS.entries.map((entry) => ({ ...entry, picture: { object: '003_gate_' + entry.id + '.jpeg', bucket: 'OSS_pic' } })),
}

const MIXED = {
  ...ALARMS,
  total: 4,
  entries: [
    { id: 'm1', event: 'VideoMotion:1', name: 'video motion', time: '2026-10-27 19:10:05', utcTime: 1793106605, channel: '0' },
    { id: 'a1', event: 'LocalAlarm:0', name: 'local alarm', time: '2026-10-27 14:20:00', utcTime: 1793089200, channel: '0' },
    { id: 'h1', event: 'appEventHumanDetectAlarm:2', name: 'human detect', time: '2026-10-27 09:05:00', utcTime: 1793070300, channel: '0' },
    { id: 'm2', event: 'VideoMotion:1', name: 'video motion', time: '2026-10-27 08:00:00', utcTime: 1793066400, channel: '0' },
  ].map((entry) => ({ ...entry, eventName: '', message: '' })),
}

async function countVendorCalls(page: Page) {
  const calls = { alarms: 0, pictures: 0 }
  await page.route(/\/api\/cloud\/alarms\/picture/, (route) => {
    calls.pictures += 1
    return route.fulfill({ status: 200, contentType: 'image/jpeg', body: Buffer.from([0xff, 0xd8, 0xff, 0xd9]) })
  })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => {
    calls.alarms += 1
    return route.fulfill({ json: PICTURED })
  })
  return calls
}

const PIXEL = Buffer.from(
  '/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q==',
  'base64',
)

const settled = (page: Page) => page.evaluate(() => fetch('/api/health').then((reply) => reply.status))

const serialOf = (url: string) => new URL(url).searchParams.get('sn')

async function openAlarms(page: Page) {
  await page.getByTestId('alarm-bell').click()
  const centre = page.getByTestId('alarm-center')
  await expect(centre).toBeVisible()
  return centre
}

async function openPage(page: Page) {
  await page.evaluate(() => (location.hash = '#/events'))
  const centre = page.getByTestId('alarm-center')
  await expect(centre).toHaveAttribute('data-mode', 'page')
  return centre
}

async function pickCamera(page: Page, name: string) {
  await page.getByTestId('alarm-camera').click()
  await page.getByRole('option', { name, exact: true }).click()
}

test('the panel lists every event with its kind and time, and opening it clears the unread badge', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  const bell = page.getByTestId('alarm-bell')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
  await expect(bell).toHaveAccessibleName('Events, 3 new')
  const centre = await openAlarms(page)
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  const first = centre.getByTestId('alarm-row').first()
  await expect(first).toContainText('Human')
  await expect(first).toContainText('21:02')
  await expect(first.getByTestId('alarm-new')).toBeVisible()
  await expect(page.getByTestId('alarm-status')).toHaveText('3 events today')
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
  await expect(bell).toHaveAccessibleName('Events')
  await centre.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(centre).toBeHidden()
  await page.reload()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect.poll(() => page.evaluate(() => localStorage.getItem('pvw_events_seen'))).toContain('gate')
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
})

test('the badge shows the unread count right after sign-in, without opening the panel', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3', { timeout: 5_000 })
  await expect(page.getByTestId('alarm-center')).toBeHidden()
})

test('an empty day keeps the badge off and says so in words', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: EMPTY }))
  await signIn(page, 'one')
  const centre = await openAlarms(page)
  await expect(page.getByTestId('alarm-empty')).toHaveText('No events today')
  await expect(page.getByTestId('alarm-empty-block')).toContainText('Gate camera reported no motion or alarms today.')
  await expect(page.getByTestId('alarm-status')).toHaveText('0 events today')
  await expect(centre.getByTestId('alarm-row')).toHaveCount(0)
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
})

test('the panel asks for today by default and the day picker changes the window', async ({ page }) => {
  const asked: { start: string; end: string }[] = []
  await page.route(/\/api\/cloud\/alarms\?/, (route) => {
    const url = new URL(route.request().url())
    asked.push({ start: url.searchParams.get('start') || '', end: url.searchParams.get('end') || '' })
    return route.fulfill({ json: ALARMS })
  })
  await page.route(/\/api\/cloud\/alarms\/calendar/, (route) => {
    const month = new URL(route.request().url()).searchParams.get('month') || ''
    return route.fulfill({ json: { serial: 'gate', month, days: [{ day: month + '-12', count: 4 }], latencyMs: 1 } })
  })
  await signIn(page, 'one')
  await openAlarms(page)
  await expect(page.getByTestId('alarm-row')).toHaveCount(3)
  const today = await page.evaluate(() => {
    const now = new Date()
    return now.getFullYear() + '-' + String(now.getMonth() + 1).padStart(2, '0') + '-' + String(now.getDate()).padStart(2, '0')
  })
  expect(asked[0]).toEqual({ start: today + ' 00:00:00', end: today + ' 23:59:59' })
  await page.getByTestId('alarm-day-picker').click()
  const menu = page.getByTestId('alarm-day-menu')
  await expect(menu).toBeVisible()
  const day = menu.locator('[data-testid="alarm-day"]:not([disabled])')
  await expect(day).toHaveCount(1)
  const chosen = (await day.getAttribute('data-day')) || ''
  expect(chosen.endsWith('-12')).toBe(true)
  await day.click()
  await expect(menu).toBeHidden()
  await expect.poll(() => asked.at(-1)).toEqual({ start: chosen + ' 00:00:00', end: chosen + ' 23:59:59' })
  await page.getByTestId('alarm-day-picker').click()
  await page.locator('[data-testid="alarm-range"][data-range="7d"]').click()
  await expect(page.getByTestId('alarm-day-picker')).toHaveText('Last 7 days')
  await expect.poll(() => asked.at(-1)!.end.slice(0, 10)).toBe(today)
  await expect(page.getByTestId('alarm-status')).toHaveText('3 events in the last 7 days')
})

test('kind chips count each kind, ask the server for that kind, and rows are grouped by part of the day', async ({ page }) => {
  const events: string[] = []
  await page.route(/\/api\/cloud\/alarms\?/, (route) => {
    const event = new URL(route.request().url()).searchParams.get('event') || ''
    events.push(event)
    const entries = event ? MIXED.entries.filter((entry) => entry.event.startsWith(event + ':')) : MIXED.entries
    return route.fulfill({ json: { ...MIXED, entries, total: entries.length } })
  })
  await signIn(page, 'one')
  const centre = await openAlarms(page)
  const chip = (kind: string) => centre.locator('[data-testid="alarm-kind"][data-kind="' + kind + '"]')
  await expect(chip('all')).toHaveText('All4')
  await expect(chip('motion')).toHaveText('Motion2')
  await expect(chip('alarm')).toHaveText('Alarm1')
  await expect(chip('human')).toHaveText('Human1')
  await expect(centre.locator('.ev-group')).toHaveText(['Evening', 'Afternoon', 'Morning'])
  expect(events).toEqual([''])
  await chip('motion').click()
  await expect(chip('motion')).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(() => events).toEqual(['', 'VideoMotion'])
  await expect(centre.getByTestId('alarm-row')).toHaveCount(2)
  await expect(centre.locator('.ev-group')).toHaveText(['Evening', 'Morning'])
  await expect(page.getByTestId('alarm-status')).toHaveText('2 motion events today')
  await chip('alarm').click()
  await expect.poll(() => events.at(-1)).toBe('')
  await expect(centre.getByTestId('alarm-row')).toHaveCount(1)
  await chip('all').click()
  await expect(centre.getByTestId('alarm-row')).toHaveCount(4)
  await settled(page)
  expect(events).toEqual(['', 'VideoMotion', ''])
})

test('a page the server cut short shows the total, not per-kind counts from the part it sent', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => {
    const event = new URL(route.request().url()).searchParams.get('event') || ''
    if (event) return route.fulfill({ json: { ...MIXED, entries: MIXED.entries.filter((entry) => entry.event.startsWith(event + ':')), total: 75, finished: false } })
    return route.fulfill({ json: { ...MIXED, total: 180, finished: false } })
  })
  await signIn(page, 'one')
  const centre = await openAlarms(page)
  const chip = (kind: string) => centre.locator('[data-testid="alarm-kind"][data-kind="' + kind + '"]')
  await expect(chip('all')).toHaveText('All180')
  await expect(chip('motion')).toHaveText('Motion')
  await expect(chip('human')).toHaveText('Human')
  await expect(page.getByTestId('alarm-status')).toHaveText('Showing 4 of 180 events today')
  await chip('motion').click()
  await expect(chip('motion')).toHaveText('Motion75')
  await expect(page.getByTestId('alarm-status')).toHaveText('Showing 2 of 75 motion events today')
})

test('the panel kinds come from the server filter list', async ({ page }) => {
  let filters = 0
  await page.route(/\/api\/cloud\/alarms\/filters/, (route) => {
    filters += 1
    return route.fulfill({ json: [{ id: 'VideoMotion', label: 'Motion detection' }] })
  })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: MIXED }))
  await signIn(page, 'one')
  const centre = await openAlarms(page)
  await expect(centre.getByTestId('alarm-row')).toHaveCount(4)
  expect(filters).toBeGreaterThan(0)
  await expect(centre.locator('[data-testid="alarm-kind"][data-kind="human"]')).toHaveCount(0)
  await expect(centre.locator('[data-testid="alarm-kind"][data-kind="alarm"]')).toHaveText('Alarm2')
})

test('going to Settings and back does not ask the vendor again while the last check is fresh', async ({ page }) => {
  const calls = await countVendorCalls(page)
  await signIn(page, 'one')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
  await expect.poll(() => calls.pictures).toBe(1)
  await settled(page)
  expect(calls).toEqual({ alarms: 1, pictures: 1 })
  for (let round = 0; round < 3; round += 1) {
    await page.getByTestId('open-settings').click()
    await expect(page.getByTestId('settings')).toBeVisible()
    await page.getByTestId('settings-back').click()
    await expect(page.getByTestId('live')).toBeVisible()
    await settled(page)
  }
  expect(calls).toEqual({ alarms: 1, pictures: 1 })
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
})

test('signing out and in again checks the new session at once', async ({ page }) => {
  const calls = await countVendorCalls(page)
  await signIn(page, 'one')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
  await page.getByTestId('open-settings').click()
  await page.getByRole('link', { name: 'Connection', exact: true }).click()
  await page.locator('#signout').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill(ACCOUNT.password)
  await page.getByTestId('signin-submit').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect.poll(() => calls.alarms).toBe(2)
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
})

test('Notify me without notification support says so under the switch, not as an events error', async ({ page }) => {
  await page.addInitScript(() => {
    delete (window as unknown as { Notification?: unknown }).Notification
  })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await openPage(page)
  await expect(page.getByTestId('alarm-status')).toHaveText('3 events today')
  const notify = page.getByTestId('alarm-notify')
  await expect(notify).toHaveAccessibleName('Notify me')
  await notify.click()
  await expect(notify).toHaveAttribute('aria-checked', 'false')
  await expect(page.getByTestId('alarm-notify-note')).toHaveText('This browser cannot show notifications.')
  await expect(page.getByTestId('alarm-error')).toHaveCount(0)
  await expect(page.getByTestId('alarm-status')).toHaveText('3 events today')
})

test('the panel takes focus, and Escape or Close hands it back to the bell', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  const bell = page.getByTestId('alarm-bell')
  await expect(bell).toHaveAttribute('aria-expanded', 'false')
  const centre = await openAlarms(page)
  await expect(bell).toHaveAttribute('aria-expanded', 'true')
  await expect(page.getByTestId('alarm-camera')).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(centre).toBeHidden()
  await expect(bell).toBeFocused()
  await expect(bell).toHaveAttribute('aria-expanded', 'false')
  await openAlarms(page)
  await page.getByTestId('alarm-close').click()
  await expect(centre).toBeHidden()
  await expect(bell).toBeFocused()
})

test('a failed check leads with a plain sentence and keeps the server reason under Details', async ({ page }) => {
  let fail = true
  await page.route(/\/api\/cloud\/alarms\?/, (route) =>
    fail ? route.fulfill({ status: 502, json: { message: 'the vendor alarm centre did not answer' } }) : route.fulfill({ json: ALARMS }),
  )
  await signIn(page, 'one')
  await openAlarms(page)
  const error = page.getByTestId('alarm-error')
  await expect(error).toHaveAttribute('role', 'alert')
  await expect(page.getByTestId('alarm-error-title')).toHaveText('Events could not be loaded')
  await expect(page.getByTestId('alarm-error-detail')).toBeHidden()
  await page.getByTestId('alarm-error-toggle').click()
  await expect(page.getByTestId('alarm-error-detail')).toHaveText('The vendor alarm centre did not answer')
  const colours = await error.locator('.ev-problem-icon').evaluate((node) => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--bad)'
    document.body.append(probe)
    const bad = getComputedStyle(probe).color
    probe.remove()
    return { icon: getComputedStyle(node).color, bad }
  })
  expect(colours.icon).toBe(colours.bad)
  await expect(page.getByTestId('alarm-status')).toHaveText('Not checked')
  fail = false
  await page.getByTestId('alarm-retry').click()
  await expect(page.getByTestId('alarm-row')).toHaveCount(3)
  await expect(error).toHaveCount(0)
})

test('signed in on this network, the error says to sign in with the cloud account', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ status: 401, json: { message: 'no cloud session' } }))
  await signIn(page, 'one')
  await openAlarms(page)
  await expect(page.getByTestId('alarm-error-title')).toHaveText('Sign in with the cloud account to see events')
})

test('switching camera while a check is out shows the new camera, never the old answer', async ({ page }) => {
  const asked: string[] = []
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route(/\/api\/cloud\/alarms\?/, async (route) => {
    const serial = serialOf(route.request().url())
    asked.push(serial || '')
    if (serial === 'gate') {
      await held
      await route.fulfill({ json: ALARMS })
      return
    }
    await route.fulfill({ json: { ...EMPTY, serial } })
  })
  await signIn(page, 'many')
  await playing(page)
  await openAlarms(page)
  await expect(page.getByTestId('alarm-loading')).toHaveText('Loading events for Gate camera…')
  await pickCamera(page, 'Porch')
  await expect(page.getByTestId('alarm-empty-block')).toContainText('Porch reported no motion or alarms today.')
  const answered = page.waitForResponse((response) => serialOf(response.url()) === 'gate')
  release()
  await (await answered).finished()
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  await expect(page.getByTestId('alarm-row')).toHaveCount(0)
  await expect(page.getByTestId('alarm-empty-block')).toContainText('Porch reported no motion or alarms today.')
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
  expect(asked).toContain('porch')
})

test('only one corner sheet is open: the Events overlay and Cloud replace each other', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 768 })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await playing(page)
  const recordings = page.getByTestId('recordings-panel')
  const cloud = async () => {
    await page.getByTestId('ctl-more').click()
    await page.getByTestId('more-cloud').click()
    await expect(recordings).toBeVisible()
    await expect(page.getByTestId('rec-tab-cloud')).toHaveAttribute('aria-selected', 'true')
  }
  await cloud()
  const centre = await openAlarms(page)
  await expect(centre).toHaveAttribute('role', 'dialog')
  await expect(recordings).toBeHidden()
  await cloud()
  await expect(centre).toBeHidden()
  await expect(page.getByTestId('alarm-bell')).toHaveAttribute('aria-expanded', 'false')
})

test('the panel reuses the camera list the app loaded', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  let devices = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') devices += 1
  })
  await signIn(page, 'many')
  await playing(page)
  await openAlarms(page)
  await page.getByTestId('alarm-camera').click()
  await expect(page.getByRole('option')).toHaveCount(5)
  await page.keyboard.press('Escape')
  expect(devices).toBe(1)
})

test('Open all events goes to the full page, which shows the picture of the chosen event', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\/picture/, (route) => route.fulfill({ status: 200, contentType: 'image/jpeg', body: PIXEL }))
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: PICTURED }))
  await signIn(page, 'one')
  const centre = await openAlarms(page)
  await centre.getByTestId('alarm-open-all').click()
  await expect(page).toHaveURL(/#\/events$/)
  await expect(centre).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('live')).toBeHidden()
  const cards = centre.getByTestId('alarm-row')
  await expect(cards).toHaveCount(3)
  await expect(cards.first()).toHaveAttribute('data-selected', 'true')
  const preview = page.getByTestId('alarm-preview')
  await expect(preview.locator('img')).toHaveAttribute('src', /\/api\/cloud\/alarms\/picture\?sn=gate&object=003_gate_261027214632\.jpeg/)
  await expect(page.getByTestId('alarm-preview-title')).toHaveText('Human')
  await cards.nth(2).locator('.ev-hit').click()
  await expect(cards.nth(2)).toHaveAttribute('data-selected', 'true')
  await expect(preview).toContainText('21:00:10')
  await expect(page.getByTestId('alarm-play')).toBeEnabled()
})

test.describe('in a browser time zone other than the camera account', () => {
  test.use({ timezoneId: 'America/Sao_Paulo' })

  test('Play from 5 s before returns to Live on that camera and seeks to the camera clock time', async ({ page }) => {
    await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: serialOf(route.request().url()) === 'porch' ? { ...ALARMS, serial: 'porch' } : EMPTY }))
    await signIn(page, 'many')
    await playing(page)
    const centre = await openPage(page)
    await pickCamera(page, 'Porch')
    await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
    await expect(page.getByTestId('alarm-preview')).toContainText('Opens Live on the timeline at 21:02:05, 5 s before the event.')
    await page.getByTestId('alarm-play').click()
    await expect(page).toHaveURL(/#\/live$/)
    await expect(page.getByTestId('live')).toBeVisible()
    await expect(page.getByTestId('camera-name')).toHaveText('Porch')
    const seek = await page.evaluate(() => {
      const value = (window as unknown as { ProviewEvents: { seek: () => { serial: string; at: number; id: string } } }).ProviewEvents.seek()
      const start = new Date(value.at - 5000)
      return { ...value, wall: new Date(2026, 9, 27, 21, 2, 10).getTime(), start: [start.getHours(), start.getMinutes(), start.getSeconds()].map((part) => String(part).padStart(2, '0')).join(':'), offset: new Date(value.at).getTimezoneOffset() }
    })
    expect(seek.offset).toBe(180)
    expect(seek.serial).toBe('porch')
    expect(seek.id).toBe('261027214632')
    expect(seek.at).toBe(seek.wall)
    expect(seek.at).not.toBe(ALARMS.entries[2].utcTime * 1000)
    expect(seek.start).toBe('21:02:05')
  })
})

test('a camera outside the list cannot be played, and the reason is on screen', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { ...ALARMS, serial: 'elsewhere' } }))
  await signIn(page, 'one')
  await openPage(page)
  await page.evaluate(() =>
    window.dispatchEvent(new CustomEvent('desktop-alarm', { detail: { id: '261027214631', serial: 'elsewhere', device: 'Shared camera' } })),
  )
  await expect(page.getByTestId('alarm-camera')).toHaveText('Shared camera')
  await expect(page.getByTestId('alarm-play')).toBeDisabled()
  await expect(page.getByTestId('alarm-play-reason')).toHaveText('This camera is not in your camera list, so its recordings cannot be opened.')
})

test('deleting selected events asks first, then sends only the chosen ids', async ({ page }) => {
  const bodies: unknown[] = []
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { ...ALARMS, canDelete: true } }))
  await page.route(/\/api\/cloud\/alarms\/delete/, (route) => {
    bodies.push(route.request().postDataJSON())
    return route.fulfill({ json: { serial: 'gate', all: false, deleted: 1, latencyMs: 2 } })
  })
  await signIn(page, 'one')
  const centre = await openPage(page)
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  await page.getByTestId('alarm-manage').click()
  await expect(page.getByTestId('alarm-delete')).toBeDisabled()
  await expect(page.getByTestId('alarm-delete-reason')).toHaveText('Select events to delete them.')
  await centre.getByTestId('alarm-select').nth(1).check()
  await expect(page.getByTestId('alarm-delete')).toBeEnabled()
  await page.getByTestId('alarm-delete').click()
  const confirm = page.getByTestId('alarm-confirm')
  await expect(confirm).toBeVisible()
  await expect(confirm).toContainText('Delete 1 event?')
  await page.getByTestId('alarm-clear-cancel').click()
  await expect(confirm).toBeHidden()
  expect(bodies).toEqual([])
  await page.getByTestId('alarm-delete').click()
  await page.getByTestId('alarm-clear-confirm').click()
  await expect(confirm).toBeHidden()
  expect(bodies).toEqual([{ sn: 'gate', ids: ['261027214631'], all: false }])
})

test('Clear all stays available on a day with no events, because it clears every event of the camera', async ({ page }) => {
  const bodies: unknown[] = []
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: EMPTY }))
  await page.route(/\/api\/cloud\/alarms\/delete/, (route) => {
    bodies.push(route.request().postDataJSON())
    return route.fulfill({ json: { serial: 'gate', all: true, deleted: 0, latencyMs: 2 } })
  })
  await signIn(page, 'one')
  await openPage(page)
  await expect(page.getByTestId('alarm-empty')).toHaveText('No events today')
  await page.getByTestId('alarm-manage').click()
  await expect(page.getByTestId('alarm-clear')).toBeEnabled()
  await expect(page.getByTestId('alarm-clear-reason')).toHaveCount(0)
  await page.getByTestId('alarm-clear').click()
  await page.getByTestId('alarm-clear-confirm').click()
  await expect(page.getByTestId('alarm-confirm')).toBeHidden()
  expect(bodies).toEqual([{ sn: 'gate', ids: [], all: true }])
})

test('Clear all asks with the camera name and sends all', async ({ page }) => {
  const bodies: unknown[] = []
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: bodies.length ? EMPTY : ALARMS }))
  await page.route(/\/api\/cloud\/alarms\/delete/, (route) => {
    bodies.push(route.request().postDataJSON())
    return route.fulfill({ json: { serial: 'gate', all: true, deleted: 3, latencyMs: 2 } })
  })
  await signIn(page, 'one')
  const centre = await openPage(page)
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  await page.getByTestId('alarm-manage').click()
  await page.getByTestId('alarm-clear').click()
  await expect(page.getByTestId('alarm-confirm')).toContainText('Delete every event of Gate camera?')
  await page.getByTestId('alarm-clear-confirm').click()
  expect(bodies).toEqual([{ sn: 'gate', ids: [], all: true }])
  await expect(page.getByTestId('alarm-empty')).toHaveText('No events today')
})

test('an account that may not delete sees why, and both delete actions stay disabled', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { ...ALARMS, canDelete: false, deleteReason: 'This camera was shared with you, so only its owner can delete alarms.' } }))
  await signIn(page, 'one')
  const centre = await openPage(page)
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  await page.getByTestId('alarm-manage').click()
  await centre.getByTestId('alarm-select').first().check()
  await expect(page.getByTestId('alarm-delete')).toBeDisabled()
  await expect(page.getByTestId('alarm-clear')).toBeDisabled()
  await expect(page.getByTestId('alarm-delete-reason')).toHaveText('This camera was shared with you, so only its owner can delete alarms.')
})

test('the full page shows the phone push state for the camera', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await page.route(/\/api\/cloud\/alarms\/subscription/, (route) => route.fulfill({ json: { serial: 'gate', subscribed: false, count: 2, asked: 0, note: '', latencyMs: 1 } }))
  await signIn(page, 'one')
  await openPage(page)
  await expect(page.getByTestId('alarm-push')).toHaveText('The alarm centre pushes this camera to 2 phones. This app checks every 60 s instead.')
})

test('on a phone the events page is a full-screen list with the kind chips above it', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: MIXED }))
  await signIn(page, 'one')
  await page.getByTestId('tab-events').click()
  const centre = page.getByTestId('alarm-center')
  await expect(centre).toHaveAttribute('data-mode', 'page')
  await expect(centre.getByTestId('alarm-row')).toHaveCount(4)
  const chips = page.getByTestId('alarm-kinds')
  const before = await chips.boundingBox()
  await centre.locator('.ev-page-scroll').evaluate((node) => node.scrollTo(0, node.scrollHeight))
  expect(await chips.boundingBox()).toEqual(before)
  await centre.getByTestId('alarm-row').first().locator('.ev-hit').click()
  const sheet = page.getByTestId('alarm-preview-sheet')
  await expect(sheet).toBeVisible()
  await expect(sheet.getByTestId('alarm-play')).toBeVisible()
})

test('Notify me asks for permission and says when notifications are on', async ({ page, context, browserName }) => {
  test.skip(browserName !== 'chromium', 'only Chromium grants the notifications permission in Playwright')
  await context.grantPermissions(['notifications'])
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await openPage(page)
  const notify = page.getByTestId('alarm-notify')
  await expect(notify).toHaveAttribute('aria-checked', 'false')
  await notify.click()
  await expect(notify).toHaveAttribute('aria-checked', 'true')
  await expect(page.getByTestId('alarm-notify-note')).toHaveText('On. A notification shows when an event arrives while this tab is in the background.')
  expect(await page.evaluate(() => localStorage.getItem('pvw_events_notify'))).toBe('on')
})
