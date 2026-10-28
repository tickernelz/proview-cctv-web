import { existsSync, readFileSync } from 'node:fs'
import type { E2EConfig } from 'e2e'
import { web } from '@e2e-dev/web'
import { createOpenAICompatible } from '@ai-sdk/openai-compatible'
import { BROWSERS, SERVERS_FILE } from './scenarios.mjs'

if (!existsSync(SERVERS_FILE)) throw new Error('no fixture servers: start `npm run serve` first, it writes ' + SERVERS_FILE)
const servers = JSON.parse(readFileSync(SERVERS_FILE, 'utf8')) as Record<string, string>

const recorder = process.env.E2E_MODEL_BASE_URL
  ? createOpenAICompatible({
      name: 'recorder',
      baseURL: process.env.E2E_MODEL_BASE_URL,
      ...(process.env.E2E_MODEL_API_KEY ? { apiKey: process.env.E2E_MODEL_API_KEY } : {}),
    })(process.env.E2E_MODEL ?? '')
  : undefined

export default {
  tests: 'tests/**/*.e2e.ts',
  timeout: 60_000,
  assertionTimeout: 10_000,
  retries: 1,
  workers: Number(process.env.E2E_WORKERS ?? 2),
  trace: 'retain-on-failure',
  video: 'retain-on-failure',
  reporters: process.env.CI ? ['list', 'junit', 'markdown'] : ['list'],
  ...(recorder === undefined ? {} : { agents: { default: { model: recorder } } }),
  targets: BROWSERS.map((browser) => ({
    name: browser,
    engine: web({
      browser: browser as 'chromium' | 'webkit',
      viewport: { width: 1440, height: 900 },
      reducedMotion: 'reduce',
    }),
    app: { url: servers.one, identity: 'proview-web' },
  })),
} satisfies E2EConfig
