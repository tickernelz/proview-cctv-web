import { createApp } from 'vue'
import LiveTop from './LiveTop.vue'

const host = document.getElementById('live-top')
if (!host) throw new Error('index.html has no #live-top')
createApp(LiveTop).mount(host)
