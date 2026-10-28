import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  globalSetup: './global-setup.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 1,
  workers: Number(process.env.E2E_WORKERS ?? 2),
  timeout: 60_000,
  expect: {
    timeout: 10_000,
    toHaveScreenshot: { maxDiffPixelRatio: 0.002, animations: 'disabled', caret: 'hide', scale: 'css' },
  },
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    trace: 'retain-on-failure',
    video: 'retain-on-failure',
    screenshot: 'only-on-failure',
    reducedMotion: 'reduce',
  },
  projects: [
    { name: 'chromium', testIgnore: /touch\.spec/, use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } },
    { name: 'webkit', testIgnore: /touch\.spec/, use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 900 } } },
    {
      name: 'mobile',
      testMatch: /touch\.spec/,
      use: { ...devices['Pixel 7'], viewport: { width: 390, height: 844 }, deviceScaleFactor: 1 },
    },
    {
      name: 'tablet',
      testMatch: /touch\.spec/,
      use: { ...devices['Desktop Chrome'], viewport: { width: 768, height: 1024 }, isMobile: true, hasTouch: true },
    },
  ],
})
