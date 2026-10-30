import { test, expect, type Page, type Route } from '@playwright/test'
import { signIn } from './app'

const gate = (key: string) => 'SystemFunction.OtherFunction.' + key + ' is absent; the app gates this control on it (e2e fixture)'

const report = (defog: { enable: boolean; level: number }) => ({
  ok: true,
  day_night: {
    supported: true,
    evidence: 'Camera.Param[0].DayNightColor=0x00000003',
    value: {
      code: 3,
      key: 'smart_light',
      label: 'Smart alarm light',
      options: [
        { code: 0, key: 'auto', label: 'Auto colour' },
        { code: 1, key: 'color', label: 'Colour' },
        { code: 2, key: 'black_white', label: 'Black and white' },
        { code: 3, key: 'smart_light', label: 'Smart alarm light' },
      ],
    },
  },
  scene: { supported: false, evidence: 'Camera.FishEye answered no data (Ret 607 or a null entry)', value: null },
  defog: { supported: true, evidence: 'Camera.ClearFog[0]', value: defog },
  low_lux: { supported: false, evidence: gate('SupportLowLuxMode'), value: true },
  white_light: {
    supported: true,
    evidence: 'Camera.WhiteLight WorkPeriod/MoveTrigLight',
    value: { mode: 'Intelligent', brightness: 50, period: { enable: true, start: '18:00', end: '06:00' }, trigger: { level: 3, duration: 60 } },
  },
  privacy: { supported: false, evidence: gate('SupportOneKeyMaskVideo'), value: null },
  sleep: { supported: false, evidence: gate('SupportTimingSleep'), value: null },
  status_led: { supported: false, evidence: gate('SupportStatusLed'), value: true },
  voice_tip: { supported: true, evidence: 'FbExtraStateCtrl', value: true },
  speaker_volume: { supported: true, evidence: 'fVideo.Volume', value: { left: 50, right: 50, block: 'fVideo.Volume' } },
  mic_volume: { supported: true, evidence: 'fVideo.VolumeIn', value: { left: 34, right: 35, block: 'fVideo.VolumeIn' } },
})

async function fakeCamera(page: Page) {
  const camera = { defog: { enable: false, level: 50 }, writes: [] as string[] }
  await page.route(/\/api\/device\/image(\/|\?|$)/, async (route: Route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.pathname === '/api/device/image/live') return route.fulfill({ status: 400, json: { ok: false, message: 'e2e: no live state' } })
    if (request.method() === 'POST') {
      camera.writes.push(url.pathname + '?' + [...url.searchParams].filter(([key]) => key !== 'device').map(([key, value]) => key + '=' + value).join('&'))
      const before = [camera.defog]
      camera.defog = { enable: url.searchParams.get('enable') === 'true', level: Number(url.searchParams.get('level')) }
      return route.fulfill({ json: { ok: true, feature: 'defog', block: 'Camera.ClearFog', before, requested: [camera.defog], after: [camera.defog], matches: true } })
    }
    return route.fulfill({ json: report(camera.defog) })
  })
  return camera
}

test('Image settings show gated controls with the camera reason and write defog with a read-back', async ({ page }) => {
  const camera = await fakeCamera(page)
  await signIn(page, 'one')
  await page.getByTestId('open-settings').click()
  await page.getByTestId('image-settings-link').click()
  const section = page.locator('#set-body > section[data-section="image"]')
  await expect(section).toBeVisible()
  await expect(page.getByTestId('image-status')).toHaveText('Read from the camera just now.')
  await expect(page.getByTestId('image-day-night')).toHaveValue('3')
  await expect(page.getByTestId('image-low-lux')).toBeDisabled()
  await expect(page.getByTestId('image-low-lux-reason')).toContainText('SupportLowLuxMode')
  await expect(page.getByTestId('image-led')).toBeDisabled()
  await expect(page.getByTestId('image-led-reason')).toContainText('SupportStatusLed')
  await expect(page.getByTestId('image-privacy')).toBeDisabled()
  await expect(page.getByTestId('image-privacy-reason')).toContainText('SupportOneKeyMaskVideo')
  await expect(page.getByTestId('image-sleep-reason')).toContainText('SupportTimingSleep')
  await expect(page.getByTestId('image-scene-reason')).toContainText('Camera.FishEye')
  await expect(page.getByTestId('image-voice-tip')).toBeEnabled()
  await expect(page.getByTestId('image-start')).toHaveValue('18:00')
  await expect(page.getByTestId('image-mic')).toHaveValue('34')
  await page.getByTestId('image-defog').check()
  await page.getByTestId('image-defog-level').fill('70')
  await page.getByTestId('image-save-defog').click()
  await expect(page.getByTestId('image-status')).toHaveText('Defog saved. The camera read it back.')
  expect(camera.writes).toEqual(['/api/device/image/defog?enable=true&level=70'])
  await expect(page.getByTestId('image-defog')).toBeChecked()
  await expect(page.getByTestId('image-defog-level')).toHaveValue('70')
})

test('the live view says the lens is covered and uncovers it', async ({ page }) => {
  let covered = true
  const posts: string[] = []
  await page.route(/\/api\/device\/image\//, async (route: Route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (request.method() === 'POST') {
      posts.push(url.pathname + '?enable=' + url.searchParams.get('enable'))
      covered = false
      return route.fulfill({ json: { ok: true, feature: 'privacy shading', block: 'General.OneKeyMaskVideo', before: {}, requested: {}, after: {}, matches: true } })
    }
    return route.fulfill({
      json: {
        ok: true,
        low_lux: { supported: false, evidence: 'e2e', value: null },
        privacy: { supported: true, evidence: 'e2e', value: covered },
        sleep: { supported: false, evidence: 'e2e', value: null },
      },
    })
  })
  await signIn(page, 'one')
  const notice = page.getByTestId('image-live-privacy')
  await expect(notice).toContainText('Privacy shading is on')
  await notice.getByRole('button', { name: 'Uncover the lens' }).click()
  await expect(notice).toHaveCount(0)
  expect(posts).toEqual(['/api/device/image/privacy?enable=false'])
})
