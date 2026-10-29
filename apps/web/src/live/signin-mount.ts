import { createApp } from 'vue'
import SignInScreen from './SignInScreen.vue'

const host = document.getElementById('signin-main')
if (!host) throw new Error('index.html has no #signin-main')
createApp(SignInScreen).mount(host)
