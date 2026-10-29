import { test, expect, type Page, type Route } from '@playwright/test'
import { signIn } from './app'
import { overflowFindings } from './layout'

type Format = { resolution: string; compression: string; fps: number; bitrate: number; gop: number; quality: number; bitrate_control: string }

const format = (fps: number, bitrate: number, resolution: string): Format => ({
  resolution,
  compression: 'H.265',
  fps,
  bitrate,
  gop: 36,
  quality: 3,
  bitrate_control: 'VBR',
})

const NETWORK = {
  ok: true,
  backend: 'dvrip',
  network: {
    host_ip: '192.168.0.100',
    submask: '255.255.255.0',
    gateway: '192.168.0.1',
    mac: '3C:EF:8C:11:22:33',
    dhcp: false,
    dns: '192.168.0.1',
    spare_dns: '8.8.8.8',
  },
}

const STORAGE = {
  ok: true,
  backend: 'dvrip',
  storage: {
    partitions: [{ index: 0, current: true, status: 1, total_space: '30528MB', remain_space: '29303MB', start: '2026-10-20 11:02:07' }],
    overwrite: 'OverWrite',
    record_mode: 'ConfigRecord',
  },
}

const field = (before: number, requested: number, after: number) => ({ field: 'MainFormat.Video.FPS', before, requested, after, matches: requested === after })

type Device = { encode: { main: Format; extra: Format }; writes: string[]; hold?: Promise<void>; answer: (fps: number) => { status: number; json: unknown } }

async function fakeDevice(page: Page, answer: Device['answer']) {
  const device: Device = { encode: { main: format(12, 1536, '3072*2048'), extra: format(12, 512, '640*360') }, writes: [], answer }
  await page.route(/\/api\/device\/settings\//, async (route: Route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (request.method() === 'PUT') {
      if (url.pathname.endsWith('/network')) return route.continue()
      device.writes.push(url.pathname + url.search)
      await device.hold
      const reply = device.answer(Number(url.searchParams.get('fps')))
      return route.fulfill({ status: reply.status, json: reply.json })
    }
    if (url.pathname.endsWith('/encode')) return route.fulfill({ json: { ok: true, backend: 'dvrip', encode: device.encode } })
    if (url.pathname.endsWith('/network')) return route.fulfill({ json: NETWORK })
    return route.fulfill({ json: STORAGE })
  })
  return device
}

async function openDeviceSettings(page: Page) {
  await signIn(page, 'one')
  await page.getByTestId('open-settings').click()
  await page.getByTestId('device-settings-link').click()
  await expect(page.locator('#set-body > section[data-section="device"]')).toBeVisible()
  await expect(page.getByTestId('device-status')).toHaveText('Read from the device just now.')
  await expect(page.locator('#device-main-fps')).toHaveValue('12')
}

test('Device settings writes one encoder field, shows the read-back, and ignores a second press', async ({ page }) => {
  let release = () => {}
  const device = await fakeDevice(page, (fps) => {
    const before = device.encode.main.fps
    device.encode.main = { ...device.encode.main, fps }
    return {
      status: 200,
      json: {
        ok: true,
        backend: 'dvrip',
        reverted: false,
        stream: 'main',
        reports: [{ block: 'Simplify.Encode', backup: '/data/config-backup-simplify-encode.json', fields: [field(before, fps, fps)], reverted: false, restored: [] }],
        errors: [],
      },
    }
  })
  await openDeviceSettings(page)
  const section = page.locator('#set-body > section[data-section="device"]')
  await expect(section).toContainText('H.265 · quality 3 · VBR')
  await expect(section).toContainText('192.168.0.100')
  await expect(section).toContainText('29303MB')
  await page.locator('#device-main-fps').fill('10')
  device.hold = new Promise((done) => (release = done))
  const save = page.getByTestId('device-save-main')
  await save.click()
  await expect(save).toHaveText('Saving main…')
  await expect(save).toHaveAttribute('aria-busy', 'true')
  await expect(page.getByTestId('device-status')).toHaveText('Writing the main encoder…')
  await save.click()
  release()
  await expect(page.getByTestId('device-status')).toHaveText('Read back after the write and kept.')
  expect(device.writes).toEqual(['/api/device/settings/encode?stream=main&fps=10'])
  const report = page.getByTestId('device-report')
  await expect(report).toContainText('Read back and kept. Copy: /data/config-backup-simplify-encode.json')
  await expect(report.locator('tbody tr')).toHaveText([/MainFormat\.Video\.FPS\s*12\s*10\s*10\s*matches/])
  await expect(page.locator('#device-main-fps')).toHaveValue('10')
  await expect(save).toHaveText('Save main')
  await expect(page.getByTestId('device-status')).not.toHaveClass(/warn-text/)
  await page.getByTestId('device-save-main').click()
  await expect(page.getByTestId('device-status')).toHaveText('Nothing changed on the main stream.')
  expect(device.writes).toHaveLength(1)
})

test('a write the device does not hold comes back as 409 and is reported as reverted', async ({ page }) => {
  const device = await fakeDevice(page, (fps) => ({
    status: 409,
    json: {
      ok: false,
      backend: 'dvrip',
      reverted: true,
      stream: 'main',
      reports: [
        {
          block: 'Simplify.Encode',
          backup: '/data/config-backup-simplify-encode.json',
          fields: [field(12, fps, 12)],
          reverted: true,
          restored: [field(12, 12, 12)],
        },
      ],
      errors: [],
    },
  }))
  await openDeviceSettings(page)
  await page.locator('#device-main-fps').fill('25')
  await page.getByTestId('device-save-main').click()
  const status = page.getByTestId('device-status')
  await expect(status).toHaveText('The write did not hold and was reverted; the device keeps its previous values.')
  await expect(status).toHaveClass(/warn-text/)
  const report = page.getByTestId('device-report')
  await expect(report).toContainText('The device returned a different value, so the saved copy was put back.')
  await expect(report.locator('tbody tr')).toHaveText([
    /MainFormat\.Video\.FPS\s*12\s*25\s*12\s*differs/,
    'put back from the saved copy',
    /MainFormat\.Video\.FPS\s*12\s*12\s*12\s*restored/,
  ])
  await expect(page.locator('#device-main-fps')).toHaveValue('12')
  expect(device.writes).toEqual(['/api/device/settings/encode?stream=main&fps=25'])
  const findings: string[] = []
  for (const width of [320, 360, 390, 640, 768, 1024, 1440, 1920]) {
    await page.setViewportSize({ width, height: 800 })
    findings.push(...(await overflowFindings(page)).map((line) => width + ' px: ' + line))
  }
  expect(findings).toEqual([])
})

test('a revert that fails is reported as a failed revert, not as kept', async ({ page }) => {
  const message =
    'Simplify.Encode did not read back after the write and the automatic revert failed (the copy was written back but FPS reads 25); the pre-write copy is at /data/config-backup-simplify-encode.json'
  await fakeDevice(page, () => ({ status: 409, json: { ok: false, message } }))
  await openDeviceSettings(page)
  await page.locator('#device-main-fps').fill('25')
  await page.getByTestId('device-save-main').click()
  const status = page.getByTestId('device-status')
  await expect(status).toHaveText('Could not write: ' + message)
  await expect(status).toHaveClass(/warn-text/)
  await expect(page.getByTestId('device-report')).toBeEmpty()
  await expect(page.getByTestId('device-save-main')).toHaveText('Save main')
})

test('the server refuses a gateway change and the device keeps its route', async ({ page }) => {
  await fakeDevice(page, () => ({ status: 500, json: { message: 'not faked' } }))
  const writes: string[] = []
  page.on('request', (request) => {
    if (request.method() === 'PUT' && request.url().includes('/api/device/settings/network')) writes.push(new URL(request.url()).search)
  })
  await openDeviceSettings(page)
  await page.locator('#device-gateway').fill('192.168.0.254')
  await page.getByTestId('device-save-network').click()
  const status = page.getByTestId('device-status')
  await expect(status).toContainText('Could not write: gateway=192.168.0.254 would change how this app reaches the camera')
  await expect(status).toHaveClass(/warn-text/)
  expect(writes).toEqual(['?gateway=192.168.0.254'])
})
