import { createApp } from 'vue'
import LiveMain from './LiveMain.vue'

const host = document.getElementById('main')
if (!host) throw new Error('index.html has no #main')
createApp(LiveMain).mount(host)
