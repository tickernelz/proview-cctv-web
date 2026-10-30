import { test, expect, type Page, type Route } from '@playwright/test'
import { playing, signIn } from './app'

const ABOUT = {
  ok: true,
  summary: { model: 'MODEL-ONE', serial: 'serial-one', firmware: 'V5.01.R02.ABCDEF01.10010.123456.0000000', build_time: '2024-11-01 19:21:23', hardware: 'HW-ONE', uptime_minutes: 1500 },
  info: {},
  info_ex: { McuVersion: '0255' },
  info_ex_error: null,
  abilities: ['NetServerFunction/WifiRouteSignalLevel'],
  gates: { cloud_upgrade: false, wifi_signal: true, mcu_version: false },
  elapsed_ms: 10,
}

const TIME = {
  ok: true,
  device_time: '2026-10-30 11:03:13',
  minutes_east: 420,
  time_zone: { FirstUserTimeZone: 0, timeMin: -420 },
  location: { DSTRule: 'Off', DateFormat: 'YYMMDD', TimeFormat: '24' },
  backup: null,
  before: null,
  elapsed_ms: 10,
}

const LANGUAGE = { ok: true, current: 'English', available: ['English', 'Spanish'], before: null, backup: null, elapsed_ms: 10 }
const USERS = { ok: true, users: [{ name: 'admin', group: 'admin', memo: null, reserved: true, sharable: true, authorities: 25 }], recovery: { Question: 2 }, recovery_error: null, elapsed_ms: 10 }
const FIRMWARE = {
  ok: true,
  firmware: ABOUT.summary.firmware,
  build_date: '2024-11-01',
  dev_id: 'ABCDEF01100101234560000000',
  device_check: null,
  device_check_error: 'the device does not report OtherFunction/SupportCfgCloudupgrade',
  server_check: { http_status: 204, up_to_date: true, offers: [], detail: 'the upgrade server offers no newer firmware for this build (HTTP 204)' },
  server_check_error: null,
  elapsed_ms: 10,
}

async function fakeSystem(page: Page) {
  const posts: string[] = []
  await page.route(/\/api\/device\/system/, async (route: Route) => {
    const request = route.request()
    const url = new URL(request.url())
    const path = url.pathname.replace('/api/device/system', '')
    if (request.method() !== 'GET') {
      posts.push(request.method() + ' ' + path + '?' + url.searchParams.toString())
      if (path === '/defaults') return route.fulfill({ json: { ok: true, action: 'defaults', detail: 'the camera restored its factory defaults (Ret 100); restart it to apply them', elapsed_ms: 5 } })
      if (path === '/language') return route.fulfill({ json: { ...LANGUAGE, current: url.searchParams.get('value'), before: 'English' } })
      return route.fulfill({ status: 500, json: { ok: false, message: 'unexpected ' + path } })
    }
    if (path === '') return route.fulfill({ json: ABOUT })
    if (path === '/time') return route.fulfill({ json: TIME })
    if (path === '/language') return route.fulfill({ json: LANGUAGE })
    if (path === '/users') return route.fulfill({ json: USERS })
    if (path === '/firmware') return route.fulfill({ json: FIRMWARE })
    return route.fulfill({ status: 404, json: { ok: false, message: 'no such route' } })
  })
  return posts
}

async function openSystem(page: Page) {
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('open-settings').click()
  await page.getByTestId('system-link').click()
  await expect(page.locator('#set-body > section[data-section="system"]')).toBeVisible()
  await expect(page.getByTestId('system-status')).toHaveText('Read from the camera just now.')
}

test('About device shows identity, clock, language and users read from the camera', async ({ page }) => {
  await fakeSystem(page)
  await openSystem(page)
  const about = page.getByTestId('system-about')
  await expect(about).toContainText('MODEL-ONE')
  await expect(about).toContainText('V5.01.R02.ABCDEF01.10010.123456.0000000')
  await expect(about).toContainText('1 d 1 h')
  await expect(page.getByTestId('system-time')).toContainText('UTC+07:00')
  await expect(page.getByTestId('system-users')).toContainText('admin')
  await expect(page.locator('#system-language-pick')).toHaveValue('English')
  await expect(page.getByTestId('system-save-language')).toBeDisabled()
})

test('Factory reset stays disabled with its reason until RESET is typed', async ({ page }) => {
  const posts = await fakeSystem(page)
  await openSystem(page)
  const reset = page.getByTestId('system-reset')
  await expect(reset).toBeDisabled()
  await expect(page.getByTestId('system-reset-reason')).toHaveText('Type RESET to confirm.')
  await page.locator('#system-reset-word').fill('reset')
  await expect(reset).toBeDisabled()
  await page.locator('#system-reset-word').fill('RESET')
  await expect(reset).toBeEnabled()
  await reset.click()
  await expect(page.getByTestId('system-status')).toContainText('restored its factory defaults')
  expect(posts.filter((line) => line.startsWith('POST /defaults'))).toHaveLength(1)
  expect(posts[0]).toContain('confirm=RESET')
})

test('Install update names why it is off when the server offers nothing', async ({ page }) => {
  await fakeSystem(page)
  await openSystem(page)
  await expect(page.getByTestId('system-start-reason')).toHaveText('Check for an update first.')
  await page.getByTestId('system-check-firmware').click()
  await expect(page.getByTestId('system-firmware-server')).toContainText('no newer firmware')
  await expect(page.getByTestId('system-start-upgrade')).toBeDisabled()
  await expect(page.getByTestId('system-start-reason')).toHaveText('The upgrade server offers no newer firmware for this camera.')
})

test('Saving another language writes it and shows the read-back', async ({ page }) => {
  const posts = await fakeSystem(page)
  await openSystem(page)
  await page.locator('#system-language-pick').selectOption('Spanish')
  await page.getByTestId('system-save-language').click()
  await expect(page.getByTestId('system-status')).toHaveText('The camera language reads back as Spanish.')
  expect(posts.some((line) => line.startsWith('PUT /language') && line.includes('value=Spanish'))).toBe(true)
})
