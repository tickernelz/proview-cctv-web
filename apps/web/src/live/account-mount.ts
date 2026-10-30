import { createApp } from 'vue'
import AccountPanel from './AccountPanel.vue'

const body = document.getElementById('set-body')
if (!body) throw new Error('index.html has no #set-body')
const host = document.createElement('section')
host.dataset.section = 'account'
host.setAttribute('aria-labelledby', 'h-account')
host.hidden = true
body.append(host)
createApp(AccountPanel, { host }).mount(host)
