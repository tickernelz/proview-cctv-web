import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

test.skip(({ browserName }) => browserName !== 'chromium', 'a fake microphone exists only behind Chromium flags')
test.use({
  launchOptions: { args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] },
  permissions: ['microphone'],
})

type Peer = { pcm: Buffer[]; text: string[]; closed: boolean }

async function openWithPeer(page: Page, first: object): Promise<Peer> {
  const peer: Peer = { pcm: [], text: [], closed: false }
  await page.route(/\/api\/device\/controls(\?|$)/, (route) =>
    route.fulfill({
      json: {
        onvif: false,
        dvrip: true,
        capabilities: [{ feature: 'two-way audio', lens: 'device', backend: 'dvrip', state: 'supported', evidence: 'e2e fixture' }],
      },
    }),
  )
  await page.routeWebSocket(/\/ws\/talk/, (socket) => {
    socket.onMessage((message) => {
      if (typeof message === 'string') peer.text.push(message)
      else peer.pcm.push(message)
    })
    socket.onClose(() => {
      peer.closed = true
    })
    socket.send(JSON.stringify(first))
  })
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('ctl-talk')).toBeEnabled()
  return peer
}

test('holding T streams 8 kHz PCM to the talk peer and releasing it stops the uplink', async ({ page }) => {
  const peer = await openWithPeer(page, { kind: 'ready', path: 'the fake peer', channel: null, claim_ms: 7 })
  const talk = page.getByTestId('ctl-talk')
  await page.keyboard.down('t')
  await expect(talk).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('live-state')).toHaveText('live — the microphone is open')
  await expect(page.getByTestId('live-lag')).toHaveText('release to stop')
  await expect(page.getByTestId('talk-note')).toContainText('sending over the fake peer · channel device · claim 7 ms')
  await expect(page.getByTestId('talk-note')).toHaveClass(/live/)
  await expect.poll(() => peer.pcm.length, { timeout: 15_000 }).toBeGreaterThanOrEqual(25)
  await page.keyboard.up('t')
  await expect(talk).toHaveAttribute('aria-pressed', 'false')
  await expect(page.getByTestId('live-state')).not.toContainText('microphone')
  await expect(page.getByTestId('talk-note')).toContainText(/^talk closed · the fake peer · \d+ chunks · \d+ PCM16 bytes · \d+\.\d s · \d+\.\d chunks\/s$/)
  await expect(page.getByTestId('ctl-talk')).toBeEnabled()
  await expect.poll(() => peer.closed).toBe(true)
  expect(peer.text).toEqual([JSON.stringify({ type: 'stop' })])
  expect(new Set(peer.pcm.map((chunk) => chunk.length))).toEqual(new Set([640]))
  const peaks = peer.pcm.map((chunk) => {
    let peak = 0
    for (let at = 0; at < chunk.length; at += 2) peak = Math.max(peak, Math.abs(chunk.readInt16LE(at)))
    return peak
  })
  const loud = peaks.filter((peak) => peak > 3276).length
  console.log('talk: ' + peer.pcm.length + ' chunks, ' + loud + ' above 10 % full scale, peak ' + Math.max(...peaks))
  expect(loud).toBeGreaterThan(0)
  const state = await page.evaluate(() => (window as unknown as { proviewTalk: { state(): { active: boolean; open: boolean; chunks: number } } }).proviewTalk.state())
  expect(state.active).toBe(false)
  expect(state.open).toBe(false)
  expect(state.chunks).toBe(peer.pcm.length)
})

test('the microphone is released when the button comes up', async ({ page }) => {
  await openWithPeer(page, { kind: 'ready', path: 'relay', channel: null, claim_ms: 4 })
  const mic = () => page.evaluate(() => (window as unknown as { proviewTalk: { state(): { mic: number } } }).proviewTalk.state().mic)
  await page.keyboard.down('t')
  await expect.poll(mic, { timeout: 10_000 }).toBe(1)
  await expect(page.getByTestId('talk-note')).toContainText('sending over the cloud relay')
  await page.keyboard.up('t')
  await expect.poll(mic).toBe(0)
})

test('a refusal from the path disables Talk and keeps the reason on screen', async ({ page }) => {
  await openWithPeer(page, { kind: 'error', scope: 'path', message: 'no path to the camera (direct: PROVIEW_DEVICE_HOST is not set; vendor relay: no camera was named)' })
  await page.keyboard.down('t')
  await expect(page.getByTestId('talk-note')).toHaveText(/^talk failed: no path to the camera /)
  const title = await page.evaluate(() => document.getElementById('talk')!.title)
  const note = await page.evaluate(() => document.getElementById('talk-note')!.textContent)
  expect(title).toBe('two-way audio is unavailable: ' + (note ?? '').replace('talk failed: ', ''))
  await expect(page.getByTestId('ctl-talk')).toBeDisabled()
  await page.keyboard.up('t')
})

test('a refusal from the camera leaves Talk usable and names the trouble', async ({ page }) => {
  await openWithPeer(page, { kind: 'error', scope: 'device', message: 'the vendor relay answered 12' })
  await page.keyboard.down('t')
  await expect(page.getByTestId('talk-note')).toHaveText('talk failed: the vendor relay answered 12')
  await expect(page.getByTestId('talk-note')).toHaveClass(/bad/)
  await page.keyboard.up('t')
  await expect(page.getByTestId('ctl-talk')).toBeEnabled()
})

test('a refusal from the talk peer is shown and leaves Talk off', async ({ page }) => {
  const peer = await openWithPeer(page, { kind: 'error', message: 'the camera speaker is busy' })
  await page.keyboard.down('t')
  await expect(page.getByTestId('talk-note')).toHaveText('talk failed: the camera speaker is busy')
  await expect(page.getByTestId('talk-note')).toHaveClass(/bad/)
  await expect(page.getByTestId('ctl-talk')).toHaveAttribute('aria-pressed', 'false')
  expect(peer.pcm).toEqual([])
})
