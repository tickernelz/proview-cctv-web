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
  const listen = (await field.inputValue()).endsWith(':9700') ? '127.0.0.1:9701' : '127.0.0.1:9700'
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
  await expect(page.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
  const field = page.getByTestId('setting-PROVIEW_LISTEN')
  await expect(field).toBeVisible()
  await field.fill((await field.inputValue()).endsWith(':9800') ? '127.0.0.1:9801' : '127.0.0.1:9800')
  await expect(page.getByTestId('settings-save')).toBeVisible()
  await page.getByTestId('settings-save').click()
  await expect(page.getByTestId('save-state')).toContainText('Restart the app to apply 1 field')
  await page.getByTestId('settings-restart').click()
  await expect(page.getByTestId('settings-restart')).toHaveText('Restarting…')
  await expect.poll(() => restarts).toEqual(['POST e2e-desktop-token'])
})

async function openSection(page: Page, name: string) {
  await signIn(page, 'one')
  await page.getByTestId('open-settings').click()
  await page.getByTestId('settings').getByRole('link', { name, exact: true }).click()
}

test('a save the server refuses is an alert in the save bar with its reason, and focus stays on Save', async ({ page }) => {
  await page.route('**/api/settings', (route) =>
    route.request().method() === 'POST'
      ? route.fulfill({ status: 400, json: { message: 'PROVIEW_LISTEN is not a socket address' } })
      : route.continue(),
  )
  await openSection(page, 'Configuration')
  await page.getByTestId('setting-PROVIEW_LISTEN').fill('nope')
  await page.getByTestId('settings-save').click()
  const bar = page.getByTestId('save-state')
  await expect(bar).toHaveText('Not saved: PROVIEW_LISTEN is not a socket address')
  await expect(bar).toHaveAttribute('role', 'alert')
  await expect(page.getByTestId('settings-save')).toBeFocused()
})

test('leaving with unsaved changes is an alert that counts the fields in plain words', async ({ page }) => {
  await openSection(page, 'Configuration')
  await page.getByTestId('setting-PROVIEW_LISTEN').fill('127.0.0.1:9999')
  await page.getByTestId('settings-back').click()
  const bar = page.getByTestId('save-state')
  await expect(bar).toHaveText('Save or discard 1 changed field before leaving.')
  await expect(bar).toHaveAttribute('role', 'alert')
  await expect(page.getByTestId('settings-save')).toBeFocused()
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await expect(bar).toHaveText('Everything saved')
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
})

test('a field that needs a restart carries a Restart tag next to its label', async ({ page }) => {
  await openSection(page, 'Configuration')
  const label = page.locator('label[for="setting-proview-listen"]')
  await expect(label.locator('.tag-restart')).toHaveText('Restart')
  await expect(label).not.toContainText('Needs a restart.')
})

test('Run checks says it is working, keeps focus, and a failed check carries a warning icon and the HTTP status', async ({ page }) => {
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route('**/api/health', async (route) => {
    await held
    await route.continue()
  })
  await openSection(page, 'Server')
  const run = page.getByTestId('run-checks')
  await run.click()
  await expect(run).toHaveAttribute('aria-busy', 'true')
  await expect(run).toHaveText('Checking…')
  release()
  const probe = page.locator('#check-probe')
  await expect(probe).toHaveAttribute('data-ok', 'false')
  await expect(probe).toContainText(/\(HTTP \d{3}\)$/)
  await expect(probe.locator('svg use')).toHaveAttribute('href', '#i-warning')
  await expect(page.locator('#check-health')).toHaveText(/^Up · /)
  await expect(run).toBeFocused()
})

test('a provider switch the server refuses stays on screen as an alert', async ({ page }) => {
  await page.route('**/api/config/mode', (route) =>
    route.request().method() === 'POST' ? route.fulfill({ status: 500, json: { message: 'the provider is busy' } }) : route.continue(),
  )
  await openSection(page, 'Connection')
  await page.getByRole('radio', { name: 'This network', exact: true }).check()
  const error = page.getByTestId('mode-error')
  await expect(error).toHaveAttribute('role', 'alert')
  await expect(error).toContainText('the provider is busy')
  await expect(page.getByRole('radio', { name: 'Cloud account', exact: true })).toBeChecked()
})

test('Settings opens again on the section that was clicked last, from the gear, the comma key and the address', async ({ page }) => {
  await openSection(page, 'Configuration')
  await expect(page.locator('#h-config')).toBeVisible()
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await page.getByTestId('open-settings').click()
  await expect(page.locator('#h-config')).toBeVisible()
  await expect(page.getByTestId('settings').getByRole('link', { name: 'Configuration', exact: true })).toHaveAttribute('aria-current', 'page')
  await page.getByTestId('settings').getByRole('link', { name: 'Server', exact: true }).focus()
  await page.keyboard.press('ArrowUp')
  await expect(page.locator('#h-connection')).toBeVisible()
  await expect(page.getByTestId('settings').getByRole('link', { name: 'Connection', exact: true })).toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(page.locator('#h-server')).toBeVisible()
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await page.keyboard.press(',')
  await expect(page.locator('#h-server')).toBeVisible()
  await expect(page).toHaveURL(/#\/settings\/server$/)
})

test('a refused provider switch is not shown again when Settings opens later', async ({ page }) => {
  await page.route('**/api/config/mode', (route) =>
    route.request().method() === 'POST' ? route.fulfill({ status: 500, json: { message: 'the provider is busy' } }) : route.continue(),
  )
  await openSection(page, 'Connection')
  await page.getByRole('radio', { name: 'This network', exact: true }).check()
  await expect(page.getByTestId('mode-error')).toBeVisible()
  await page.getByTestId('settings-back').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await page.getByTestId('open-settings').click()
  await page.getByTestId('settings').getByRole('link', { name: 'Connection', exact: true }).click()
  await expect(page.locator('#h-connection')).toBeVisible()
  await expect(page.getByTestId('mode-error')).toHaveCount(0)
})

test('the sections sit in three groups and Manage cameras opens the camera list from Settings', async ({ page }) => {
  await page.route('**/api/cloud/device-attributes', (route) => route.fulfill({ json: { devices: [{ uuid: 'gate', nickname: 'Gate camera' }], groups: [] } }))
  await openSection(page, 'Playback')
  const nav = page.getByTestId('settings').getByRole('navigation', { name: 'Settings sections' })
  await expect(nav.locator('.set-nav-g')).toHaveText(['Camera', 'App', 'Account & server'])
  await expect(nav.locator('a[data-section]')).toHaveText([
    'This camera',
    'Video & storage',
    'Image, light & sound',
    'Detection & alarms',
    'System',
    'Playback',
    'Configuration',
    'Account',
    'Connection',
    'Server',
  ])
  await expect(page.getByRole('radio', { name: 'Picture in picture', exact: true })).toBeVisible()
  const manage = page.getByTestId('settings-cameras')
  await manage.click()
  await expect(page.getByTestId('device-admin')).toBeVisible()
  await page.getByTestId('device-admin-close').click()
  await expect(page.getByTestId('device-admin')).toBeHidden()
  await expect(manage).toBeFocused()
})

test('Ctrl+S saves and Escape leaves only when nothing is unsaved', async ({ page }) => {
  await openSection(page, 'Configuration')
  await expect(page.getByTestId('settings-status')).not.toHaveText('Not loaded yet.')
  const field = page.getByTestId('setting-PROVIEW_LISTEN')
  await field.fill('nope')
  await expect(page.getByTestId('save-state')).toHaveText('Unsaved changes · 1 field')
  await page.locator('#h-config').focus()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('settings')).toBeVisible()
  await page.route('**/api/settings', (route) =>
    route.request().method() === 'POST' ? route.fulfill({ status: 400, json: { message: 'PROVIEW_LISTEN is not a socket address' } }) : route.continue(),
  )
  await page.keyboard.press('Control+s')
  await expect(page.getByTestId('save-state')).toHaveText('Not saved: PROVIEW_LISTEN is not a socket address')
  await page.getByRole('button', { name: 'Discard', exact: true }).click()
  await page.locator('#h-config').focus()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('live')).toBeVisible()
})

test('a phone shows the section list first, then the section, and All settings goes back', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await signIn(page, 'one')
  await page.getByTestId('tab-settings').click()
  const nav = page.getByTestId('settings').getByRole('navigation', { name: 'Settings sections' })
  await expect(nav).toBeVisible()
  await expect(page.locator('#set-body')).toBeHidden()
  await nav.getByRole('link', { name: 'Server', exact: true }).click()
  await expect(page.locator('#h-server')).toBeVisible()
  await expect(nav).toBeHidden()
  await page.locator('#set-body > section[data-section="server"]').getByRole('button', { name: 'All settings' }).click()
  await expect(nav).toBeVisible()
  await expect(nav.getByRole('link', { name: 'Server', exact: true })).toBeFocused()
})
