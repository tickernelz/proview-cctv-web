import { expect, type Page } from '@playwright/test'
import { scenarioUrl, signIn } from './app'

export type Check = (state: string) => Promise<void>

const ALARMS = {
  serial: 'gate',
  total: 2,
  finished: true,
  pollSeconds: 60,
  entries: [0, 1].map((index) => ({
    id: 'layout' + index,
    name: 'human detect',
    time: '2026-10-27 21:0' + index + ':10',
    utcTime: 1793112000 + index,
    channel: '0',
  })),
}

export async function overflowFindings(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const width = document.documentElement.clientWidth
    const found: string[] = []
    const name = (node: Element) => {
      const element = node as HTMLElement
      return element.dataset.testid || element.id || String(element.className || element.tagName).split(' ')[0]
    }
    const root = document.scrollingElement as HTMLElement
    if (root.scrollWidth > root.clientWidth) found.push('page scrolls ' + root.scrollWidth + '>' + root.clientWidth)
    const clipped = (node: Element) => {
      for (let parent = node.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        if (getComputedStyle(parent).overflowX !== 'visible') return true
      }
      return false
    }
    for (const node of document.querySelectorAll<HTMLElement>('body *')) {
      const box = node.getBoundingClientRect()
      if (box.width === 0 && box.height === 0) continue
      if (!node.checkVisibility({ visibilityProperty: true })) continue
      const style = getComputedStyle(node)
      const scroller = style.overflowX === 'auto' || style.overflowX === 'scroll'
      if (scroller && !node.matches('.tbl') && node.scrollWidth > node.clientWidth + 1) {
        found.push(name(node) + ' scrolls sideways ' + node.scrollWidth + '>' + node.clientWidth)
      }
      if (!clipped(node) && (box.right > width + 1 || box.left < -1)) {
        found.push(name(node) + ' outside ' + Math.round(box.left) + '..' + Math.round(box.right) + ' of ' + width)
      }
    }
    return [...new Set(found)]
  })
}

export async function smallTargets(page: Page, floor = 44): Promise<string[]> {
  return page.evaluate((floor) => {
    const found: string[] = []
    const query = 'button, a[href], input, select, textarea, [role="tab"], [role="menuitem"], [role="menuitemradio"]'
    for (const node of document.querySelectorAll<HTMLElement>(query)) {
      if (node.matches('.skip, .sr') || node.closest('[hidden], .sprite')) continue
      const target = node.matches('input[type="checkbox"], input[type="radio"]') ? node.closest('label') ?? node : node
      if (!target.checkVisibility({ visibilityProperty: true })) continue
      const box = target.getBoundingClientRect()
      if (box.width <= 1 || box.height <= 1) continue
      if (box.width < floor - 0.5 || box.height < floor - 0.5) {
        const label = node.dataset.testid || node.id || node.getAttribute('aria-label') || (node.textContent || '').trim().slice(0, 24) || node.tagName
        found.push(label + ' ' + Math.round(box.width) + 'x' + Math.round(box.height))
      }
    }
    return [...new Set(found)]
  }, floor)
}

async function liveAgain(page: Page) {
  await page.reload()
  await expect(page.getByTestId('live')).toBeVisible()
}

export async function walkScreens(page: Page, check: Check) {
  await page.goto(scenarioUrl('one'))
  await expect(page.getByTestId('signin')).toBeVisible()
  await check('sign-in, account')
  await page.getByRole('tab', { name: 'This network', exact: true }).click()
  await expect(page.locator('#panel-local')).toBeVisible()
  await check('sign-in, this network')

  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await page.route(/\/api\/device\/controls(\/presets)?$/, (route) =>
    route.fulfill({
      json: route.request().url().endsWith('/presets')
        ? [{ token: '1', name: 'Gate' }, { token: '2', name: 'Driveway' }]
        : {
            onvif: false,
            dvrip: true,
            capabilities: ['ptz move', 'ptz preset'].map((feature) => ({ feature, lens: 'bottom', backend: 'dvrip', state: 'supported', evidence: 'layout fixture' })),
          },
    }),
  )
  await signIn(page, 'many')
  await expect(page.getByTestId('live')).toBeVisible()
  await check('live')

  await expect(page.getByTestId('ctl-move')).toBeEnabled({ timeout: 20_000 })
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ptz-pad')).toBeVisible()
  await expect(page.getByTestId('ptz-preset')).toHaveCount(2)
  await check('pan and tilt pad')
  await page.getByTestId('ctl-move').click()
  await expect(page.getByTestId('ptz-pad')).toBeHidden()

  await page.getByTestId('camera-switcher').click()
  await expect(page.getByTestId('camera-menu')).toBeVisible()
  await check('camera menu')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('camera-menu')).toBeHidden()

  await page.getByTestId('quality-pick').click()
  await expect(page.getByTestId('quality-menu')).toBeVisible()
  await check('quality menu')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('quality-menu')).toBeHidden()

  await page.getByTestId('alarm-bell').click()
  await expect(page.getByTestId('alarm-row')).toHaveCount(2)
  await check('alarm sheet')
  await liveAgain(page)

  await page.locator('#recordings-launcher').click()
  await expect(page.locator('#recordings-panel')).toBeVisible()
  await check('cloud recordings')
  await liveAgain(page)

  await page.locator('#device-admin-open').click()
  await expect(page.locator('#device-admin')).toBeVisible()
  await check('devices sheet')
  await liveAgain(page)

  await page.locator('#pb-open').click()
  await expect(page.locator('#pb')).toBeVisible()
  await check('playback')
  await liveAgain(page)

  await page.getByTestId('grid-toggle').click()
  await expect(page.getByTestId('grid-tile')).toHaveCount(5)
  await check('every camera')
  await liveAgain(page)

  await page.getByTestId('open-settings').click()
  await expect(page.getByTestId('settings')).toBeVisible()
  const links = page.locator('.set-nav a[data-section]')
  for (const section of await links.evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.section!))) {
    await page.locator('.set-nav a[data-section="' + section + '"]').click()
    await expect(page.locator('#set-body > section[data-section="' + section + '"]')).toBeVisible()
    await check('settings, ' + section)
  }
}
