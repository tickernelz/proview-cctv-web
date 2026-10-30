import { test, expect, type Page } from '@playwright/test'
import { ACCOUNT, playing, signIn } from './app'

const ATTRIBUTES = {
  ok: true,
  groups: ['gate'],
  devices: [
    { uuid: 'AASN000000000001', nickname: 'Front gate', group: 'gate', favourite: false, shared_accounts: 1, administrator: true },
    { uuid: '0123456789abcdef0123', nickname: 'Porch', group: '', favourite: true, shared_accounts: 0, administrator: false },
  ],
}

type Call = { path: string; body: any }

async function fakeCloud(page: Page, attributes: unknown = ATTRIBUTES) {
  const calls: Call[] = []
  await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: attributes }))
  await page.route(/\/api\/cloud\/device\//, (route) => {
    const request = route.request()
    calls.push({ path: new URL(request.url()).pathname, body: request.postDataJSON() })
    return route.fulfill({ json: { ok: true, message: 'saved for the test' } })
  })
  return calls
}

async function openSheet(page: Page) {
  await page.getByTestId('device-admin-open').click()
  const sheet = page.getByTestId('device-admin')
  await expect(sheet).toBeVisible()
  return sheet
}

test('Devices lists the cameras, focuses the chosen one, and Escape or Close hands focus back', async ({ page }) => {
  await fakeCloud(page)
  await signIn(page, 'one')
  await playing(page)
  const launcher = page.getByTestId('device-admin-open')
  const sheet = await openSheet(page)
  await expect(launcher).toHaveAttribute('aria-pressed', 'true')
  const rows = sheet.getByTestId('device-admin-row')
  await expect(rows).toHaveCount(2)
  await expect(rows.first()).toHaveAttribute('aria-checked', 'true')
  await expect(rows.first()).toBeFocused()
  await expect(rows.first()).toContainText('743hrc · gate · administrator · 1 shared')
  await page.keyboard.press('ArrowDown')
  await expect(rows.nth(1)).toBeFocused()
  await expect(rows.nth(1)).toHaveAttribute('aria-checked', 'true')
  await expect(page.locator('#device-admin-name')).toHaveValue('Porch')
  await expect(page.locator('#device-admin-favourite')).toBeChecked()
  await page.keyboard.press('Escape')
  await expect(sheet).toBeHidden()
  await expect(launcher).toBeFocused()
  await expect(launcher).toHaveAttribute('aria-pressed', 'false')
  await openSheet(page)
  await page.getByTestId('device-admin-close').click()
  await expect(sheet).toBeHidden()
  await expect(launcher).toBeFocused()
})

test('Rename says it is working, sends one request, and reports the result where it can be seen', async ({ page }) => {
  const calls: Call[] = []
  let release!: () => void
  const held = new Promise<void>((resolve) => (release = resolve))
  await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: ATTRIBUTES }))
  await page.route(/\/api\/cloud\/device\/rename/, async (route) => {
    calls.push({ path: '/rename', body: route.request().postDataJSON() })
    await held
    await route.fulfill({ json: { ok: true } })
  })
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await page.locator('#device-admin-name').fill('Gate, north side')
  const rename = sheet.getByRole('button', { name: 'Rename through the cloud' })
  await rename.click()
  const busy = sheet.getByRole('button', { name: 'Renaming…' })
  await expect(busy).toHaveAttribute('aria-busy', 'true')
  await busy.click()
  release()
  const status = page.getByTestId('device-admin-status')
  await expect(status).toHaveText('Renamed to Gate, north side')
  await expect(status).toHaveAttribute('role', 'status')
  await expect(status).toBeInViewport()
  expect(calls).toEqual([{ path: '/rename', body: { uuid: 'AASN000000000001', nickname: 'Gate, north side' } }])
})

test('a refused action is a red alert with the server reason, visible without scrolling', async ({ page }) => {
  await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: ATTRIBUTES }))
  await page.route(/\/api\/cloud\/device\/rename/, (route) => route.fulfill({ status: 400, json: { message: 'the cloud refused the new name' } }))
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await sheet.getByRole('button', { name: 'Rename through the cloud' }).click()
  const status = page.getByTestId('device-admin-status')
  await expect(status).toHaveText('The cloud refused the new name')
  await expect(status).toHaveAttribute('role', 'alert')
  await expect(status).toBeInViewport()
  const colours = await status.evaluate((node) => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--bad)'
    document.body.append(probe)
    const bad = getComputedStyle(probe).color
    probe.remove()
    return { text: getComputedStyle(node).color, bad }
  })
  expect(colours.text).toBe(colours.bad)
})

test('a camera list that fails to load is an alert with Try again, and only Accept stays', async ({ page }) => {
  let fail = true
  await page.route(/\/api\/cloud\/device-attributes/, (route) =>
    fail ? route.fulfill({ status: 502, json: { message: 'the vendor cloud did not answer' } }) : route.fulfill({ json: ATTRIBUTES }),
  )
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await expect(sheet.getByTestId('device-admin-error')).toHaveText('The vendor cloud did not answer')
  await expect(sheet.getByTestId('device-admin-error')).toHaveAttribute('role', 'alert')
  await expect(page.locator('#device-admin-name')).toHaveCount(0)
  await expect(sheet.getByRole('tab', { name: 'Name', exact: true })).toBeDisabled()
  await expect(page.locator('#device-admin-share-device')).toBeVisible()
  fail = false
  await sheet.getByTestId('device-admin-retry').click()
  await expect(sheet.getByTestId('device-admin-row')).toHaveCount(2)
  await expect(sheet.getByTestId('device-admin-error')).toHaveCount(0)
})

test('an account without cameras says so and can still accept a shared camera', async ({ page }) => {
  const calls: Call[] = []
  await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: { ok: true, groups: [], devices: [] } }))
  await page.route(/\/api\/cloud\/device\/share\/accept/, (route) => {
    calls.push({ path: new URL(route.request().url()).pathname, body: route.request().postDataJSON() })
    return route.fulfill({ json: { ok: true, materialised: true } })
  })
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await expect(sheet.getByTestId('device-admin-empty')).toHaveText('No camera on this account yet.')
  await expect(sheet.getByRole('tab', { name: 'Sharing' })).toHaveAttribute('aria-selected', 'true')
  await expect(sheet.getByRole('tab', { name: 'Name', exact: true })).toBeDisabled()
  await expect(sheet.getByRole('tab', { name: 'Password' })).toBeDisabled()
  await expect(page.locator('#device-admin-accept')).toHaveCount(0)
  await page.locator('#device-admin-share-device').fill('SERIAL9')
  await page.locator('#device-admin-share-pass').fill('account-secret')
  await sheet.getByRole('button', { name: 'Accept', exact: true }).click()
  await expect(page.getByTestId('device-admin-status')).toHaveText("Accepted SERIAL9; it is in this account's device list")
  expect(calls).toEqual([{ path: '/api/cloud/device/share/accept', body: { devId: 'SERIAL9', password: 'account-secret' } }])
  await expect(page.locator('#device-admin-share-pass')).toHaveValue('')
})

test('Settings closes the sheet, and signing out drops what was typed', async ({ page }) => {
  await fakeCloud(page)
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await page.getByTestId('open-settings').click()
  await expect(page.getByTestId('settings')).toBeVisible()
  await expect(sheet).toBeHidden()
  await page.getByTestId('settings-back').click()
  await openSheet(page)
  await sheet.getByRole('tab', { name: 'Sharing' }).click()
  await page.locator('#device-admin-share-pass').fill('typed-secret')
  const settings = page.getByTestId('open-settings')
  const [bar, box] = await Promise.all([settings.boundingBox(), sheet.boundingBox()])
  expect(box!.y).toBeGreaterThanOrEqual(bar!.y + bar!.height)
  await settings.click()
  await page.getByRole('link', { name: 'Connection', exact: true }).click()
  await page.locator('#signout').click()
  await expect(page.getByTestId('signin')).toBeVisible()
  await expect(sheet).toBeHidden()
  const typed = await page.evaluate(() => [...document.querySelectorAll('input')].some((input) => input.value === 'typed-secret'))
  expect(typed).toBe(false)
})

test('a removal that took other cameras with it is a red alert naming them, not a tooltip', async ({ page }) => {
  const sent: unknown[] = []
  const [gate, porch] = ATTRIBUTES.devices
  const message = `removed ${gate.nickname} (${gate.uuid}), but the vendor also removed ${porch.nickname} (${porch.uuid}), which was not asked for; add those cameras again from the phone app`
  await page.route(/\/api\/cloud\/device-attributes/, (route) => route.fulfill({ json: ATTRIBUTES }))
  await page.route(/\/api\/devices\/unbind/, (route) => {
    sent.push(route.request().postDataJSON())
    return route.fulfill({
      json: { ok: true, status: 'removed', serial: gate.uuid, vendor_code: 2000, verified: true, also_missing: [{ uuid: porch.uuid, name: porch.nickname }], message },
    })
  })
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  const submit = sheet.getByTestId('unbind-submit')
  await expect(submit).toBeDisabled()
  await sheet.getByTestId('unbind-confirm').fill('front gate')
  await submit.click()
  const status = page.getByTestId('device-admin-status')
  await expect(status).toHaveText(message.charAt(0).toUpperCase() + message.slice(1))
  await expect(status).toHaveAttribute('role', 'alert')
  await expect(status).toHaveClass(/bad/)
  await expect(status).toBeInViewport()
  expect(sent).toEqual([{ sn: gate.uuid, confirm: 'front gate' }])
})

test('the three jobs sit on tabs, and the favourite switch is saved with the group', async ({ page }) => {
  const calls = await fakeCloud(page)
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  const favourite = page.locator('#device-admin-favourite')
  await expect(favourite).toHaveAttribute('role', 'switch')
  await favourite.check()
  await page.locator('#device-admin-group').fill('driveway')
  await sheet.getByRole('button', { name: 'Save group and favourite' }).click()
  await expect(page.getByTestId('device-admin-status')).toHaveText('Saved for the test')
  expect(calls).toEqual([
    { path: '/api/cloud/device/group', body: { uuid: 'AASN000000000001', group: 'driveway' } },
    { path: '/api/cloud/device/favourite', body: { uuid: 'AASN000000000001', favourite: true } },
  ])
  await expect(page.locator('#device-admin-accept')).toHaveCount(0)
  await sheet.getByRole('tab', { name: 'Sharing' }).click()
  await expect(page.locator('#device-admin-accept')).toBeVisible()
  await expect(page.locator('#device-admin-name')).toHaveCount(0)
  await page.keyboard.press('ArrowRight')
  await expect(sheet.getByRole('tab', { name: 'Password' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.locator('#device-admin-new')).toBeVisible()
  await expect(sheet.getByRole('tab', { name: 'Name', exact: true })).toHaveAttribute('aria-selected', 'false')
})

test('Devices, Cloud and Alarms share one corner and replace each other', async ({ page }) => {
  await fakeCloud(page)
  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: { entries: [], total: 0, pollSeconds: 60 } }))
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await page.getByTestId('recordings-launcher').click()
  await expect(page.getByTestId('recordings-panel')).toBeVisible()
  await expect(sheet).toBeHidden()
  await expect(page.getByTestId('device-admin-open')).toHaveAttribute('aria-pressed', 'false')
  await openSheet(page)
  await expect(page.getByTestId('recordings-panel')).toBeHidden()
  await page.getByTestId('alarm-bell').click()
  await expect(page.getByTestId('alarm-center')).toBeVisible()
  await expect(sheet).toBeHidden()
})

test('transferring a camera has its own field and asks before ownership moves', async ({ page }) => {
  const calls = await fakeCloud(page)
  await signIn(page, 'one')
  await playing(page)
  const sheet = await openSheet(page)
  await sheet.getByRole('tab', { name: 'Sharing' }).click()
  await page.locator('#device-admin-permit-to').fill('friend@example.invalid')
  await expect(page.locator('#device-admin-accept')).toHaveValue('')
  await expect(sheet.getByTestId('transfer-ask')).toBeDisabled()
  await expect(sheet.getByTestId('transfer-reason')).toHaveText('Type the account that will own this camera')
  await expect(sheet.getByText('Transfer to account', { exact: true })).toBeVisible()
  await page.locator('#device-admin-accept').fill('heir@example.invalid')
  await expect(page.locator('#device-admin-permit-to')).toHaveValue('friend@example.invalid')
  await sheet.getByTestId('transfer-ask').click()
  await expect(sheet.getByTestId('transfer-confirm-panel')).toContainText('heir@example.invalid')
  expect(calls.filter((call) => call.path === '/api/cloud/device/share')).toEqual([])
  await sheet.getByTestId('transfer-cancel').click()
  await expect(sheet.getByTestId('transfer-confirm')).toHaveCount(0)
  expect(calls.filter((call) => call.path === '/api/cloud/device/share')).toEqual([])
  await sheet.getByTestId('transfer-ask').click()
  await sheet.getByTestId('transfer-confirm').click()
  await expect(page.getByTestId('device-admin-status')).toContainText('invitation')
  expect(calls.filter((call) => call.path === '/api/cloud/device/share')).toEqual([
    { path: '/api/cloud/device/share', body: { uuids: [ATTRIBUTES.devices[0].uuid], account: 'heir@example.invalid' } },
  ])
})

