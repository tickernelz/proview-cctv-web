import { test, expect, type Page, type WebSocketRoute } from '@playwright/test'
import { playing, signIn } from './app'

type Socket = { url: string; sent: string[]; audio: boolean; open: boolean; burst: (count: number) => void }

function alaw(frame: number) {
  return JSON.stringify({ type: 'audio', frame, codec: 'alaw', rate: 8000, data: Buffer.alloc(320, frame % 2 ? 0x55 : 0xd5).toString('base64') })
}

async function liveWithAudio(page: Page, audio = true, helloAudio?: string) {
  const sockets: Socket[] = []
  await page.routeWebSocket(/\/ws\/live/, (ws: WebSocketRoute) => {
    const socket: Socket = {
      url: ws.url(),
      sent: [],
      audio,
      open: true,
      burst: (count) => {
        for (let at = 0; at < count; at += 1) ws.send(alaw(9000 + at))
      },
    }
    sockets.push(socket)
    const server = ws.connectToServer()
    ws.onMessage((message) => {
      if (typeof message === 'string') socket.sent.push(message)
      server.send(message)
    })
    server.onMessage((message) => {
      if (helloAudio && typeof message === 'string' && message.includes('"hello"')) ws.send(JSON.stringify({ ...JSON.parse(message), audio: helloAudio }))
      else ws.send(message)
    })
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
  await expect(button).toHaveAttribute('data-sound', /^(waiting|playing)$/)
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

const RECORD_STARTS = () => {
  const starts: number[][] = []
  Object.assign(window, { __starts: starts })
  const start = AudioBufferSourceNode.prototype.start
  AudioBufferSourceNode.prototype.start = function (this: AudioBufferSourceNode, when?: number) {
    starts.push([when ?? 0, this.buffer ? this.buffer.duration : 0])
    return start.call(this, when)
  } as typeof start
}

test('twenty records that arrive together play one after another over 0.8 s', async ({ page, browserName }) => {
  test.skip(browserName !== 'chromium', "Linux WebKit never buffers the fixture's H.264, so there is no video clock to queue against")
  await page.addInitScript(() => localStorage.setItem('pvw_prefs_v2', JSON.stringify({ muted: false })))
  await page.addInitScript(RECORD_STARTS)
  const sockets = await liveWithAudio(page)
  await signIn(page, 'one')
  await playing(page)
  await expect.poll(async () => (await stageSound(page))?.queued ?? 0, { timeout: 10_000 }).toBeGreaterThan(2)
  for (const socket of sockets) socket.audio = false
  await page.waitForTimeout(150)
  await page.evaluate(() => {
    const player = (window as unknown as { state: { players: Map<string, { sound: { enqueue: (at: number, samples: Float32Array, rate: number) => void } }> } }).state.players.get('stage')!
    const enqueued: number[][] = []
    Object.assign(window, { __enqueued: enqueued })
    const enqueue = player.sound.enqueue.bind(player.sound)
    player.sound.enqueue = (at, samples, rate) => {
      enqueued.push([at, samples.length / rate])
      enqueue(at, samples, rate)
    }
  })
  sockets.at(-1)!.burst(20)
  await expect.poll(() => page.evaluate(() => (window as unknown as { __enqueued: number[][] }).__enqueued.length)).toBe(20)
  const enqueued = await page.evaluate(() => (window as unknown as { __enqueued: number[][] }).__enqueued)
  for (let index = 1; index < enqueued.length; index += 1) expect(enqueued[index][0]).toBeCloseTo(enqueued[index - 1][0] + enqueued[index - 1][1], 6)
  await page.evaluate(() => {
    const player = (window as unknown as { state: { players: Map<string, { video: HTMLVideoElement; sound: { context: AudioContext; queue: { at: number }[] } }> } }).state.players.get('stage')!
    const enqueued = (window as unknown as { __enqueued: number[][] }).__enqueued
    const first = enqueued[0][0]
    player.sound.queue = player.sound.queue.filter((chunk) => chunk.at >= first - 1e-9)
    const context = player.sound.context
    const zero = context.currentTime
    ;(window as unknown as { __starts: number[][] }).__starts.length = 0
    Object.defineProperty(player.video, 'currentTime', { configurable: true, get: () => first - 0.2 + (context.currentTime - zero), set: () => {} })
  })
  await expect.poll(() => page.evaluate(() => (window as unknown as { __starts: number[][] }).__starts.length), { timeout: 5_000 }).toBe(20)
  const starts = await page.evaluate(() => (window as unknown as { __starts: number[][] }).__starts)
  for (let index = 1; index < starts.length; index += 1) expect(starts[index][0]).toBeGreaterThanOrEqual(starts[index - 1][0] + starts[index - 1][1] - 1e-3)
  expect(starts.at(-1)![0] + starts.at(-1)![1] - starts[0][0]).toBeGreaterThan(0.79)
})

const START_SUSPENDED = () => {
  const gesture = { seen: false }
  window.addEventListener('pointerdown', () => (gesture.seen = true), true)
  window.addEventListener('keydown', () => (gesture.seen = true), true)
  const Native = window.AudioContext
  const resume = Native.prototype.resume
  Native.prototype.resume = function (this: AudioContext) {
    return gesture.seen ? resume.call(this) : Promise.resolve()
  }
  window.AudioContext = class extends Native {
    constructor(options?: AudioContextOptions) {
      super(options)
      void this.suspend()
    }
  }
}

test('sound saved on but held by the browser asks for a click, and the click starts it without muting', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('pvw_prefs_v2', JSON.stringify({ muted: false })))
  await page.addInitScript(START_SUSPENDED)
  await liveWithAudio(page)
  await signIn(page, 'one')
  await playing(page)
  const button = page.getByTestId('ctl-mute')
  await expect(button).toHaveAttribute('data-sound', 'blocked')
  await expect(page.getByTestId('sound-label')).toHaveText('Click to enable sound')
  await page.waitForTimeout(1000)
  await expect(button).toHaveAttribute('data-sound', 'blocked')
  await button.click()
  await expect(button).toHaveAttribute('data-sound', /^(waiting|playing)$/)
  await expect(button).toHaveAttribute('aria-pressed', 'true')
  const context = await page.evaluate(() => (window as unknown as { state: { players: Map<string, { sound: { context: AudioContext | null } }> } }).state.players.get('stage')?.sound.context?.state)
  expect(context).toBe('running')
})

test('a codec the page cannot play and a browser without Web Audio are named on screen', async ({ page }) => {
  await liveWithAudio(page, true, 'AAC')
  await signIn(page, 'one')
  await playing(page)
  await page.getByTestId('ctl-mute').click()
  await expect(page.getByTestId('sound-label')).toHaveText('Sound not playable')
  await page.addInitScript(() => {
    Object.assign(window, { AudioContext: undefined, webkitAudioContext: undefined })
  })
  await page.reload()
  await playing(page)
  await page.getByTestId('ctl-mute').click()
  await expect(page.getByTestId('sound-label')).toHaveText('No Web Audio')
  await expect(page.getByTestId('ctl-mute')).toHaveAttribute('title', /no Web Audio/)
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
  await page.getByTestId('view-grid').click()
  const tiles = page.getByTestId('grid-tile')
  await expect(tiles).toHaveCount(5)
  await expect(page.locator('.tile.sel')).toHaveCount(1)
  await expect(page.locator('.tile.sel')).toHaveAttribute('data-state', 'playing', { timeout: 20_000 })
  await page.waitForTimeout(500)
  expect(sockets.flatMap((socket) => socket.sent)).not.toContain(SOUND_ON)
  await page.getByTestId('grid-sound').click()
  await expect(page.getByTestId('grid-sound')).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(() => sockets.filter((socket) => socket.open && socket.sent.includes(SOUND_ON)).length).toBe(1)
  await expect(page.locator('.tile.sel').getByTestId('grid-tile-state')).toHaveAttribute('data-sound', /^(waiting|playing)$/)
  await expect(page.locator('.tile:not(.sel)').getByTestId('grid-tile-state').first()).toHaveAttribute('data-sound', 'off')
  await page.keyboard.press('m')
  await expect(page.getByTestId('grid-sound')).toHaveAttribute('aria-pressed', 'false')
  await expect(page.locator('.tile.sel').getByTestId('grid-tile-state')).toHaveAttribute('data-sound', 'off')
})
