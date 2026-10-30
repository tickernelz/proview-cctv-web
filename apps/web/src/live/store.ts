import { reactive } from 'vue'
import type { SoundState } from '../lib/stream-player'

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
export type Prefs = { theme: string; quality: string; lens: string; osd: boolean; muted: boolean; flip: boolean; ptzSpeed: string }
export type ModeReport = { mode?: string; source?: string; notice?: { message?: string; to?: string } | null }
export type CameraOption = { key: string; index: number; name: string; sub: string; online: boolean }
export type Preset = { token: string; name: string }
export type TourLine = { channel: number; id: number; name: string; points: number[] }
export type CameraInfo = { name: string; id: string; channel: string; model: string; connection: string; stream: string }
export type TalkPhase = 'idle' | 'asking' | 'opening' | 'sending' | 'error'

const noop = () => {}

export const live = reactive({
  camera: '',
  state: '',
  flag: { text: '', tone: '' },
  clock: '',
  muted: true,
  sound: 'off' as SoundState,
  record: { on: false, disabled: true, elapsed: '' },
  snapshotDisabled: true,
  gate: Object.fromEntries(
    ['intercom', 'move', 'up', 'down', 'left', 'right', 'home', 'cruise', 'aim', 'zoom', 'focus', 'presetEdit', 'calibrate'].map((act) => [act, { ok: false, why: 'Device controls have not been checked yet.' } as Gate]),
  ) as Record<string, Gate>,
  padOpen: false,
  pad: { presets: [] as Preset[], presetsShown: false, tour: false, status: '', editing: false, busy: false, edit: { loaded: false, presets: [] as Preset[], lines: [] as TourLine[] } },
  panes: { empty: false, live: true, grid: false },
  stage: {
    name: 'idle',
    title: '',
    text: '',
    code: '',
    trail: '',
    trailShown: false,
    canCopy: false,
    copied: false,
    passwordHint: false,
    fs: { shown: false, name: '', time: '' },
    lens: 'combined',
    crop: { width: '', height: '', flipped: 'false' },
    view: { left: '0%', top: '0%', width: '100%', height: '100%' },
  },
  sheet: '' as '' | 'alarms' | 'recordings' | 'devices',
  screen: '' as '' | 'playback',
  signin: { expired: false, expiredAt: 0, lastFrameAt: 0, generation: 0, focus: 0, mode: null as { mode?: string; source?: string } | null },
  theme: 'dark',
  top: {
    name: 'No camera',
    meta: 'nothing selected',
    dotShown: false,
    dotOff: false,
    dotLabel: 'cloud status not read',
    several: false,
    gridDisabled: true,
    inGrid: false,
    menuOpen: false,
    keysOpen: false,
    restoreFocus: false,
    selected: '',
    cameras: [] as CameraOption[],
  },
  banner: { title: '', text: '', tone: '' as '' | 'network' | 'device' | 'list', action: '' as '' | 'retry' | 'refresh' },
  settings: {
    shown: false,
    section: 'playback',
    focusHeading: 0,
    focusBack: 0,
    load: 0,
    reset: 0,
    focusKey: '',
    signedIn: false,
    account: '',
    prefs: { theme: 'system', quality: 'main', lens: 'combined', osd: true, muted: true, flip: false } as Prefs,
    mode: null as ModeReport | null,
    modeError: '',
    camera: null as CameraInfo | null,
    lensNote: 'Waiting for the first frame.',
    lensTitle: '',
    lensRolesKnown: false,
    controlNote: 'Device controls have not been checked yet.',
    capabilities: [] as string[][],
    light: { ok: false, why: 'Device controls have not been checked yet.' } as Gate,
    lightStatus: '',
  },
  grid: { open: false, generation: 0, selected: '', moveFocus: noop as (key: string) => void },
  talk: { on: false, level: 0, phase: 'idle' as TalkPhase, note: '' },
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
