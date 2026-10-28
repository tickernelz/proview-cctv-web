import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { signIn } from './app.ts'

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

test('the alarm centre lists every alarm and the bell badge counts them', async ({ app, screen, browser }) => {
  await browser.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await signIn(app, browser, 'one')
  const bell = screen.getByTestId('alarm-bell')
  await expect(bell).toBeVisible()
  await bell.tap()
  const centre = screen.getByTestId('alarm-center')
  await expect(centre).toBeVisible()
  await expect(centre.getByTestId('alarm-row')).toHaveCount(3)
  await expect(centre.getByTestId('alarm-row').first()).toContainText('10-27 21:02')
  await expect(screen.getByTestId('alarm-badge')).toHaveText('3')
  await centre.getByRole('button', 'Close').tap()
  await expect(centre).toBeHidden()
})

test('an empty alarm window keeps the badge off and says so', async ({ app, screen, browser }) => {
  await browser.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { ...ALARMS, total: 0, entries: [] } }))
  await signIn(app, browser, 'one')
  await screen.getByTestId('alarm-bell').tap()
  await expect(screen.getByTestId('alarm-status')).toContainText('0 alarms')
  await expect(screen.getByTestId('alarm-center').getByTestId('alarm-row')).toHaveCount(0)
  await expect(screen.getByTestId('alarm-badge')).toBeHidden()
})
