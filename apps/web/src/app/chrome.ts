import { live } from '../live/store'
import { lensEntries } from './lens'
import { currentCamera, deviceStatus, statusLine } from './devices'
import { hms } from './format'
import { state } from './state'

export function closeFloating(restoreFocus: boolean) {
  if (live.top.keysOpen) {
    live.top.restoreFocus = restoreFocus
    live.top.keysOpen = false
    return true
  }
  if (live.top.menuOpen) {
    live.top.menuOpen = false
    return true
  }
  return false
}

export function toggleKeys() {
  if (live.top.keysOpen) {
    closeFloating(true)
    return
  }
  live.top.menuOpen = false
  live.top.keysOpen = true
}

function lensMeta() {
  if (lensEntries().length > 1) return 'dual lens'
  return state.lens ? 'single lens' : ''
}

export function renderSwitcher() {
  const camera = currentCamera()
  const status = camera ? deviceStatus(camera.device) : { known: false, online: false }
  const top = live.top
  top.several = state.cameras.length > 1
  top.name = camera ? camera.name : 'No camera'
  top.dotOff = Boolean(camera) && status.known && !status.online
  top.dotLabel = statusLine(camera ? camera.device : null)
  top.meta = camera ? ['channel ' + camera.channel, lensMeta()].filter(Boolean).join(' · ') : 'nothing selected'
  top.selected = state.selectedId + ':' + state.channel
  top.gridDisabled = state.cameras.length === 0
  top.inGrid = state.view === 'grid'
}

export function renderCameraMenu() {
  live.top.cameras = state.cameras.map((camera, index) => ({
    key: camera.key,
    index,
    name: camera.name,
    online: Boolean(camera.device.online),
    sub: camera.device.online ? ['channel ' + camera.channel, camera.device.model].filter(Boolean).join(' · ') : statusLine(camera.device),
  }))
  live.top.selected = state.selectedId + ':' + state.channel
}

export function renderBanner() {
  const camera = currentCamera()
  const status = camera ? deviceStatus(camera.device) : null
  const detail = status && status.detail ? status.detail + '. ' : ''
  const banner = { title: '', text: '', tone: '' as typeof live.banner.tone, action: '' as typeof live.banner.action }
  if (state.networkLostAt) {
    banner.title = 'This computer lost its network at ' + hms(state.networkLostAt) + '.'
    banner.text = 'What you see below is the last frame the app received.'
    banner.tone = 'network'
    banner.action = 'retry'
  } else if (state.view === 'live' && camera && status!.known && !camera.device.online) {
    banner.title = camera.name + ' is offline according to the cloud.'
    banner.text = detail + 'Refresh the list once it answers again.'
    banner.tone = 'device'
    banner.action = 'refresh'
  } else if (state.view === 'live' && camera && !status!.known) {
    banner.title = 'The cloud status of ' + camera.name + ' could not be read.'
    banner.text = detail + 'The stream is still attempted, and its own error is shown below.'
    banner.tone = 'device'
    banner.action = 'refresh'
  } else if (state.bannerError) {
    banner.title = state.bannerError
    banner.tone = 'list'
    banner.action = 'refresh'
  }
  Object.assign(live.banner, banner)
}
