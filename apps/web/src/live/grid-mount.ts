import { createApp } from 'vue'
import GridView from './GridView.vue'

const host = document.getElementById('grid')
if (!host) throw new Error('index.html has no #grid')
createApp(GridView).mount(host)
