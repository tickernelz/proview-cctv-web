import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'
import { overflowFindings } from './layout'

type Dewarp = {
  gl: WebGL2RenderingContext
  frame: unknown
  mode: number
  setLens(lens: object): void
  setMount(name: string): void
  setView(view: object): void
  uploadFrame(source: TexImageSource): void
  render(): boolean
}
type Fisheye = {
  ProviewFisheye: { create(canvas: HTMLCanvasElement): Dewarp; supported(): boolean; model: { library: string } }
  ProviewFisheyeLive?: { dewarp: Dewarp; canvas: HTMLCanvasElement; stop(): void }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'the overlay needs WebGL2, which the WebKit build in CI does not expose')

async function openWith(page: Page, query: string) {
  const base = await signIn(page, 'one')
  await page.goto(base + '/?signed-in' + query + '#/live')
  await playing(page)
}

async function paintedShare(page: Page) {
  await expect.poll(() => page.evaluate(() => (window as unknown as Fisheye).ProviewFisheyeLive?.dewarp.frame != null)).toBe(true)
  const share = await page.evaluate(() => {
    const { dewarp, canvas } = (window as unknown as Fisheye).ProviewFisheyeLive!
    dewarp.render()
    const pixels = new Uint8Array(canvas.width * canvas.height * 4)
    dewarp.gl.readPixels(0, 0, canvas.width, canvas.height, dewarp.gl.RGBA, dewarp.gl.UNSIGNED_BYTE, pixels)
    let painted = 0
    for (let index = 3; index < pixels.length; index += 4) if (pixels[index] > 0) painted += 1
    return painted / (canvas.width * canvas.height)
  })
  console.log('fisheye overlay painted ' + (share * 100).toFixed(2) + ' %')
  return share
}

test('the fisheye overlay stays off until the URL asks for it', async ({ page }) => {
  await openWith(page, '')
  await expect(page.getByTestId('fisheye-overlay')).toHaveCount(0)
  expect(await page.evaluate(() => (window as unknown as Fisheye).ProviewFisheye.supported())).toBe(true)
  expect(await page.evaluate(() => (window as unknown as Fisheye).ProviewFisheye.model.library)).toBe('libfisheye.so')
})

test('?fisheye=equirect paints the covered hemisphere over the live frame and stop() removes it', async ({ page }) => {
  await openWith(page, '&fisheye=equirect')
  const overlay = page.getByTestId('fisheye-overlay')
  await expect(overlay).toHaveAttribute('data-mode', 'equirect')
  await expect(overlay).toHaveAttribute('data-mount', 'ceiling')
  await expect(page.getByTestId('live-frame').getByTestId('fisheye-overlay')).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as Fisheye).ProviewFisheyeLive!.dewarp.mode)).toBe(0)
  const share = await paintedShare(page)
  expect(share).toBeGreaterThan(0.45)
  expect(share).toBeLessThan(0.55)
  await page.evaluate(() => (window as unknown as Fisheye).ProviewFisheyeLive!.stop())
  await expect(overlay).toHaveCount(0)
})

test('?fisheye=ptz fills the virtual camera view', async ({ page }) => {
  await openWith(page, '&fisheye=ptz&pan=30&fov=70&mount=wall')
  const overlay = page.getByTestId('fisheye-overlay')
  await expect(overlay).toHaveAttribute('data-mode', 'ptz')
  await expect(overlay).toHaveAttribute('data-mount', 'wall')
  expect(await page.evaluate(() => (window as unknown as Fisheye).ProviewFisheyeLive!.dewarp.mode)).toBe(1)
  expect(await paintedShare(page)).toBeGreaterThan(0.95)
})

for (const view of [
  { mode: 'equirect', width: 180, height: 90, query: 'mode=equirect&width=180&height=90' },
  { mode: 'ptz', width: 160, height: 90, query: 'mode=ptz&width=160&height=90&pan=30&tilt=20&fov=70' },
]) {
  test(`the ${view.mode} shader samples the same fisheye pixels as the server map`, async ({ page }) => {
    await openWith(page, '')
    const result = await page.evaluate(async (view) => {
      const map = await fetch('/api/fisheye/map?' + view.query, { credentials: 'same-origin' }).then((reply) => reply.json())
      const source = document.createElement('canvas')
      source.width = 256
      source.height = 256
      const context = source.getContext('2d')!
      const image = context.createImageData(256, 256)
      for (let y = 0; y < 256; y += 1) {
        for (let x = 0; x < 256; x += 1) {
          const at = (y * 256 + x) * 4
          image.data[at] = x
          image.data[at + 1] = y
          image.data[at + 3] = 255
        }
      }
      context.putImageData(image, 0, 0)
      const canvas = document.createElement('canvas')
      canvas.width = view.width
      canvas.height = view.height
      const dewarp = (window as unknown as Fisheye).ProviewFisheye.create(canvas)
      dewarp.setLens({ imageWidth: 512, imageHeight: 512, centerX: 256, centerY: 256, radius: 240, maxThetaDeg: 90 })
      dewarp.setMount('ceiling')
      const params = new URLSearchParams(view.query)
      dewarp.setView({
        mode: view.mode,
        panDeg: Number(params.get('pan') || 0),
        tiltDeg: Number(params.get('tilt') || 0),
        fovDeg: Number(params.get('fov') || 90),
      })
      dewarp.uploadFrame(source)
      dewarp.render()
      const pixels = new Uint8Array(view.width * view.height * 4)
      dewarp.gl.readPixels(0, 0, view.width, view.height, dewarp.gl.RGBA, dewarp.gl.UNSIGNED_BYTE, pixels)
      let both = 0
      let disagree = 0
      let worst = 0
      for (let index = 0; index < view.width * view.height; index += 1) {
        const column = index % view.width
        const bottomUp = view.height - 1 - Math.floor(index / view.width)
        const [x, y] = map.sources[bottomUp * view.width + column]
        const painted = pixels[index * 4 + 3] > 0
        if (painted !== x >= 0) {
          disagree += 1
          continue
        }
        if (!painted) continue
        both += 1
        const dx = pixels[index * 4] * 2 + 1 - x
        const dy = pixels[index * 4 + 1] * 2 + 1 - y
        worst = Math.max(worst, Math.hypot(dx, dy))
      }
      return { valid: map.valid, both, disagree, worst }
    }, view)
    console.log('fisheye ' + view.mode + ': ' + JSON.stringify(result))
    expect(result.both).toBeGreaterThan(result.valid * 0.99)
    expect(result.disagree).toBeLessThan(view.width * view.height * 0.01)
    expect(result.worst).toBeLessThan(2.5)
  })
}

test('the overlay keeps the page inside 320 px', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 568 })
  await openWith(page, '&fisheye=equirect')
  await expect(page.getByTestId('fisheye-overlay')).toBeVisible()
  expect(await overflowFindings(page)).toEqual([])
})
