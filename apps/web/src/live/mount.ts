import { createApp } from 'vue'
import LiveBar from './LiveBar.vue'

const host = document.getElementById('live-bar')
if (!host) throw new Error('index.html has no #live-bar')
createApp(LiveBar).mount(host)
