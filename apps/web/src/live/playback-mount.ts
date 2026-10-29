import { createApp } from 'vue'
import PlaybackScreen from './PlaybackScreen.vue'

const host = document.createElement('div')
document.body.append(host)
createApp(PlaybackScreen).mount(host)
