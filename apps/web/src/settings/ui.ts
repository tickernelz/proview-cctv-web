import { reactive } from 'vue'

export const CAMERA_SECTIONS = ['camera', 'device', 'image', 'alarm', 'system']

export const settingsUi = reactive({
  reload: {} as Record<string, number>,
  devices: 0,
  devicesFrom: '',
})

export function reloadSection(id: string) {
  settingsUi.reload[id] = (settingsUi.reload[id] || 0) + 1
}

export function openDevices(from = '') {
  settingsUi.devicesFrom = from
  settingsUi.devices += 1
}
