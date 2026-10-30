import { reactive } from 'vue'

export const PLAYBACK_NOTE = 'Talk and Move are off while a recording plays. Go back to live to use them.'

export const QUALITIES = [
  { value: 'main', name: 'HD', sub: 'the main stream' },
  { value: 'sub', name: 'SD', sub: 'the sub stream, lighter on the network' },
]

export const alarmStopNote = reactive({ text: '', bad: false })
