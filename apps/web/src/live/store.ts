import { reactive } from 'vue'

export type Gate = { ok: boolean; why: string }
export type Launcher = {
  id: string
  label: string
  icon: string
  slot: 'start' | 'end'
  title?: string
  controls?: string
  pressed?: boolean | null
  onClick: () => void
}
export type LensOption = { value: string; name: string; sub: string }

const noop = () => {}

export const live = reactive({
  camera: '',
  state: '',
  flag: { text: '', tone: '' },
  clock: '',
  muted: true,
  record: { on: false, disabled: true, elapsed: '' },
  snapshotDisabled: true,
  gate: {
    intercom: { ok: false, why: 'Device controls have not been checked yet.' } as Gate,
    move: { ok: false, why: 'Device controls have not been checked yet.' } as Gate,
  } as Record<string, Gate>,
  padOpen: false,
  sheet: '' as '' | 'alarms' | 'recordings' | 'devices',
  screen: '' as '' | 'playback',
  grid: { open: false, generation: 0, selected: '', moveFocus: noop as (key: string) => void },
  talk: { on: false, level: 0 },
  lens: { shown: false, label: 'Both lenses', value: 'combined', options: [] as LensOption[], note: '' },
  quality: 'main',
  launchers: [] as Launcher[],
  actions: {
    startTalk: noop,
    stopTalk: noop,
    toggleMute: noop,
    toggleRecord: noop,
    captureSnapshot: noop,
    togglePad: noop,
    toggleFullscreen: noop,
    chooseLens: noop as (value: string) => void,
    setQuality: noop as (value: string) => void,
  },
})

export const menuOpen = () => document.querySelector('.menu-panel[data-state="open"]') !== null

export function addLauncher(launcher: Launcher): Launcher {
  live.launchers.push(launcher)
  return live.launchers[live.launchers.length - 1]
}
