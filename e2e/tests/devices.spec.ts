import { test, expect } from '@playwright/test'
import { signIn } from './app'
import { SCENARIOS } from '../scenarios.mjs'

test('an account without cameras shows the empty state instead of a player', async ({ page }) => {
  await signIn(page, 'empty')
  await expect(page.getByTestId('devices-empty')).toBeVisible()
  await expect(page.getByTestId('devices-empty')).toContainText('No cameras')
  await expect(page.getByTestId('devices-refresh')).toBeEnabled()
})

test('one camera opens straight into Live with nothing to switch to', async ({ page }) => {
  await signIn(page, 'one')
  await expect(page.getByTestId('camera-name')).toHaveText('Gate camera')
  await expect(page.getByTestId('devices-empty')).toBeHidden()
  await expect(page.getByTestId('camera-switcher')).toBeDisabled()
  await expect(page.getByTestId('live-well')).toBeVisible()
})

test('many cameras are all listed and picking one switches Live to it', async ({ page }) => {
  await signIn(page, 'many')
  await page.getByTestId('camera-switcher').click()
  const options = page.getByTestId('camera-menu').getByTestId('camera-option')
  await expect(options).toHaveCount(SCENARIOS.many.length)
  await expect(options.filter({ hasText: 'Shed' })).toContainText('offline', { ignoreCase: true })
  await options.filter({ hasText: 'Porch' }).click()
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await expect(page.getByTestId('camera-menu')).toBeHidden()
})
