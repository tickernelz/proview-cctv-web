import { test, expect, type Page } from '@playwright/test'
import { signIn } from './app'

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

test('the alarm centre lists every alarm and the bell badge counts them', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(page, 'one')
  const bell = page.getByTestId('alarm-bell')
  await expect(bell).toBeVisible()
  await bell.click()
  const centre = page.getByTestId('alarm-center')
  await expect(centre).toBeVisible()
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  await expect(centre.getByTestId('alarm-row').first()).toContainText('10-27 21:02')
  await expect(page.getByTestId('alarm-badge')).toHaveText('3')
  await centre.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(centre).toBeHidden()
})

test('an empty alarm window keeps the badge off and says so', async ({ page }) => {
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { ...ALARMS, total: 0, entries: [] } }))
  await signIn(page, 'one')
  await page.getByTestId('alarm-bell').click()
  await expect(page.getByTestId('alarm-status')).toContainText('0 alarms')
  await expect(page.getByTestId('alarm-center').getByTestId('alarm-row')).toHaveCount(0)
  await expect(page.getByTestId('alarm-badge')).toBeHidden()
})
