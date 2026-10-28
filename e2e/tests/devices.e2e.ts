import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { signIn } from './app.ts'
import { SCENARIOS } from '../scenarios.mjs'

test('an account without cameras shows the empty state instead of a player', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'empty')
  await expect(screen.getByTestId('devices-empty')).toBeVisible()
  await expect(screen.getByTestId('devices-empty')).toContainText('No cameras')
  await expect(screen.getByTestId('devices-refresh')).toBeEnabled()
})

test('one camera opens straight into Live with nothing to switch to', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'one')
  await expect(screen.getByTestId('camera-name')).toHaveText('Gate camera')
  await expect(screen.getByTestId('devices-empty')).toBeHidden()
  await expect(screen.getByTestId('camera-switcher')).toBeDisabled()
  await expect(screen.getByTestId('live-well')).toBeVisible()
})

test('many cameras are all listed and picking one switches Live to it', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'many')
  await screen.getByTestId('camera-switcher').tap()
  const options = screen.getByTestId('camera-menu').getByTestId('camera-option')
  await expect(options).toHaveCount(SCENARIOS.many.length)
  await expect(options.filter({ hasText: 'Shed' })).toContainText('offline', { ignoreCase: true })
  await options.filter({ hasText: 'Porch' }).tap()
  await expect(screen.getByTestId('camera-name')).toHaveText('Porch')
  await expect(screen.getByTestId('camera-menu')).toBeHidden()
})
