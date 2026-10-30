import { closePad, probeControlChannel, setControlStatus } from './controls'
import { loadLens, renderLensPicker } from './lens'
import { live } from '../live/store'
import { stopTalk } from '../live/talk'
import { renderBanner, renderCameraMenu, renderSwitcher } from './chrome'
import { buildCameras, currentCamera, deviceStatus } from './devices'
import { stopRecording } from './media'
import { renderPrefsIntoForm, sessionLost } from './shell'
import { renderCameraInfo, requestStream, setStage, stopStagePlayer } from './stage'
import { savePrefs, state } from './state'
import { api } from '../lib/api'
import { shell } from '../shell/layout'
import { liveHash, replaceHash } from '../shell/router'

function renderPanes() {
  const any = state.cameras.length > 0
  live.panes.empty = !any
  live.panes.live = any && state.view === 'live'
  live.panes.grid = any && state.view === 'grid'
}

export function openCamera(index: number) {
  const camera = state.cameras[index]
  if (!camera) return
  if (state.view !== 'live') setView('live', { keepStream: true })
  selectDevice(camera.device.id, { channel: camera.channel })
}

export async function loadDevices(options: { keepScreen?: boolean } = {}) {
  state.bannerError = ''
  renderBanner()
  try {
    state.devices = await api('/api/devices')
  } catch (error: any) {
    if (sessionLost(error)) return
    state.bannerError = 'Could not load the cameras: ' + error.message
    renderBanner()
    return
  }
  buildCameras()
  renderCameraMenu()
  window.dispatchEvent(new Event('proview:devices'))
  const keep = state.cameras.find((camera) => camera.key === state.selectedId + ':' + state.channel)
  const resume = state.resume && state.cameras.find((camera) => camera.key === state.resume.id + ':' + state.resume.channel)
  state.resume = null
  renderPanes()
  if (!state.cameras.length) {
    state.selectedId = null
    stopAll()
    renderSwitcher()
    renderCameraInfo()
    return
  }
  const target = resume || keep || state.cameras[0]
  if (!keep || resume || options.keepScreen) {
    state.selectedId = null
    await selectDevice(target.device.id, { channel: target.channel })
  } else {
    renderSwitcher()
    if (state.view === 'grid') startGrid()
  }
}

async function selectDevice(deviceId: string, options: { channel?: number } = {}) {
  const device = state.devices.find((entry) => entry.id === deviceId)
  if (!device) return
  const channel = options.channel || 1
  const changed = state.selectedId !== deviceId || state.channel !== channel
  if (changed) {
    stopTalk()
    stopRecording()
    closePad()
    state.lensView = state.prefs.lens
  }
  state.selectedId = deviceId
  state.channel = channel
  state.ticket = null
  state.stage.lastFrameAt = 0
  state.tour = null
  live.pad.tour = false
  renderSwitcher()
  renderCameraMenu()
  setControlStatus('')
  live.settings.lightStatus = ''
  if (state.view === 'grid') renderTileSelection()
  if (deviceStatus(device).known && !device.online) {
    stopStagePlayer()
    state.lens = null
    renderLensPicker()
    setStage('offline')
    renderCameraInfo()
    await probeControlChannel(device)
    return
  }
  setStage('loading')
  await loadLens(device.id, channel)
  if (state.view === 'live' && state.screen === 'watch' && shell.view !== 'events') await requestStream()
  await probeControlChannel(device)
  renderCameraInfo()
}

export function stopAll() {
  stopTalk()
  stopStagePlayer()
  stopGrid()
  closePad()
  state.ticket = null
}

export async function toggleFullscreen() {
  const target = document.getElementById(state.view === 'grid' ? 'tiles' : 'well')
  if (document.fullscreenElement) {
    await document.exitFullscreen().catch(() => {})
    return
  }
  if (target && target.requestFullscreen) await target.requestFullscreen().catch(() => setControlStatus('Full screen was refused by the browser.'))
}

function startGrid() {
  renderTileSelection()
  live.grid.generation += 1
  live.grid.open = true
}

function stopGrid() {
  live.grid.open = false
}

function renderTileSelection() {
  live.grid.selected = state.selectedId + ':' + state.channel
}

export function moveTileFocus(key: string) {
  live.grid.moveFocus(key)
}

export function setView(name: string, options: { keepStream?: boolean } = {}) {
  const next = name === 'grid' && state.cameras.length > 0 ? 'grid' : 'live'
  const changed = state.view !== next
  state.view = next
  shell.view = 'live'
  shell.mode = next === 'grid' ? 'grid' : 'single'
  replaceHash(liveHash(shell.mode))
  const any = state.cameras.length > 0
  renderPanes()
  renderSwitcher()
  renderBanner()
  if (next === 'grid') {
    stopTalk()
    closePad()
    stopStagePlayer()
    startGrid()
    return
  }
  stopGrid()
  if (any && (changed || !state.players.get('stage')) && !options.keepStream) {
    const camera = currentCamera()
    if (camera && (!deviceStatus(camera.device).known || camera.device.online)) requestStream()
    else if (camera) setStage('offline')
  }
}

export function setQuality(quality: string) {
  if (state.prefs.quality === quality) return
  state.prefs.quality = quality
  savePrefs()
  renderPrefsIntoForm()
  if (state.screen !== 'watch') return
  if (state.view === 'grid') startGrid()
  else requestStream()
}
