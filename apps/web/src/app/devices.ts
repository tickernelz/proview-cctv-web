import { state, type Device } from './state'

type StatusFields = Pick<Device, 'online' | 'online_status' | 'online_detail'>

export function deviceStatus(device: StatusFields | null | undefined) {
  if (!device) return { known: false, online: false, detail: '' }
  const status = String(device.online_status || '').toLowerCase()
  const known = status ? status !== 'unknown' : Boolean(device.online !== undefined)
  return { known, online: Boolean(device.online), detail: device.online_detail || '' }
}

export function statusLine(device: StatusFields | null | undefined) {
  const status = deviceStatus(device)
  if (status.detail) return status.detail
  return status.known ? (status.online ? 'cloud status online' : 'cloud status offline') : 'cloud status not read'
}

export function buildCameras() {
  state.cameras = []
  for (const device of state.devices) {
    const count = Math.max(1, device.channels || 1)
    for (let channel = 1; channel <= count; channel += 1) {
      state.cameras.push({
        device,
        channel,
        key: device.id + ':' + channel,
        name: count > 1 ? device.name + ' · channel ' + channel : device.name,
      })
    }
  }
}

export function selectedDevice() {
  return state.devices.find((device) => device.id === state.selectedId) || null
}

export function currentCamera() {
  const key = state.selectedId + ':' + state.channel
  return state.cameras.find((camera) => camera.key === key) || null
}
