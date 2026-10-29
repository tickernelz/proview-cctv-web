import { createApp } from 'vue'
import SettingsScreen from './SettingsScreen.vue'

const host = document.getElementById('settings')
if (!host) throw new Error('index.html has no #settings')
createApp(SettingsScreen).mount(host)
