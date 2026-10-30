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

const settled = (page: Page) => page.evaluate(() => fetch('/api/health').then((reply) => reply.status))

const serialOf = (url: string) => new URL(url).searchParams.get('sn')

async function openAlarms(page: Page) {
  await page.getByTestId('alarm-bell').click()
  const centre = page.getByTestId('alarm-center')
  await expect(centre).toBeVisible()
  return centre
}

test('the alarm centre lists every alarm and the bell badge counts them', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  const bell = page.getByTestId('alarm-bell')
  await expect(bell).toBeVisible()
  const centre = await openAlarms(page)
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  await expect(centre.getByTestId('alarm-row').first()).toHaveText('human detect10-27 21:02')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
  await expect(page.getByTestId('alarm-status')).toContainText('3 alarms · updated ')
  await centre.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(centre).toBeHidden()
})

test('the badge shows the alarm count right after sign-in, without opening the centre', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3', { timeout: 5_000 })
  await expect(page.getByTestId('alarm-center')).toBeHidden()
})

test('an empty alarm window keeps the badge off and names the camera and window', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: EMPTY }))
  await signIn(page, 'one')
  const centre = await openAlarms(page)
  await expect(page.getByTestId('alarm-empty')).toHaveText('No alarms for Gate camera in the last 24 hours.')
  await expect(page.getByTestId('alarm-status')).toContainText('0 alarms')
  await expect(centre.getByTestId('alarm-row')).toHaveCount(0)
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
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

test('Notify me without notification support says so on the button, not as an alarm error', async ({ page }) => {
  await page.addInitScript(() => {
    delete (window as unknown as { Notification?: unknown }).Notification
  })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await openAlarms(page)
  await expect(page.getByTestId('alarm-status')).toContainText('3 alarms')
  await page.getByTestId('alarm-notify').click()
  await expect(page.getByTestId('alarm-notify')).toHaveText('Notifications unavailable')
  await expect(page.getByTestId('alarm-error')).toHaveCount(0)
  await expect(page.getByTestId('alarm-status')).toContainText('3 alarms')
})

test('the centre takes focus, and Escape or Close hands it back to the bell', async ({ page }) => {
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

test('a failed check is an error with the server reason, not a grey footnote', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ status: 502, json: { message: 'the vendor alarm centre did not answer' } }))
  await signIn(page, 'one')
  await openAlarms(page)
  const error = page.getByTestId('alarm-error')
  await expect(error).toHaveAttribute('role', 'alert')
  await expect(error).toHaveText('The vendor alarm centre did not answer')
  const colours = await error.evaluate((node) => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--bad)'
    document.body.append(probe)
    const bad = getComputedStyle(probe).color
    probe.remove()
    return { text: getComputedStyle(node).color, bad }
  })
  expect(colours.text).toBe(colours.bad)
  await expect(page.getByTestId('alarm-status')).toHaveText('Not checked')
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
  await expect(page.getByTestId('alarm-loading')).toHaveText('Loading alarms for Gate camera…')
  await page.getByTestId('alarm-camera').selectOption('porch')
  await expect(page.getByTestId('alarm-empty')).toHaveText('No alarms for Porch in the last 24 hours.')
  const answered = page.waitForResponse((response) => serialOf(response.url()) === 'gate')
  release()
  await (await answered).finished()
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  await expect(page.getByTestId('alarm-row')).toHaveCount(0)
  await expect(page.getByTestId('alarm-empty')).toHaveText('No alarms for Porch in the last 24 hours.')
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
  expect(asked).toContain('porch')
})

test('only one corner sheet is open: the Events overlay and Cloud replace each other', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 768 })
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await playing(page)
  const recordings = page.getByTestId('recordings-panel')
  await page.getByTestId('recordings-launcher').click()
  await expect(recordings).toBeVisible()
  const centre = await openAlarms(page)
  await expect(recordings).toBeHidden()
  await expect(page.getByTestId('recordings-launcher')).toHaveAttribute('aria-pressed', 'false')
  await page.getByTestId('recordings-launcher').click()
  await expect(recordings).toBeVisible()
  await expect(centre).toBeHidden()
  await expect(page.getByTestId('alarm-bell')).toHaveAttribute('aria-expanded', 'false')
})

test('the alarm centre reuses the camera list the app loaded', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  let devices = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') devices += 1
  })
  await signIn(page, 'many')
  await playing(page)
  await openAlarms(page)
  await expect(page.getByTestId('alarm-camera').locator('option')).toHaveCount(5)
  expect(devices).toBe(1)
})

test('Notify me asks for permission and says when notifications are on', async ({ page, context, browserName }) => {
  test.skip(browserName !== 'chromium', 'only Chromium grants the notifications permission in Playwright')
  await context.grantPermissions(['notifications'])
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  await openAlarms(page)
  const notify = page.getByTestId('alarm-center').getByRole('button', { name: 'Notify me', exact: true })
  await notify.click()
  await expect(page.getByTestId('alarm-center').getByRole('button', { name: 'Notifications on', exact: true })).toBeVisible()
})
