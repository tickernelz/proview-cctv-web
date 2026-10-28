import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { signIn } from './app.ts'

test('a field that needs a restart is saved and the bar asks for the restart', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'settings')
  await screen.getByTestId('open-settings').tap()
  const settings = screen.getByTestId('settings')
  await settings.getByRole('link', 'Configuration').tap()
  await expect(screen.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
  const field = screen.getByTestId('setting-PROVIEW_LISTEN')
  await expect(field).toBeVisible()
  const listen = '127.0.0.1:' + (9700 + Math.floor(Math.random() * 200))
  await field.fill(listen)
  await expect(screen.getByTestId('settings-save')).toBeVisible()
  await screen.getByTestId('settings-save').tap()
  await expect(screen.getByTestId('save-state')).toContainText('Restart the server process to apply 1 field')
  await expect(screen.getByTestId('settings-restart')).toBeHidden()
  await browser.reload()
  await expect(settings).toBeVisible()
  await expect(screen.getByTestId('setting-PROVIEW_LISTEN')).toHaveValue(listen)
})

test('the desktop build restarts through its local route after the save', async ({ app, screen, browser }) => {
  const restarts: string[] = []
  await browser.addInitScript(() => {
    ;(window as unknown as { __PROVIEW_DESKTOP__: { token: string } }).__PROVIEW_DESKTOP__ = { token: 'e2e-desktop-token' }
  })
  await browser.route('**/api/desktop/restart', (route) => {
    const request = route.request
    restarts.push(request.method + ' ' + (request.headers['x-proview-desktop-token'] ?? ''))
    return route.fulfill({ status: 202, json: { ok: true } })
  })
  await signIn(app, browser, 'desktop')
  await screen.getByTestId('open-settings').tap()
  await screen.getByTestId('settings').getByRole('link', 'Configuration').tap()
  await screen.getByTestId('setting-PROVIEW_LISTEN').fill('127.0.0.1:' + (9900 + Math.floor(Math.random() * 90)))
  await screen.getByTestId('settings-save').tap()
  await expect(screen.getByTestId('save-state')).toContainText('Restart the app to apply 1 field')
  await screen.getByTestId('settings-restart').tap()
  await expect(screen.getByTestId('settings-restart')).toHaveText('Restarting…')
  await expect.poll(() => restarts).toEqual(['POST e2e-desktop-token'])
})
