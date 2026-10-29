import { createApp } from 'vue'
import DevicesSheet from './DevicesSheet.vue'

const host = document.createElement('div')
document.body.append(host)
createApp(DevicesSheet).mount(host)
