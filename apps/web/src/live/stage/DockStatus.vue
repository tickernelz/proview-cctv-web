<script setup lang="ts">
import { computed } from 'vue'
import ImageLive from '../ImageLive.vue'
import { live } from '../store'
import { alarmStopNote, PLAYBACK_NOTE } from './notes'

const playback = computed(() => live.mode === 'playback')
const talkNote = computed(() => (playback.value ? '' : live.talk.note || (live.gate.intercom.ok ? '' : live.gate.intercom.why)))
const talkTone = computed(() => (live.talk.phase === 'error' ? 'bad' : live.talk.phase === 'sending' ? 'live' : ''))
const moveNote = computed(() => {
  if (live.gate.move.ok || live.mode === 'playback') return ''
  const why = (live.gate.move.why || '').trim()
  return why ? 'Move is off: ' + why : 'Move is off for this camera.'
})
</script>

<template>
  <div id="dock-status" class="dock-status" data-testid="dock-status">
    <p id="caption" class="sr" data-testid="live-caption" role="status">
      <span id="caption-state" data-testid="live-state">{{ live.state }}</span>
      <span id="caption-flag" data-testid="live-lag" :class="live.flag.tone">{{ live.flag.text }}</span>
    </p>
    <p id="control-status" class="dock-note" data-testid="control-status" role="status">{{ live.pad.status }}</p>
    <p v-if="talkNote" id="talk-note" data-testid="talk-note" :class="['dock-note', talkTone]" role="status">{{ talkNote }}</p>
    <p v-if="playback" id="playback-note" class="dock-note" data-testid="playback-note">{{ PLAYBACK_NOTE }}</p>
    <p v-if="moveNote" id="move-note" class="dock-note" data-testid="move-note">{{ moveNote }}</p>
    <p v-if="alarmStopNote.text" id="alarm-stop-note" data-testid="ctl-alarm-stop-note" :class="['dock-note', { bad: alarmStopNote.bad }]" role="status">{{ alarmStopNote.text }}</p>
    <p v-if="live.settings.lightStatus" class="dock-note" data-testid="live-light-status" role="status">{{ live.settings.lightStatus }}</p>
    <ImageLive />
  </div>
</template>
