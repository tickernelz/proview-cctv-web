import { join } from 'node:path'

const camera = (id, name, online = true) => ({ id, name, model: 'XM-IPC', online, channels: 1 })

export const SCENARIOS = {
  empty: [],
  one: [camera('gate', 'Gate camera')],
  many: [
    camera('gate', 'Gate camera'),
    camera('porch', 'Porch'),
    camera('garage', 'Garage'),
    camera('yard', 'Back yard'),
    camera('shed', 'Shed', false),
  ],
  grid: [
    camera('gate', 'Gate camera'),
    camera('porch', 'Porch'),
    camera('garage', 'Garage'),
    camera('yard', 'Back yard'),
    camera('shed', 'Shed', false),
  ],
  settings: [camera('gate', 'Gate camera')],
  desktop: [camera('gate', 'Gate camera')],
}

export const BROWSERS = ['chromium', 'webkit']
export const PER_BROWSER = ['grid', 'settings', 'desktop']
export const SERVERS_FILE = join(import.meta.dirname, '.servers.json')

export const serverNames = () =>
  Object.keys(SCENARIOS).flatMap((name) => (PER_BROWSER.includes(name) ? BROWSERS.map((browser) => name + '-' + browser) : [name]))
