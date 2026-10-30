import { createApp } from 'vue'
import ImageSettings from './ImageSettings.vue'
import ImageLive from './ImageLive.vue'

const body = document.getElementById('set-body')
if (!body) throw new Error('index.html has no #set-body')
const host = document.createElement('section')
host.dataset.section = 'image'
host.setAttribute('aria-labelledby', 'h-image')
host.hidden = true
body.append(host)
createApp(ImageSettings, { host }).mount(host)

const bar = document.getElementById('live-bar')
const live = document.createElement('div')
live.className = 'live-bar'
if (bar) bar.after(live)
else document.body.append(live)
createApp(ImageLive).mount(live)
