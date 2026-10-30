import { test, expect } from '@playwright/test'
import { overflowFindings } from './layout'
import { playing, signIn } from './app'

test('an offline camera banner says it once and offers Refresh the list, not Retry now', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('5')
  const banner = page.getByTestId('live-banner')
  await expect(banner).toHaveAttribute('role', 'alert')
  await expect(banner).toContainText('Shed is offline according to the cloud.')
  await expect(banner).not.toContainText('cloud status offline')
  const action = page.getByTestId('banner-action')
  await expect(action).toHaveText('Refresh the list')
  await expect(action).toHaveAttribute('data-variant', 'secondary')
  let lists = 0
  page.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/devices') lists += 1
  })
  await action.click()
  await expect.poll(() => lists).toBe(1)
})

test('a lost network offers Retry now and a list that fails to load offers Refresh the list with a warning', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.evaluate(() => window.dispatchEvent(new Event('offline')))
  await expect(page.getByTestId('live-banner')).toContainText('lost its network')
  await expect(page.getByTestId('banner-action')).toHaveText('Retry now')
  await page.evaluate(() => window.dispatchEvent(new Event('online')))
  await expect(page.getByTestId('live-banner')).toHaveCount(0)
  await page.route('**/api/devices', (route) => route.fulfill({ status: 502, json: { message: 'the cloud did not answer' } }))
  await page.getByTestId('camera-switcher').click()
  await page.getByTestId('camera-menu').getByRole('menuitem', { name: 'Refresh the list', exact: true }).click()
  const banner = page.getByTestId('live-banner')
  await expect(banner).toContainText('Could not load the cameras: the cloud did not answer')
  await expect(banner).toHaveAttribute('data-tone', 'list')
  await expect(page.getByTestId('banner-action')).toHaveText('Refresh the list')
})

test('the keyboard list closes from its button and gives focus back to where it was', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.getByTestId('view-single').focus()
  await page.keyboard.press('?')
  await expect(page.getByTestId('keys-sheet')).toBeVisible()
  await expect(page.locator('#keys-close')).toBeFocused()
  await page.locator('#keys-close').click()
  await expect(page.getByTestId('keys-sheet')).toBeHidden()
  await expect(page.getByTestId('view-single')).toBeFocused()
})

test('Tab in the keyboard list closes it and moves on from where focus was', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.getByTestId('open-settings').focus()
  await page.keyboard.press('?')
  await expect(page.locator('#keys-close')).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByTestId('keys-sheet')).toBeHidden()
  await expect(page.getByTestId('app-header').getByTestId('theme-toggle')).toBeFocused()
})

test('a digit typed while the camera menu is open switches to that camera', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.getByTestId('camera-switcher').click()
  await expect(page.getByTestId('camera-menu')).toBeVisible()
  await page.keyboard.press('3')
  await expect(page.getByTestId('camera-name')).toHaveText('Garage')
  await expect(page.getByTestId('camera-menu')).toBeHidden()
})

test('the banner fits from 320 px up without pushing the page sideways', async ({ page }) => {
  await signIn(page, 'many')
  await playing(page)
  await page.keyboard.press('5')
  await expect(page.getByTestId('live-banner')).toBeVisible()
  const findings: string[] = []
  for (const width of [320, 360, 390, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 800 })
    findings.push(...(await overflowFindings(page)).map((line) => width + ' px: ' + line))
  }
  expect(findings).toEqual([])
})
