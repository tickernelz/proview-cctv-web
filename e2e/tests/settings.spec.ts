import { test, expect, type Page } from '@playwright/test'
import { signIn } from './app'

test('a field that needs a restart is saved and the bar asks for the restart', async ({ page }) => {
  await signIn(page, 'settings')
  await page.getByTestId('open-settings').click()
  const settings = page.getByTestId('settings')
  await settings.getByRole('link', { name: 'Configuration', exact: true }).click()
  await expect(page.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
  const field = page.getByTestId('setting-PROVIEW_LISTEN')
  await expect(field).toBeVisible()
  const listen = '127.0.0.1:' + (9700 + Math.floor(Math.random() * 200))
  await field.fill(listen)
  await expect(page.getByTestId('settings-save')).toBeVisible()
  await page.getByTestId('settings-save').click()
  await expect(page.getByTestId('save-state')).toContainText('Restart the server process to apply 1 field')
  await expect(page.getByTestId('settings-restart')).toBeHidden()
  await page.reload()
  await expect(settings).toBeVisible()
  await expect(page.getByTestId('setting-PROVIEW_LISTEN')).toHaveValue(listen)
})

test('the desktop build restarts through its local route after the save', async ({ page }) => {
  const restarts: string[] = []
  await page.addInitScript(() => {
    ;(window as unknown as { __PROVIEW_DESKTOP__: { token: string } }).__PROVIEW_DESKTOP__ = { token: 'e2e-desktop-token' }
  })
  await page.route('**/api/desktop/restart', (route) => {
    const request = route.request()
    restarts.push(request.method() + ' ' + (request.headers()['x-proview-desktop-token'] ?? ''))
    return route.fulfill({ status: 202, json: { ok: true } })
  })
  await signIn(page, 'desktop')
  await page.getByTestId('open-settings').click()
  await page.getByTestId('settings').getByRole('link', { name: 'Configuration', exact: true }).click()
  await page.getByTestId('setting-PROVIEW_LISTEN').fill('127.0.0.1:' + (9900 + Math.floor(Math.random() * 90)))
  await page.getByTestId('settings-save').click()
  await expect(page.getByTestId('save-state')).toContainText('Restart the app to apply 1 field')
  await page.getByTestId('settings-restart').click()
  await expect(page.getByTestId('settings-restart')).toHaveText('Restarting…')
  await expect.poll(() => restarts).toEqual(['POST e2e-desktop-token'])
})
