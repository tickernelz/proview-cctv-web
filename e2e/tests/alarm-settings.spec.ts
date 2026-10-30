import { test, expect, type Page, type Route } from '@playwright/test'
import { signIn } from './app'

const SLOTS = (first: string) => Array.from({ length: 7 }, () => [first, '0 00:00:00-24:00:00'])

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
  region: Array.from({ length: 3 }, () => [true, true, true]),
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
      { id: 'car', label: 'Vehicle detection', config: 'Detect.CarShapeDetection.[0]', gate: { supported: false, evidence: 'device does not report AlarmFunction.CarShapeDetection' }, state: null, error: null },
    ],
    tones_gate: { supported: true, evidence: 'device reports OtherFunction.SupportAlarmVoiceTips=true' },
    tones: [
      { id: 24, text: 'Bell Alarm' },
      { id: 523, text: 'You have entered the monitoring area' },
    ],
    grid: { rows: 3, columns: 3 },
    rule_limit: { area: true, line: true, sensitivity: false, show_track: true },
    combined: { supported: true, evidence: 'device reports AlarmFunction.MotionHumanDection=true' },
    manual_stop: { supported: false, evidence: 'device does not report AlarmFunction.ManuIntellAlertAlarm' },
  },
}

async function fake(page: Page) {
  const writes: { path: string; body: any }[] = []
  await page.route(/\/api\/device\/detect(\?|$)/, (route: Route) => route.fulfill({ json: OVERVIEW }))
  await page.route(/\/api\/device\/detect\/(motion|human)(\?|$)/, async (route: Route) => {
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

test('A grid cell and a schedule slot travel as the full grid and seven-day table', async ({ page }) => {
  const writes = await fake(page)
  await openAlarm(page)
  await page.getByTestId('detect-cell-1-2').click()
  await expect(page.getByTestId('detect-cell-1-2')).toHaveAttribute('aria-pressed', 'false')
  await page.getByLabel('Monday slot 2', { exact: true }).check()
  await page.getByTestId('detect-save-motion').click()
  await expect(page.getByTestId('detect-status')).toHaveText('Motion detection read back after the write and kept.')
  const sent = writes[0].body
  expect(sent.region).toEqual([[true, true, true], [true, true, false], [true, true, true]])
  expect(sent.schedule[1][1]).toBe('1 00:00:00-24:00:00')
  expect(sent.schedule[0][1]).toBe('0 00:00:00-24:00:00')
  expect(Object.keys(sent).sort()).toEqual(['region', 'schedule'])
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
  await expect(page.getByTestId('ctl-alarm-stop')).toHaveAttribute('title', 'The camera acknowledged the stop')
  expect(stops.length).toBe(1)
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
