import './tailwind.css'
import './tokens.css'
import './styles/base.css'
import './styles/live.css'
import './styles/grid.css'
import './styles/playback.css'
import './styles/events.css'
import './styles/settings.css'
import './styles/signin.css'
import './shell/shell.css'
import './app/keys'
import { createApp } from 'vue'
import App from './shell/App.vue'
import { boot } from './app/boot'

createApp(App).mount('#app')
boot()
