import { createApp } from 'vue'
import AlarmSettings from './AlarmSettings.vue'

const body = document.getElementById('set-body')
if (!body) throw new Error('index.html has no #set-body')
const host = document.createElement('section')
host.dataset.section = 'alarm'
host.setAttribute('aria-labelledby', 'h-alarm')
host.hidden = true
body.append(host)
createApp(AlarmSettings, { host }).mount(host)
