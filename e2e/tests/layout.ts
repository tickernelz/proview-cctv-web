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
    const height = document.documentElement.clientHeight
    const found: string[] = []
    const name = (node: Element) => {
      const element = node as HTMLElement
      return element.dataset?.testid || element.id || (element.getAttribute('class') || element.tagName).split(' ')[0]
    }
    const root = document.scrollingElement as HTMLElement
    if (root.scrollWidth > root.clientWidth) found.push('page scrolls ' + root.scrollWidth + '>' + root.clientWidth)
    const scrolls = (value: string) => value === 'auto' || value === 'scroll'
    const cuts = (value: string) => value === 'hidden' || value === 'clip'
    const ignored = (node: HTMLElement) =>
      node.closest('.sr, .sprite, .lens-crop, .prog') !== null || node.matches('video') || getComputedStyle(node).clipPath !== 'none'

    const cutBy = (node: HTMLElement, axis: 'x' | 'y') => {
      if (getComputedStyle(node).position === 'fixed') return null
      for (let parent = node.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        const style = getComputedStyle(parent)
        const value = axis === 'x' ? style.overflowX : style.overflowY
        if (scrolls(value)) return null
        if (cuts(value)) return parent
        if (style.position === 'fixed') return null
      }
      return null
    }
    const insideScrollerOrCut = (node: HTMLElement) => {
      if (getComputedStyle(node).position === 'fixed') return false
      for (let parent = node.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        const style = getComputedStyle(parent)
        if (style.overflowX !== 'visible') return true
        if (style.position === 'fixed') return false
      }
      return false
    }
    const insideVerticalScroller = (node: HTMLElement) => {
      for (let parent = node.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        const style = getComputedStyle(parent)
        if (scrolls(style.overflowY) && parent.scrollHeight > parent.clientHeight + 1) {
          const view = parent.getBoundingClientRect()
          return view.top >= -1 && view.top < height
        }
      }
      return false
    }
    const cutOff = (node: HTMLElement, box: DOMRect) => {
      const out: string[] = []
      for (const axis of ['x', 'y'] as const) {
        const parent = cutBy(node, axis)
        if (!parent || getComputedStyle(parent).textOverflow === 'ellipsis') continue
        const outer = parent.getBoundingClientRect()
        const start = axis === 'x' ? outer.left + parent.clientLeft : outer.top + parent.clientTop
        const end = start + (axis === 'x' ? parent.clientWidth : parent.clientHeight)
        const [low, high] = axis === 'x' ? [box.left, box.right] : [box.top, box.bottom]
        if (low < start - 2 || high > end + 2) {
          out.push(name(node) + ' cut ' + axis + ' ' + Math.round(low) + '..' + Math.round(high) + ' by ' + name(parent) + ' ' + Math.round(start) + '..' + Math.round(end))
        }
      }
      return out
    }

    for (const node of document.querySelectorAll<HTMLElement>('body *')) {
      const box = node.getBoundingClientRect()
      if (box.width === 0 && box.height === 0) continue
      if (!node.checkVisibility({ visibilityProperty: true })) continue
      if (ignored(node)) continue
      const style = getComputedStyle(node)
      if (scrolls(style.overflowX) && !node.matches('.tbl') && node.scrollWidth > node.clientWidth + 1) {
        found.push(name(node) + ' scrolls sideways ' + node.scrollWidth + '>' + node.clientWidth)
      }
      if ((box.right > width + 1 || box.left < -1) && !insideScrollerOrCut(node)) {
        found.push(name(node) + ' outside ' + Math.round(box.left) + '..' + Math.round(box.right) + ' of ' + width)
      }
      if ((box.top < -1 || box.bottom > height + 1) && !node.closest('.skip') && !insideVerticalScroller(node)) {
        found.push(name(node) + ' outside y ' + Math.round(box.top) + '..' + Math.round(box.bottom) + ' of ' + height)
      }
      found.push(...cutOff(node, box))
    }

    const surfaces = ['#pb', '#device-admin', '#recordings-panel', '.alarm-sheet', '.menu:not(.sheet)', '.menu-panel', '#pad']
      .flatMap((selector) => [...document.querySelectorAll<HTMLElement>(selector)])
      .filter((node) => !node.hidden && node.checkVisibility({ visibilityProperty: true }))
    const surface = surfaces.length ? surfaces[surfaces.length - 1] : document.body
    const query = 'button, a[href], input, select, textarea, [role="tab"], [role="menuitem"], [role="menuitemradio"]'
    for (const node of surface.querySelectorAll<HTMLElement>(query)) {
      if (node.matches('.skip, .sr') || node.closest('[hidden]')) continue
      if (!node.checkVisibility({ visibilityProperty: true })) continue
      const target = node.matches('input[type="checkbox"], input[type="radio"]') ? node.closest('label') ?? node : node
      const box = target.getBoundingClientRect()
      if (box.width <= 1 || box.height <= 1) continue
      const x = box.left + box.width / 2
      const y = box.top + box.height / 2
      if (x < 0 || y < 0 || x > width || y > height) {
        if (!insideVerticalScroller(target)) found.push(name(node) + ' unreachable at ' + Math.round(x) + ',' + Math.round(y))
        continue
      }
      const points = [
        [x, y],
        [box.left + box.width / 4, box.top + box.height / 4],
        [box.left + (box.width * 3) / 4, box.top + box.height / 4],
        [box.left + box.width / 4, box.top + (box.height * 3) / 4],
        [box.left + (box.width * 3) / 4, box.top + (box.height * 3) / 4],
      ]
      let inView = true
      for (let parent = target.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        const style = getComputedStyle(parent)
        if (scrolls(style.overflowY) || scrolls(style.overflowX)) {
          const view = parent.getBoundingClientRect()
          inView = y >= view.top && y <= view.bottom && x >= view.left && x <= view.right
          break
        }
      }
      if (!inView) continue
      const reaches = (hit: Element | null) => hit !== null && (target.contains(hit) || hit.contains(target))
      if (!points.some(([px, py]) => reaches(document.elementFromPoint(px, py)))) {
        const hit = document.elementFromPoint(x, y)
        found.push(name(node) + ' covered at ' + Math.round(x) + ',' + Math.round(y) + ' by ' + (hit ? name(hit) : 'nothing'))
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
  await expect(page.getByRole('tabpanel', { name: 'This network', exact: true })).toBeVisible()
  await check('sign-in, this network')

  await page.route(/\/api\/cloud\/alarms\?/, (route) => route.fulfill({ json: ALARMS }))
  await page.route(/\/api\/device\/controls(\/presets)?(\?|$)/, (route) =>
    route.fulfill({
      json: route.request().url().endsWith('/presets')
        ? [{ token: '1', name: 'Gate' }, { token: '2', name: 'Driveway' }]
        : {
            onvif: false,
            dvrip: true,
            capabilities: [
              ...['ptz move', 'ptz preset'].map((feature) => ({ feature, lens: 'bottom', backend: 'dvrip', state: 'supported', evidence: 'layout fixture' })),
              { feature: 'two-way audio', lens: 'device', backend: 'dvrip', state: 'supported', evidence: 'layout fixture' },
            ],
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
