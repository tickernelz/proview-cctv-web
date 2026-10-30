import { test, expect, type Page, type Route } from '@playwright/test'
import { openDetails, signIn } from './app'

const gate = (key: string) => 'SystemFunction.OtherFunction.' + key + ' is absent; the app gates this control on it (e2e fixture)'

type Light = { mode: string; brightness: number; period: { enable: boolean; start: string; end: string } | null; trigger: { level: number; duration: number } | null }

const report = (speaker: number, light: Light, double = false) => ({
  ok: true,
  light_mode: double
    ? {
        supported: true,
        evidence: 'Camera.WhiteLight.WorkMode=' + light.mode,
        value: {
          mode: light.mode,
          label: light.mode === 'Close' ? 'General night vision' : light.mode === 'Auto' ? 'Full colour' : 'Double light',
          options: [
            { mode: 'Auto', label: 'Full colour' },
            { mode: 'Close', label: 'General night vision' },
            { mode: 'Intelligent', label: 'Double light' },
          ],
        },
      }
    : { supported: false, evidence: gate('SupportDoubleLightBoxCamera'), value: null },
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
  low_lux: { supported: false, evidence: gate('SupportLowLuxMode'), value: true },
  white_light: { supported: true, evidence: 'Camera.WhiteLight WorkPeriod/MoveTrigLight', value: light },
  privacy: { supported: false, evidence: gate('SupportOneKeyMaskVideo'), value: null },
  sleep: { supported: false, evidence: gate('SupportTimingSleep'), value: null },
  status_led: { supported: false, evidence: gate('SupportStatusLed'), value: true },
  voice_tip: { supported: true, evidence: 'FbExtraStateCtrl', value: true },
  speaker_volume: { supported: true, evidence: 'fVideo.Volume', value: { left: speaker, right: speaker, block: 'fVideo.Volume' } },
  mic_volume: { supported: true, evidence: 'fVideo.VolumeIn', value: { left: 34, right: 35, block: 'fVideo.VolumeIn' } },
})

const FULL_LIGHT: Light = { mode: 'Intelligent', brightness: 50, period: { enable: true, start: '18:00', end: '06:00' }, trigger: { level: 3, duration: 60 } }

async function fakeCamera(page: Page, light: Light, double = false) {
  const camera = { speaker: 50, light, writes: [] as string[] }
  await page.route(/\/api\/device\/image(\/|\?|$)/, async (route: Route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.pathname === '/api/device/image/live') return route.fulfill({ status: 400, json: { ok: false, message: 'e2e: no live state' } })
    if (request.method() === 'POST') {
      camera.writes.push(url.pathname + '?' + [...url.searchParams].filter(([key]) => key !== 'device').map(([key, value]) => key + '=' + value).join('&'))
      if (url.pathname.endsWith('/volume')) camera.speaker = Number(url.searchParams.get('level'))
      if (url.pathname.endsWith('/light-mode')) camera.light = { ...camera.light, mode: url.searchParams.get('mode') || '' }
      if (url.pathname.endsWith('/light') && camera.light.period) {
        camera.light = { ...camera.light, period: { enable: url.searchParams.get('period') === 'true', start: url.searchParams.get('start') || '', end: url.searchParams.get('end') || '' } }
      }
      return route.fulfill({ json: { ok: true, feature: 'e2e', block: 'e2e', before: {}, requested: {}, after: {}, matches: true } })
    }
    return route.fulfill({ json: report(camera.speaker, camera.light, double) })
  })
  return camera
}

async function openImageSettings(page: Page) {
  await signIn(page, 'one')
  await page.getByTestId('open-settings').click()
  await page.getByTestId('image-settings-link').click()
  await expect(page.locator('#set-body > section[data-section="image"]')).toBeVisible()
  await expect(page.getByTestId('image-status')).toHaveText('Read from the camera just now.')
}

test('Image settings say in words what the camera does not offer, keep the flag under Details, and write a volume with a read-back', async ({ page }) => {
  const camera = await fakeCamera(page, FULL_LIGHT)
  await openImageSettings(page)
  await expect(page.getByTestId('image-day-night')).toHaveValue('3')
  await expect(page.locator('#set-body > section[data-section="image"]')).not.toContainText(/SystemFunction|OtherFunction|Camera\.FishEye|block back|e2e fixture/, { useInnerText: true })
  await expect(page.getByTestId('image-low-lux')).toBeDisabled()
  await expect(page.getByTestId('image-low-lux-reason')).toHaveText('Not offered by this camera.')
  await expect(await openDetails(page.getByTestId('image-low-lux-details'))).toHaveText(gate('SupportLowLuxMode'))
  await expect(page.getByTestId('image-led')).toBeDisabled()
  await expect(page.getByTestId('image-led-reason')).toHaveText('Not offered by this camera.')
  await expect(await openDetails(page.getByTestId('image-led-details'))).toHaveText(gate('SupportStatusLed'))
  await expect(page.getByTestId('image-privacy')).toBeDisabled()
  await expect(page.getByTestId('image-privacy-reason')).toHaveText('Not offered by this camera.')
  await expect(await openDetails(page.getByTestId('image-privacy-details'))).toHaveText(gate('SupportOneKeyMaskVideo'))
  await expect(page.getByTestId('image-sleep-reason')).toHaveText('Not offered by this camera.')
  await expect(await openDetails(page.getByTestId('image-sleep-details'))).toHaveText(gate('SupportTimingSleep'))
  await expect(page.getByTestId('image-scene-reason')).toHaveText('Not offered by this camera.')
  await expect(await openDetails(page.getByTestId('image-scene-details'))).toHaveText('Camera.FishEye answered no data (Ret 607 or a null entry)')
  await expect(page.getByTestId('image-voice-tip-details')).toHaveCount(0)
  await expect(page.getByTestId('image-voice-tip')).toBeEnabled()
  await expect(page.getByTestId('image-start')).toHaveValue('18:00')
  await expect(page.getByTestId('image-mic')).toHaveValue('34')
  await expect(page.locator('#image-defog')).toHaveCount(0)
  await page.getByTestId('image-speaker').fill('70')
  await page.getByTestId('image-save-speaker').click()
  await expect(page.getByTestId('image-status')).toHaveText('Speaker volume saved. The camera read it back.')
  expect(camera.writes).toEqual(['/api/device/image/volume?target=speaker&level=70'])
  await expect(page.getByTestId('image-speaker')).toHaveValue('70')
})

test('a spotlight without a motion trigger saves only its schedule', async ({ page }) => {
  const camera = await fakeCamera(page, { mode: 'Auto', brightness: 50, period: { enable: true, start: '18:00', end: '06:00' }, trigger: null })
  await openImageSettings(page)
  await expect(page.getByTestId('image-level')).toHaveCount(0)
  await page.getByTestId('image-start').fill('19:30')
  await page.getByTestId('image-save-light').click()
  await expect(page.getByTestId('image-status')).toHaveText('Light schedule saved. The camera read it back.')
  expect(camera.writes).toEqual(['/api/device/image/light?period=true&start=19:30&end=06:00'])
})

test('a double-light camera sets day and night through the light mode, as the app does', async ({ page }) => {
  const camera = await fakeCamera(page, FULL_LIGHT, true)
  await openImageSettings(page)
  await expect(page.getByTestId('image-day-night')).toHaveCount(0)
  await expect(page.getByTestId('image-light-mode')).toHaveValue('Intelligent')
  await page.getByTestId('image-light-mode').selectOption('Close')
  await page.getByTestId('image-save-light-mode').click()
  await expect(page.getByTestId('image-status')).toHaveText('Day/night mode saved. The camera read it back.')
  expect(camera.writes).toEqual(['/api/device/image/light-mode?mode=Close'])
  await expect(page.getByTestId('image-light-mode')).toHaveValue('Close')
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
