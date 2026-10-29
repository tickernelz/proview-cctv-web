import { test, expect } from '@playwright/test'
import { ACCOUNT, scenarioUrl, signIn } from './app'

test('signing in with the fixture account opens Live on the first camera', async ({ page }) => {
  await page.goto(scenarioUrl('one'))
  await expect(page.getByTestId('signin')).toBeVisible()
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill(ACCOUNT.password)
  await page.getByTestId('signin-submit').click()
  await expect(page.getByTestId('live')).toBeVisible()
  await expect(page.getByTestId('signin')).toBeHidden()
  await expect(page.getByTestId('camera-name')).toHaveText('Gate camera')
})

test('a wrong password answers 401 and keeps the form with the reason', async ({ page }) => {
  await page.goto(scenarioUrl('one'))
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill('not-the-password')
  const [reply] = await Promise.all([page.waitForResponse('**/api/login'), page.getByTestId('signin-submit').click()])
  expect(reply.status()).toBe(401)
  await expect(page.getByTestId('signin-error')).toBeVisible()
  await expect(page.getByTestId('signin-error')).toContainText('were not accepted')
  await expect(page.getByTestId('live')).toBeHidden()
})

test('a session the server no longer knows sends the viewer back to sign in to resume', async ({ page }) => {
  await signIn(page, 'many')
  await expect(page.getByTestId('live')).toBeVisible()
  await page.evaluate(() =>
    fetch('/api/logout', { method: 'POST', credentials: 'same-origin' }).then((reply) => reply.status),
  )
  await page.getByTestId('camera-switcher').click()
  await page.getByTestId('camera-menu').getByRole('menuitem', { name: 'Refresh the list', exact: true }).click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(page.getByTestId('signin-notice')).toBeVisible()
  await expect(page.getByTestId('signin-submit')).toHaveText('Sign in to resume')
})

test('the sign-in page puts the cursor in Email, so typing starts at once', async ({ page }) => {
  await page.goto(scenarioUrl('one'))
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(page.getByTestId('signin-email')).toBeFocused()
})

test('a wrong password is an alert that keeps the password selected, without repeating the server words', async ({ page }) => {
  await page.goto(scenarioUrl('one'))
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill('not-the-password')
  await page.getByTestId('signin-submit').click()
  const error = page.getByTestId('signin-error')
  await expect(error).toHaveText('That email and password were not accepted.')
  await expect(error).toHaveAttribute('role', 'alert')
  const password = page.getByTestId('signin-password')
  await expect(password).toBeFocused()
  await expect(password).toHaveAttribute('aria-invalid', 'true')
  await expect(password).toHaveAttribute('aria-describedby', 'login-error')
})

test('Sign in says it is working, keeps focus, and a second press sends nothing more', async ({ page }) => {
  let logins = 0
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route('**/api/login', async (route) => {
    logins += 1
    await held
    await route.continue()
  })
  await page.goto(scenarioUrl('one'))
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill(ACCOUNT.password)
  const submit = page.getByTestId('signin-submit')
  await submit.focus()
  await page.keyboard.press('Enter')
  await expect(submit).toHaveText('Signing in…')
  await expect(submit).toHaveAttribute('aria-busy', 'true')
  await expect(submit).toBeFocused()
  await page.keyboard.press('Enter')
  release()
  await expect(page.getByTestId('live')).toBeVisible()
  expect(logins).toBe(1)
})

test('This network on a cloud server says to use Account instead of showing a dead form, and a failed probe is an alert', async ({ page }) => {
  await page.route('**/api/device/probe', (route) =>
    route.fulfill({ status: 409, json: { code: 'device_host_missing', message: 'no camera address' } }),
  )
  await page.goto(scenarioUrl('one'))
  await page.getByRole('tab', { name: 'This network', exact: true }).click()
  const panel = page.getByRole('tabpanel', { name: 'This network', exact: true })
  await expect(panel.locator('#local-mode')).toContainText('Use Account')
  await expect(panel.locator('input')).toHaveCount(0)
  await page.getByTestId('signin-probe').click()
  const error = page.getByTestId('signin-probe-error')
  await expect(error).toHaveAttribute('role', 'alert')
  await expect(error).toContainText('no camera address yet')
  await expect(panel.locator('#local-mode')).toContainText('Use Account')
})

test('Remember is a switch, and with it on a sign-in asks the server to remember the account', async ({ page }) => {
  let remembered = 0
  await page.route('**/api/account/remember', (route) => {
    remembered += 1
    return route.fulfill({ json: { ok: true, email: ACCOUNT.email, restored: true, message: 'ok' } })
  })
  await page.goto(scenarioUrl('one'))
  const remember = page.getByRole('switch', { name: 'Remember this account in this browser' })
  await expect(remember).not.toBeChecked()
  await remember.check()
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill(ACCOUNT.password)
  await page.getByTestId('signin-submit').click()
  await expect(page.getByTestId('live')).toBeVisible()
  expect(remembered).toBe(1)
})

test('Open Settings before signing in is a button, not a link in running text', async ({ page }) => {
  await page.goto(scenarioUrl('one'))
  const open = page.getByTestId('signin-settings')
  await expect(open).toHaveClass(/\bbtn\b/)
  await open.click()
  await expect(page.getByTestId('settings')).toBeVisible()
})
