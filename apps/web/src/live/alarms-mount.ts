import { createApp } from 'vue'
import AlarmCenter from './AlarmCenter.vue'

const host = document.createElement('span')
host.className = 'alarm-host'
const topbar = document.querySelector('#watch .topbar')
const settings = document.getElementById('open-settings')
if (topbar && settings) topbar.insertBefore(host, settings)
else document.body.append(host)
createApp(AlarmCenter).mount(host)
