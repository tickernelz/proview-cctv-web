import { test, expect, type Page } from '@playwright/test'
import { playing, signIn } from './app'

test.skip(({ browserName }) => browserName !== 'chromium', 'a fake microphone exists only behind Chromium flags')
test.use({
  launchOptions: { args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] },
  permissions: ['microphone'],
})

const METRICS = `(() => ({
  live: window.__streams.reduce((total, stream) => total + stream.getTracks().filter((track) => track.readyState === 'live').length, 0),
  streams: window.__streams.length,
  openTalk: window.__talkSockets.filter((socket) => socket.readyState === 1).length,
  sockets: window.__talkSockets.length,
  talk: (({ active, phase, mic, open, chunks, note }) => ({ active, phase, mic, open, chunks, note }))(window.proviewTalk.state()),
}))()`

function init(delays: number[]) {
  return `(() => {
  window.__streams = [];
  window.__talkSockets = [];
  const delays = ${JSON.stringify(delays)};
  let calls = 0;
  const media = navigator.mediaDevices;
  const original = media.getUserMedia.bind(media);
  media.getUserMedia = async (constraints) => {
    const stream = await original(constraints);
    window.__streams.push(stream);
    await new Promise((resolve) => setTimeout(resolve, delays[calls++] ?? 0));
    return stream;
  };
  const Native = window.WebSocket;
  class FakeTalk extends EventTarget {
    constructor(url) {
      super();
      this.url = url;
      this.readyState = 0;
      this.binaryType = 'blob';
      window.__talkSockets.push(this);
      setTimeout(() => {
        if (this.readyState !== 0) return;
        this.readyState = 1;
        this.dispatchEvent(new Event('open'));
        this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind: 'ready', path: 'relay', channel: null, claim_ms: 4 }) }));
      }, 30);
    }
    send() {}
    close() {
      if (this.readyState >= 2) return;
      this.readyState = 3;
      this.dispatchEvent(new CloseEvent('close'));
    }
  }
  window.WebSocket = new Proxy(Native, {
    construct(target, args) {
      return String(args[0]).includes('/ws/talk') ? new FakeTalk(args[0]) : new target(...args);
    },
  });
})()`
}

async function openWithGaps(page: Page, delays: number[]) {
  await page.route(/\/api\/device\/controls(\?|$)/, (route) =>
    route.fulfill({
      json: {
        onvif: false,
        dvrip: true,
        capabilities: [{ feature: 'two-way audio', lens: 'device', backend: 'dvrip', state: 'supported', evidence: 'e2e fixture' }],
      },
    }),
  )
  await page.addInitScript(init(delays))
  await signIn(page, 'one')
  await playing(page)
  await expect(page.getByTestId('ctl-talk')).toBeEnabled()
}

test('a press released while the microphone prompt is open leaves no stream and no socket', async ({ page }) => {
  await openWithGaps(page, [800, 50])
  const talk = page.getByTestId('ctl-talk')
  await page.keyboard.down('t')
  await page.waitForTimeout(120)
  await page.keyboard.up('t')
  await page.keyboard.down('t')
  await expect.poll(() => page.evaluate(METRICS), { timeout: 15_000 }).toMatchObject({ openTalk: 1 })
  await page.waitForTimeout(600)
  const held = await page.evaluate(METRICS)
  expect(held.talk.chunks, 'the second press still sends audio').toBeGreaterThan(0)
  await page.keyboard.up('t')
  await expect.poll(() => page.evaluate(METRICS), { timeout: 15_000 }).toMatchObject({
    live: 0,
    openTalk: 0,
    talk: { active: false, mic: 0, open: false },
  })
  const rested = await page.evaluate(METRICS)
  expect(rested.streams).toBe(2)
  expect(rested.talk.phase).not.toBe('error')
  expect(rested.talk.note).not.toContain('talk failed')
  await expect(talk).toHaveAttribute('aria-pressed', 'false')
})

test('two press-and-release rounds in a row leave the microphone at zero', async ({ page }) => {
  await openWithGaps(page, [300, 300])
  const talk = page.getByTestId('ctl-talk')
  await page.keyboard.down('t')
  await page.waitForTimeout(320)
  await page.keyboard.up('t')
  await page.keyboard.down('t')
  await expect.poll(() => page.evaluate(METRICS), { timeout: 15_000 }).toMatchObject({
    openTalk: 1,
    talk: { active: true, mic: 1 },
  })
  await page.waitForTimeout(320)
  const held = await page.evaluate(METRICS)
  expect(held.talk.chunks, 'the second press actually sends').toBeGreaterThan(0)
  await page.keyboard.up('t')
  await expect.poll(() => page.evaluate(METRICS), { timeout: 15_000 }).toMatchObject({
    live: 0,
    openTalk: 0,
    talk: { active: false, mic: 0, open: false },
  })
  expect((await page.evaluate(METRICS)).talk.phase).not.toBe('error')
  await expect(talk).toHaveAttribute('aria-pressed', 'false')
})
