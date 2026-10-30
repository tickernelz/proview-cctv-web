import { test, expect, type Page, type WebSocketRoute } from '@playwright/test'
import { playing, signIn } from './app'

type Socket = { url: string; sent: string[]; audio: boolean; open: boolean }

function alaw(frame: number) {
  return JSON.stringify({ type: 'audio', frame, codec: 'alaw', rate: 8000, data: Buffer.alloc(320, frame % 2 ? 0x55 : 0xd5).toString('base64') })
}

async function liveWithAudio(page: Page, audio = true) {
  const sockets: Socket[] = []
  await page.routeWebSocket(/\/ws\/live/, (ws: WebSocketRoute) => {
    const socket: Socket = { url: ws.url(), sent: [], audio, open: true }
    sockets.push(socket)
    const server = ws.connectToServer()
    ws.onMessage((message) => {
      if (typeof message === 'string') socket.sent.push(message)
      server.send(message)
    })
    server.onMessage((message) => ws.send(message))
    let frame = 0
    const timer = setInterval(() => {
      if (socket.open && socket.audio) ws.send(alaw(frame++))
    }, 40)
    ws.onClose((code, reason) => {
      socket.open = false
      clearInterval(timer)
      server.close({ code, reason })
    })
    server.onClose((code, reason) => {
      socket.open = false
      clearInterval(timer)
      ws.close({ code, reason })
    })
  })
  return sockets
}

const SOUND_ON = JSON.stringify({ type: 'sound', on: true })
const SOUND_OFF = JSON.stringify({ type: 'sound', on: false })

function stageSound(page: Page) {
  return page.evaluate(() => {
    const player = (window as unknown as { state: { players: Map<string, { heard: number; soundState: string; sound: { received: number; context: AudioContext | null } }> } }).state.players.get('stage')
    return player ? { heard: player.heard, state: player.soundState, queued: player.sound.received, context: player.sound.context?.state ?? null } : null
  })
}

test('live sound stays off until Sound asks the server for the camera audio', async ({ page }) => {
  const sockets = await liveWithAudio(page)
  await signIn(page, 'one')
  await playing(page)
  const button = page.getByTestId('ctl-mute')
  await expect(button).toHaveAttribute('data-sound', 'off')
  await page.waitForTimeout(500)
  expect(sockets.flatMap((socket) => socket.sent)).not.toContain(SOUND_ON)
  await button.click()
  await expect.poll(() => sockets.at(-1)?.sent ?? []).toContain(SOUND_ON)
  await expect(button).toHaveAttribute('data-sound', 'playing')
  await expect(page.getByTestId('sound-label')).toHaveText('Sound')
  await button.click()
  await expect.poll(() => sockets.at(-1)?.sent ?? []).toContain(SOUND_OFF)
  await expect(button).toHaveAttribute('data-sound', 'off')
})

test('with Sound already on, the camera audio is decoded and queued on the video clock as the stream starts', async ({ page, browserName }) => {
  test.skip(browserName !== 'chromium', "Linux WebKit never buffers the fixture's H.264 (readyState 1, empty buffered range), so there is no video clock to queue against")
  await page.addInitScript(() => localStorage.setItem('pvw_prefs_v2', JSON.stringify({ muted: false })))
  const sockets = await liveWithAudio(page)
  await signIn(page, 'one')
  await expect.poll(() => sockets.at(-1)?.sent ?? [], { timeout: 15_000 }).toContain(SOUND_ON)
  await playing(page)
  await expect.poll(async () => (await stageSound(page))?.queued ?? 0, { timeout: 10_000 }).toBeGreaterThan(5)
  expect((await stageSound(page))?.heard).toBeGreaterThan(5)
})

test('a camera that sends no audio says so on the Sound button', async ({ page }) => {
  await liveWithAudio(page, false)
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('ctl-mute').click()
  await expect(page.getByTestId('ctl-mute')).toHaveAttribute('data-sound', 'silent', { timeout: 10_000 })
  await expect(page.getByTestId('sound-label')).toHaveText('No sound')
  await expect(page.getByTestId('ctl-mute')).toHaveAttribute('title', /the camera is sending no audio/)
})

test('in the grid only the selected tile asks for sound, and only once Sound is on', async ({ page }) => {
  const sockets = await liveWithAudio(page)
  await signIn(page, 'grid')
  await page.getByTestId('grid-toggle').click()
  const tiles = page.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect(page.locator('.tile.sel')).toHaveCount(1)
  await expect(page.locator('.tile.sel')).toHaveAttribute('data-state', 'playing', { timeout: 20_000 })
  await page.waitForTimeout(500)
  expect(sockets.flatMap((socket) => socket.sent)).not.toContain(SOUND_ON)
  await page.getByTestId('grid-sound').click()
  await expect(page.getByTestId('grid-sound')).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(() => sockets.filter((socket) => socket.open && socket.sent.includes(SOUND_ON)).length).toBe(1)
  await expect(page.locator('.tile.sel').getByTestId('grid-tile-state')).toHaveAttribute('data-sound', 'playing')
  await expect(page.locator('.tile:not(.sel)').getByTestId('grid-tile-state').first()).toHaveAttribute('data-sound', 'off')
  await page.keyboard.press('m')
  await expect(page.getByTestId('grid-sound')).toHaveAttribute('aria-pressed', 'false')
  await expect(page.locator('.tile.sel').getByTestId('grid-tile-state')).toHaveAttribute('data-sound', 'off')
})
