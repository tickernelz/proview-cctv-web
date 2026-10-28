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
