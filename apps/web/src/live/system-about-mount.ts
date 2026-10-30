import { createApp } from 'vue'
import SystemAbout from './SystemAbout.vue'

const body = document.getElementById('set-body')
if (!body) throw new Error('index.html has no #set-body')
const host = document.createElement('section')
host.dataset.section = 'system'
host.setAttribute('aria-labelledby', 'h-system')
host.hidden = true
body.append(host)
createApp(SystemAbout, { host }).mount(host)
