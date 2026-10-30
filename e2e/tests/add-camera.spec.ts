import { test, expect } from '@playwright/test'
import { ACCOUNT, signIn } from './app'

const CAMERA = { id: 'NEWCAM0001', name: 'Side door', model: 'XM-IPC', online: true, channels: 1 }

test('an empty serial is caught before any request and names what to type', async ({ page }) => {
  let binds = 0
  await page.route('**/api/devices/bind', (route) => {
    binds += 1
    return route.fulfill({ json: { serial: 'x', status: 'bound' } })
  })
  await signIn(page, 'empty')
  const panel = page.getByTestId('add-camera')
  await expect(panel).toBeVisible()
  await expect(page.locator('#switcher-dot')).toHaveCount(0)
  await panel.getByRole('button', { name: 'Add this camera' }).click()
  await expect(page.locator('#add-sn')).toBeFocused()
  await expect(page.locator('#add-sn')).toHaveAttribute('aria-invalid', 'true')
  await expect(page.locator('#add-sn-missing')).toHaveText('Type the serial printed on the camera or in the phone app.')
  expect(binds).toBe(0)
})

test('a refused bind is an alert with the cloud reason, and a second press while adding sends nothing', async ({ page }) => {
  const bodies: unknown[] = []
  let release: () => void = () => {}
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route('**/api/devices/bind', async (route) => {
    bodies.push(route.request().postDataJSON())
    await held
    await route.fulfill({ status: 502, json: { message: 'device already bound to another account', code: 'bind_refused' } })
  })
  await signIn(page, 'empty')
  await page.locator('#add-sn').fill('  ABCDEF0123456789 ')
  await page.locator('#add-pass').fill('camera-secret')
  const submit = page.locator('#add-device-submit')
  await submit.click()
  await expect(submit).toHaveText('Adding…')
  await expect(submit).toHaveAttribute('aria-busy', 'true')
  await submit.click()
  release()
  const result = page.getByTestId('add-camera-result')
  await expect(result).toHaveAttribute('role', 'alert')
  await expect(result).toHaveText('Not added: device already bound to another account')
  await expect(result.locator('use')).toHaveAttribute('href', '#i-warning')
  await expect(submit).toHaveText('Add this camera')
  expect(bodies).toEqual([{ sn: 'ABCDEF0123456789', username: 'admin', password: 'camera-secret' }])
})

test('a code that is not a PROVIEW code says so, and a bound camera opens in Live', async ({ page }) => {
  let bound = false
  await page.route('**/api/devices/bind/qr', (route) => {
    const payload = route.request().postDataJSON().payload
    if (payload === 'hello') return route.fulfill({ status: 400, json: { message: 'no shareInfo in the payload', code: 'bad_qr' } })
    bound = true
    return route.fulfill({ json: { serial: CAMERA.id, status: 'bound' } })
  })
  await page.route('**/api/devices', (route) => (bound ? route.fulfill({ json: [CAMERA] }) : route.fallback()))
  await signIn(page, 'empty')
  await page.locator('#add-qr').fill('hello')
  await page.locator('#add-qr-submit').click()
  await expect(page.getByTestId('add-camera-result')).toHaveText('Not added: that code is not a PROVIEW device code. no shareInfo in the payload')
  await page.locator('#add-qr').fill('https://d.xmeye.net/CSee?shareInfo=abc')
  await page.locator('#add-qr-submit').click()
  await expect(page.getByTestId('camera-name')).toHaveText('Side door')
  await expect(page.getByTestId('devices-empty')).toBeHidden()
  await expect(page.locator('#switcher-dot')).toBeVisible()
})

test('Sign out forgets the remembered token before it ends the session', async ({ page }) => {
  const calls: string[] = []
  let signoutAnswered = false
  let logoutAfterAnswer: boolean | null = null
  page.on('response', (response) => {
    if (new URL(response.url()).pathname === '/api/account/signout') signoutAnswered = true
  })
  await page.route('**/api/account/signout', async (route) => {
    calls.push('/api/account/signout')
    await new Promise((resolve) => setTimeout(resolve, 300))
    await route.fallback()
  })
  await page.route('**/api/logout', async (route) => {
    calls.push('/api/logout')
    logoutAfterAnswer = signoutAnswered
    await route.fallback()
  })
  const base = await signIn(page, 'empty')
  await page.goto(base + '/?signed-in&boot=connection#/settings/connection')
  await page.getByTestId('settings-signout').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect.poll(() => calls).toEqual(['/api/account/signout', '/api/logout'])
  expect(logoutAfterAnswer).toBe(true)
})

test('a serial and camera password typed before sign-out are gone for the next account', async ({ page }) => {
  const base = await signIn(page, 'empty')
  await page.goto(base + '/?signed-in&boot=connection#/settings/connection')
  await page.evaluate(() => (location.hash = '#/live'))
  await page.locator('#add-sn').fill('SN-9')
  await page.locator('#add-pass').fill('typed-secret')
  await page.evaluate(() => (location.hash = '#/settings/connection'))
  await page.getByTestId('settings-signout').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await page.getByTestId('signin-email').fill(ACCOUNT.email)
  await page.getByTestId('signin-password').fill(ACCOUNT.password)
  await page.getByTestId('signin-submit').click()
  await expect(page.getByTestId('signin')).toBeHidden()
  await page.evaluate(() => (location.hash = '#/live'))
  await expect(page.getByTestId('devices-empty')).toBeVisible()
  await expect(page.locator('#add-sn')).toHaveValue('')
  await expect(page.locator('#add-pass')).toHaveValue('')
})
