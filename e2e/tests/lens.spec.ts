import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

async function frameBoxes(page: Page) {
  return page.evaluate(() => {
    const frame = document.getElementById('lens-crop')!.getBoundingClientRect()
    const video = document.getElementById('video')!.getBoundingClientRect()
    return { frame: { w: frame.width, h: frame.height }, video: { dy: video.top - frame.top, h: video.height, w: video.width } }
  })
}

async function pickLens(page: Page, lens: string) {
  await page.getByTestId('lens-pick').click()
  await page.getByTestId('lens-menu').getByTestId('lens-option').filter({ hasText: lens }).click()
  await expect(page.getByTestId('lens-menu')).toBeHidden()
}

test('picture in picture is the default: the top lens fills the frame and the bottom lens sits in the corner', async ({ page, browserName }) => {
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'pip')
  const main = await frameBoxes(page)
  expect(Math.abs(main.frame.w / main.frame.h - 640 / 360)).toBeLessThan(0.02)
  expect(Math.abs(main.video.h - 2 * main.frame.h)).toBeLessThan(2)
  expect(Math.abs(main.video.dy)).toBeLessThan(2)
  const inset = page.getByTestId('pip-inset')
  await expect(inset).toBeVisible()
  await expect(inset).toHaveAttribute('aria-label', 'Swap lenses: show the bottom lens large')
  const box = (await inset.boundingBox())!
  expect(Math.abs(box.width / box.height - 640 / 360)).toBeLessThan(0.05)
  expect(box.width).toBeLessThan(main.frame.w / 2)
  if (browserName === 'chromium') {
    const painted = await page.waitForFunction(() => {
    const canvas = document.querySelector('[data-testid="pip-inset"] canvas') as HTMLCanvasElement | null
    if (!canvas || !canvas.width) return false
    const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data
    for (let index = 0; index < pixels.length; index += 4) if (pixels[index] + pixels[index + 1] + pixels[index + 2] > 30) return true
    return false
    })
    expect(await painted.jsonValue()).toBe(true)
  }
  await inset.click()
  await expect(inset).toHaveAttribute('aria-label', 'Swap lenses: show the top lens large')
  const swapped = await frameBoxes(page)
  expect(Math.abs(swapped.video.dy + swapped.frame.h)).toBeLessThan(2)
  await inset.click()
  await expect(inset).toHaveAttribute('aria-label', 'Swap lenses: show the bottom lens large')
})

test('one lens fills the frame at its own shape: the other half of the stacked picture sits outside it', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  await pickLens(page, 'Both lenses')
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
  await expect(page.getByTestId('pip-inset')).toHaveCount(0)
  const both = await frameBoxes(page)
  expect(both.video.dy).toBe(0)
  expect(Math.abs(both.frame.w / both.frame.h - 640 / 720)).toBeLessThan(0.02)
  for (const [lens, shift] of [['Bottom lens', -1], ['Top lens', 0]] as const) {
    await pickLens(page, lens)
    await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', lens === 'Top lens' ? '0' : '1')
    const one = await frameBoxes(page)
    expect(Math.abs(one.frame.w / one.frame.h - 640 / 360)).toBeLessThan(0.02)
    expect(Math.abs(one.video.h - 2 * one.frame.h)).toBeLessThan(2)
    expect(Math.abs(one.video.dy - shift * one.frame.h)).toBeLessThan(2)
  }
})

test('the lens menu offers picture in picture, both lenses, the top lens and the bottom lens', async ({ page }) => {
  await signIn(page, 'one')
  await playing(page)
  const menu = page.getByTestId('lens-menu')
  await page.getByTestId('lens-pick').click()
  await expect(menu.getByTestId('lens-option')).toHaveText([/Picture in picture/, /Both lenses/, /Top lens/, /Bottom lens/])
  await expect(menu.getByTestId('lens-option').filter({ hasText: 'Picture in picture' })).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.press('Escape')
  for (const lens of ['Top lens', 'Bottom lens', 'Picture in picture']) {
    await pickLens(page, lens)
    await expect(page.getByTestId('live-frame')).toBeVisible()
    await page.getByTestId('lens-pick').click()
    await expect(menu.getByTestId('lens-option').filter({ hasText: lens })).toHaveAttribute('aria-checked', 'true')
    await page.keyboard.press('Escape')
    await expect(menu).toBeHidden()
  }
  await page.keyboard.press('l')
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
})

test('More holds the lens modes and the swap, and a saved choice of both lenses still loads', async ({ page }) => {
  await page.addInitScript(() => {
    if (!sessionStorage.getItem('seeded')) {
      sessionStorage.setItem('seeded', '1')
      localStorage.setItem('pvw_prefs_v2', JSON.stringify({ lens: 'combined' }))
    }
  })
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
  await page.getByTestId('ctl-more').click()
  await expect(page.getByTestId('more-swap')).toHaveCount(0)
  await page.getByTestId('more-lens').click()
  await page.getByTestId('more-lens-menu').getByTestId('more-lens-option').filter({ hasText: 'Picture in picture' }).click()
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'pip')
  await page.getByTestId('ctl-more').click()
  await page.getByTestId('more-swap').click()
  await expect(page.getByTestId('pip-label')).toHaveText('Top lens')
})

test('a single-lens camera stays on its one picture: no inset, and the snapshot is named for the whole frame', async ({ page }) => {
  await page.route(/\/api\/lens\?/, (route) =>
    route.fulfill({ json: { deviceId: 'gate', channel: 1, streams: [{ codec: 'h264', width: 640, height: 720 }], lenses: [{ index: 0, label: 'Single lens', crop: { x: 0, y: 0, w: 1, h: 1 } }] } }),
  )
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
  await expect(page.getByTestId('pip-inset')).toHaveCount(0)
  await expect(page.getByTestId('lens-pick')).toHaveCount(0)
  const file = await Promise.all([page.waitForEvent('download'), page.getByTestId('ctl-snapshot').click()]).then(([download]) => download)
  expect(file.suggestedFilename()).toMatch(/_lens-combined_/)
})


test('a cloud camera whose lens layout is unknown until the first frame still opens in picture in picture', async ({ page }) => {
  const asked: string[] = []
  await page.route(/\/api\/lens\?/, (route) => {
    const url = new URL(route.request().url())
    asked.push(url.search)
    const width = Number(url.searchParams.get('width') || 0)
    const height = Number(url.searchParams.get('height') || 0)
    if (!width || !height) {
      return route.fulfill({
        json: {
          deviceId: 'gate',
          channel: 1,
          streams: [{ quality: 'main', codec: 'unknown', width: 0, height: 0, sizeSource: 'unknown' }],
          lenses: [],
          lensCount: 0,
          countSource: 'unknown',
          stack: 'single',
          cropsVerified: false,
          ptzSource: 'unknown',
        },
      })
    }
    return route.fulfill({
      json: {
        deviceId: 'gate',
        channel: 1,
        streams: [{ quality: 'main', codec: 'unknown', width, height, sizeSource: 'client_hint' }],
        lenses: [
          { index: 0, label: 'top', crop: { x: 0, y: 0, w: width, h: height / 2 }, ptz: false },
          { index: 1, label: 'bottom', crop: { x: 0, y: height / 2, w: width, h: height / 2 }, ptz: true },
        ],
        lensCount: 2,
        countSource: 'geometry',
        stack: 'vertical',
        cropsVerified: true,
        ptzSource: 'measured_default',
      },
    })
  })
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'pip')
  await expect(page.getByTestId('pip-inset')).toBeVisible()
  await expect(page.getByTestId('lens-pick')).toBeVisible()
  await expect(page.getByTestId('camera-switcher')).toContainText('dual lens')
  await expect(page.getByTestId('camera-switcher')).not.toContainText('single lens')
  expect(asked[0]).not.toContain('width=')
  expect(asked.some((query) => query.includes('width=640') && query.includes('height=720'))).toBe(true)
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('pvw_prefs_v2') || '{}').lens)).not.toBe('combined')
})


test('a frame from the camera being left does not decide the next camera\'s lens layout', async ({ page }) => {
  await page.addInitScript(() => {
    const queue: [WebSocket, unknown][] = []
    Object.assign(window, { __hold: true, __queued: queue })
    const Real = window.WebSocket
    window.WebSocket = class extends Real {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols)
        if (!String(url).includes('/ws/live')) return
        this.addEventListener('message', (event) => {
          if (!(window as unknown as { __hold: boolean }).__hold) return
          event.stopImmediatePropagation()
          queue.push([this, event.data])
        })
      }
    } as typeof WebSocket
    Object.assign(window, {
      __flush: () => {
        ;(window as unknown as { __hold: boolean }).__hold = false
        for (const [socket, data] of queue.splice(0)) socket.dispatchEvent(new MessageEvent('message', { data }))
      },
    })
  })
  const unknown = (device: string) => ({ deviceId: device, channel: 1, streams: [], lenses: [], lensCount: 0, countSource: 'unknown', stack: 'single', cropsVerified: false, ptzSource: 'unknown' })
  const dual = (device: string, width: number, height: number) => ({
    deviceId: device,
    channel: 1,
    streams: [{ quality: 'main', codec: 'unknown', width, height, sizeSource: 'client_hint' }],
    lenses: [
      { index: 0, label: 'top', crop: { x: 0, y: 0, w: width, h: height / 2 }, ptz: false },
      { index: 1, label: 'bottom', crop: { x: 0, y: height / 2, w: width, h: height / 2 }, ptz: true },
    ],
    lensCount: 2,
    countSource: 'geometry',
    stack: 'vertical',
    cropsVerified: true,
    ptzSource: 'measured_default',
  })
  const single = (device: string, width: number, height: number) => ({
    deviceId: device,
    channel: 1,
    streams: [{ quality: 'main', codec: 'unknown', width, height, sizeSource: 'client_hint' }],
    lenses: [{ index: 0, label: 'single', crop: { x: 0, y: 0, w: width, h: height }, ptz: false }],
    lensCount: 1,
    countSource: 'geometry',
    stack: 'single',
    cropsVerified: true,
    ptzSource: 'unknown',
  })
  let porchStreaming = false
  const porchSized: { afterStream: boolean }[] = []
  let release = () => {}
  const held = new Promise<void>((done) => (release = done))
  page.on('request', (request) => {
    if (request.url().includes('/api/stream?') && request.url().includes('device=porch')) porchStreaming = true
  })
  await page.route(/\/api\/lens\?/, async (route) => {
    const url = new URL(route.request().url())
    const device = url.searchParams.get('device') || ''
    const width = Number(url.searchParams.get('width') || 0)
    const height = Number(url.searchParams.get('height') || 0)
    if (device === 'porch' && !width) {
      await held
      return route.fulfill({ json: unknown(device) })
    }
    if (!width) return route.fulfill({ json: unknown(device) })
    if (device !== 'porch') return route.fulfill({ json: dual(device, width, height) })
    porchSized.push({ afterStream: porchStreaming })
    return route.fulfill({ json: porchStreaming ? single(device, width, height) : dual(device, width, height) })
  })
  await signIn(page, 'many')
  await expect.poll(() => page.evaluate(() => (window as unknown as { __queued: unknown[] }).__queued.length), { timeout: 20_000 }).toBeGreaterThan(3)
  await page.keyboard.press('2')
  await expect(page.getByTestId('camera-name')).toHaveText('Porch')
  await page.evaluate(() => (window as unknown as { __flush: () => void }).__flush())
  await expect.poll(() => page.evaluate(() => (document.getElementById('video') as HTMLVideoElement).videoWidth)).toBeGreaterThan(0)
  await page.waitForTimeout(500)
  release()
  await expect.poll(() => porchSized.some((call) => call.afterStream), { timeout: 20_000 }).toBe(true)
  expect(porchSized.filter((call) => !call.afterStream)).toEqual([])
  await playing(page)
  await expect(page.getByTestId('pip-inset')).toHaveCount(0)
  await expect(page.getByTestId('lens-pick')).toHaveCount(0)
  await expect(page.getByTestId('live-well')).toHaveAttribute('data-lens', 'combined')
  await expect(page.getByTestId('camera-switcher')).toContainText('single lens')
})

