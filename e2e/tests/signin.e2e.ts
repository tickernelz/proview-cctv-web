import { test } from '@e2e-dev/web'
import { expect } from 'e2e'
import { ACCOUNT, signIn } from './app.ts'

test('signing in with the fixture account opens Live on the first camera', async ({ app, screen }) => {
  await app.open('/')
  await expect(screen.getByTestId('signin')).toBeVisible()
  await screen.getByTestId('signin-email').fill(ACCOUNT.email)
  await screen.getByTestId('signin-password').fill(ACCOUNT.password)
  await screen.getByTestId('signin-submit').tap()
  await expect(screen.getByTestId('live')).toBeVisible()
  await expect(screen.getByTestId('signin')).toBeHidden()
  await expect(screen.getByTestId('camera-name')).toHaveText('Gate camera')
})

test('a wrong password answers 401 and keeps the form with the reason', async ({ app, screen, browser }) => {
  await app.open('/')
  await screen.getByTestId('signin-email').fill(ACCOUNT.email)
  await screen.getByTestId('signin-password').fill('not-the-password')
  const [reply] = await Promise.all([browser.waitForResponse('**/api/login'), screen.getByTestId('signin-submit').tap()])
  expect(reply.status).toBe(401)
  await expect(screen.getByTestId('signin-error')).toBeVisible()
  await expect(screen.getByTestId('signin-error')).toContainText('were not accepted')
  await expect(screen.getByTestId('live')).toBeHidden()
})

test('a session the server no longer knows sends the viewer back to sign in to resume', async ({ app, screen, browser }) => {
  await signIn(app, browser, 'many')
  await expect(screen.getByTestId('live')).toBeVisible()
  await browser.evaluate(() =>
    fetch('/api/logout', { method: 'POST', credentials: 'same-origin' }).then((reply) => reply.status),
  )
  await screen.getByTestId('camera-switcher').tap()
  await screen.getByTestId('camera-menu').getByRole('menuitem', 'Refresh the list').tap()
  await expect(screen.getByTestId('signin')).toBeVisible()
  await expect(screen.getByTestId('signin-notice')).toBeVisible()
  await expect(screen.getByTestId('signin-submit')).toHaveText('Sign in to resume')
})
