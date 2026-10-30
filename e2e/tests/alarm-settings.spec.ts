import { test, expect, type Page, type Route } from '@playwright/test'
import { signIn } from './app'

const SLOTS = (first: string) => Array.from({ length: 7 }, (_, day) => [first, day === 3 ? '0 07:00-08:00' : '0 00:00:00-24:00:00'])

const motion = {
  enable: true,
  level: 3,
  sensitivity: null,
  record: true,
  snapshot: true,
  push: true,
  voice: false,
  beep: false,
  alarm_out: false,
  tip: false,
  voice_type: 24,
  schedule: SLOTS('1 00:00:00-24:00:00'),
  show_track: null,
  rule: null,
  duration: null,
  remote_call: null,
  push_interval: null,
  push_server: null,
}

const empty = Object.fromEntries(Object.keys(motion).map((key) => [key, null]))

const OVERVIEW = {
  ok: true,
  overview: {
    detectors: [
      { id: 'motion', label: 'Motion detection', config: 'Detect.MotionDetect.[0]', gate: { supported: true, evidence: 'base ability' }, state: motion, error: null },
      {
        id: 'human',
        label: 'Human detection',
        config: 'Detect.HumanDetection.[0]',
        gate: { supported: true, evidence: 'device reports AlarmFunction.PEAInHumanPed=true' },
        state: {
          ...empty,
          enable: true,
          sensitivity: 1,
          show_track: true,
          rule: { rule_type: 1, enabled: true, area: [[100, 100], [8191, 100], [8191, 8191], [100, 8191]], line: [100, 100, 8191, 8191] },
        },
        error: null,
      },
      {
        id: 'push',
        label: 'Device push service',
        config: 'NetWork.PMS',
        gate: { supported: true, evidence: 'NetWork.PMS is a base ability; the app reads it without a flag' },
        state: { ...empty, enable: true, push_interval: 10, push_server: 'push.example.invalid' },
        error: null,
      },
      { id: 'car', label: 'Vehicle detection', config: 'Detect.CarShapeDetection.[0]', gate: { supported: false, evidence: 'device does not report AlarmFunction.CarShapeDetection' }, state: null, error: null },
    ],
    tones_gate: { supported: true, evidence: 'device reports OtherFunction.SupportAlarmVoiceTips=true' },
    tones: [
      { id: 24, text: 'Bell Alarm' },
      { id: 523, text: 'You have entered the monitoring area' },
    ],
    rule_limit: { area: true, line: true, sensitivity: false, show_track: true },
    combined: { supported: true, evidence: 'device reports AlarmFunction.MotionHumanDection=true' },
    manual_stop: { supported: false, evidence: 'device does not report AlarmFunction.ManuIntellAlertAlarm' },
  },
}

async function fake(page: Page) {
  const writes: { path: string; body: any }[] = []
  await page.route(/\/api\/device\/detect(\?|$)/, (route: Route) => route.fulfill({ json: OVERVIEW }))
  await page.route(/\/api\/device\/detect\/(motion|human|push)(\?|$)/, async (route: Route) => {
    const request = route.request()
    const body = request.postDataJSON()
    writes.push({ path: new URL(request.url()).pathname, body })
    const field = Object.keys(body)[0]
    return route.fulfill({
      json: {
        ok: true,
        report: {
          block: 'Detect.MotionDetect.[0]',
          backup: '/tmp/backup.json',
          fields: [{ field: 'Level', before: 3, requested: body[field], after: body[field], matches: true }],
          written: {},
          read_back: {},
          reverted: false,
          restored: [],
        },
      },
    })
  })
  return writes
}

async function openAlarm(page: Page) {
  await signIn(page, 'one')
  await page.getByTestId('open-settings').click()
  await page.getByTestId('detect-settings-link').click()
  await expect(page.locator('#set-body > section[data-section="alarm"]')).toBeVisible()
  await expect(page.getByTestId('detect-status')).toHaveText('Read from the camera just now.')
}

test('Alarm settings show what the camera offers and name the flag for what it does not', async ({ page }) => {
  await fake(page)
  await openAlarm(page)
  await expect(page.getByTestId('detect-motion')).toBeVisible()
  await expect(page.getByTestId('detect-human')).toContainText('HumanRuleLimit Sensitivity=false')
  await expect(page.getByTestId('detect-missing-car')).toContainText('does not report AlarmFunction.CarShapeDetection')
  await expect(page.getByTestId('detect-stop-button')).toBeDisabled()
  await expect(page.getByTestId('detect-stop')).toContainText('ManuIntellAlertAlarm')
})

test('Saving motion detection sends only the fields that changed', async ({ page }) => {
  const writes = await fake(page)
  await openAlarm(page)
  await page.locator('#detect-motion-level').selectOption('6')
  await page.getByTestId('detect-save-motion').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Motion detection read back after the write and kept.')
  expect(writes).toEqual([{ path: '/api/device/detect/motion', body: { level: 6 } }])
  await page.getByTestId('detect-save-motion').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Nothing changed on motion detection.')
  expect(writes.length).toBe(1)
})

test('A schedule slot travels in the seven-day table and an unreadable slot keeps its raw text', async ({ page }) => {
  const writes = await fake(page)
  await openAlarm(page)
  await page.getByLabel('Monday slot 2', { exact: true }).check()
  await page.getByTestId('detect-save-motion').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Motion detection read back after the write and kept.')
  const sent = writes[0].body
  expect(Object.keys(sent)).toEqual(['schedule'])
  expect(sent.schedule[1][1]).toBe('1 00:00:00-24:00:00')
  expect(sent.schedule[0][1]).toBe('0 00:00:00-24:00:00')
  expect(sent.schedule[3][1]).toBe('0 07:00-08:00')
})

test('The push service offers only the interval and clamps it to the app range', async ({ page }) => {
  const writes = await fake(page)
  await openAlarm(page)
  const push = page.getByTestId('detect-push')
  await expect(push.getByRole('switch')).toHaveCount(0)
  await expect(page.getByTestId('detect-push-outside')).toContainText('holds 10 s')
  await page.getByTestId('detect-save-push').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Nothing changed on device push service.')
  expect(writes).toEqual([])
  await page.getByTestId('detect-push-interval').fill('5')
  await page.getByTestId('detect-save-push').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Device push service read back after the write and kept.')
  expect(writes).toEqual([{ path: '/api/device/detect/push', body: { push_interval: 30 } }])
})

test('The live bar shows Stop alarm only when the camera reports the manual alarm ability', async ({ page }) => {
  const stops: string[] = []
  await page.route(/\/api\/device\/detect\/alarm\/stop/, (route: Route) => {
    if (route.request().method() === 'POST') {
      stops.push(route.request().url())
      return route.fulfill({ json: { ok: true, ret: 100 } })
    }
    return route.fulfill({ json: { ok: true, gate: { supported: true, evidence: 'device reports AlarmFunction.ManuIntellAlertAlarm=true' } } })
  })
  await signIn(page, 'one')
  await expect(page.getByTestId('ctl-alarm-stop')).toBeVisible()
  await page.getByTestId('ctl-alarm-stop').click()
  await expect(page.getByTestId('ctl-alarm-stop-note')).toHaveText('The camera acknowledged the stop.')
  await expect(page.getByTestId('ctl-alarm-stop-note')).toBeVisible()
  expect(stops.length).toBe(1)
})

test('A stop that never reaches the server says so next to the button', async ({ page }) => {
  await page.route(/\/api\/device\/detect\/alarm\/stop/, (route: Route) => {
    if (route.request().method() === 'POST') return route.abort('connectionrefused')
    return route.fulfill({ json: { ok: true, gate: { supported: true, evidence: 'device reports AlarmFunction.ManuIntellAlertAlarm=true' } } })
  })
  await signIn(page, 'one')
  await page.getByTestId('ctl-alarm-stop').click()
  await expect(page.getByTestId('ctl-alarm-stop-note')).toContainText('Stop failed')
  await expect(page.getByTestId('ctl-alarm-stop')).toBeEnabled()
})

test('The live bar keeps Stop alarm hidden when the ability is missing', async ({ page }) => {
  let asked = 0
  await page.route(/\/api\/device\/detect\/alarm\/stop/, (route: Route) => {
    asked += 1
    return route.fulfill({ json: { ok: true, gate: { supported: false, evidence: 'device does not report AlarmFunction.ManuIntellAlertAlarm' } } })
  })
  await signIn(page, 'one')
  await expect.poll(() => asked).toBeGreaterThan(0)
  await expect(page.getByTestId('ctl-alarm-stop')).toHaveCount(0)
})

test('Painting the weekly grid rewrites only that day and refuses more ranges than the camera holds', async ({ page }) => {
  const writes = await fake(page)
  await openAlarm(page)
  const grid = page.getByTestId('detect-motion-schedule')
  await grid.getByRole('button', { name: 'Monday 00:00 to 01:00, detect', exact: true }).focus()
  for (let step = 0; step < 6; step++) await page.keyboard.press('ArrowRight')
  await page.keyboard.press('Space')
  await expect(grid.getByRole('button', { name: 'Monday 06:00 to 07:00, off', exact: true })).toHaveAttribute('aria-pressed', 'false')
  for (let step = 0; step < 6; step++) await page.keyboard.press('ArrowRight')
  await page.keyboard.press('Space')
  await expect(page.getByTestId('detect-motion-schedule-refusal')).toContainText('This camera holds 2 time ranges a day')
  await expect(grid.getByRole('button', { name: 'Monday 12:00 to 13:00, detect', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await page.getByTestId('detect-save-motion').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Motion detection read back after the write and kept.')
  const sent = writes[0].body
  expect(Object.keys(sent)).toEqual(['schedule'])
  expect(sent.schedule[1]).toEqual(['1 00:00:00-06:00:00', '1 07:00:00-24:00:00'])
  expect(sent.schedule[0]).toEqual(['1 00:00:00-24:00:00', '0 00:00:00-24:00:00'])
  expect(sent.schedule[3][1]).toBe('0 07:00-08:00')
})

test('A region corner moves by keyboard, clamps to 8192 and travels in the rule area', async ({ page }) => {
  const writes = await fake(page)
  await page.route(/\/api\/device\/controls\/snapshot/, (route: Route) => route.fulfill({ status: 502, json: { message: 'camera did not answer' } }))
  await openAlarm(page)
  await expect(page.getByTestId('detect-human-picture-note')).toHaveText('No picture to draw on: camera did not answer')
  await page.getByTestId('detect-human-handle-1').focus()
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('Shift+ArrowDown')
  await expect(page.getByTestId('detect-human-handle-1')).toHaveAttribute('aria-label', 'Corner 1: 164, 612')
  await page.getByTestId('detect-human-handle-2').focus()
  await page.keyboard.press('Shift+ArrowRight')
  await page.getByTestId('detect-save-human').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Human detection read back after the write and kept.')
  expect(writes).toEqual([
    {
      path: '/api/device/detect/human',
      body: { rule: { area: [[164, 612], [8192, 100], [8191, 8191], [100, 8191]] } },
    },
  ])
})
