import { createApp } from 'vue'
import RecordingsPanel from './RecordingsPanel.vue'

const host = document.createElement('div')
document.body.append(host)
createApp(RecordingsPanel).mount(host)
