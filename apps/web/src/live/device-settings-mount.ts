import { createApp } from 'vue'
import DeviceSettings from './DeviceSettings.vue'

const body = document.getElementById('set-body')
if (!body) throw new Error('index.html has no #set-body')
const host = document.createElement('section')
host.dataset.section = 'device'
host.setAttribute('aria-labelledby', 'h-device')
host.hidden = true
body.append(host)
createApp(DeviceSettings, { host }).mount(host)
