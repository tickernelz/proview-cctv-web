<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import AlarmStop from './AlarmStop.vue'
import MoreMenu from './stage/MoreMenu.vue'
import PlaybackControls from './timeline/PlaybackControls.vue'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu'
import { live } from './store'
import { PLAYBACK_NOTE, QUALITIES } from './stage/notes'
import { shell } from '../shell/layout'

const quality = computed(() => (live.quality === 'sub' ? 'SD' : 'HD'))
const SOUND_TITLES: Record<string, string> = {
  waiting: 'Sound is on, waiting for the camera audio',
  playing: 'Sound is on',
  silent: 'Sound is on, but the camera is sending no audio',
  blocked: 'The browser holds sound until you click or press a key on this page',
  unplayable: 'The camera sends audio in a codec this page cannot play',
  unsupported: 'This browser has no Web Audio, so the camera audio cannot play',
}
const SOUND_LABELS: Record<string, string> = {
  silent: 'No sound',
  blocked: 'Click to enable sound',
  unplayable: 'Sound not playable',
  unsupported: 'No Web Audio',
}
const soundTitle = computed(() => (live.muted ? 'Sound is off' : SOUND_TITLES[live.sound] || 'Sound is on') + ' (M)')
const soundLabel = computed(() => (live.muted ? 'Sound' : SOUND_LABELS[live.sound] || 'Sound'))

const playback = computed(() => live.mode === 'playback')
const talkOff = computed(() => playback.value || !live.gate.intercom.ok)
const moveOff = computed(() => playback.value || !live.gate.move.ok)
const talkWhy = computed(() => (playback.value ? 'playback-note' : live.gate.intercom.ok ? undefined : 'talk-note'))
const moveWhy = computed(() => (playback.value ? 'playback-note' : live.gate.move.ok ? undefined : 'move-note'))

const MIN_SHEET = 240
const bar = ref<HTMLElement>()
let observer: ResizeObserver | null = null

function clearDock() {
  const root = document.documentElement.style
  const dock = bar.value
  if (!dock || shell.narrow || !dock.getClientRects().length) return root.removeProperty('--dock-clear')
  const clear = Math.max(0, Math.ceil(window.innerHeight - dock.getBoundingClientRect().top))
  const header = document.querySelector('.app-header')?.getBoundingClientRect().bottom || 0
  if (window.innerHeight - header - clear < MIN_SHEET) return root.removeProperty('--dock-clear')
  root.setProperty('--dock-clear', clear + 'px')
}

onMounted(() => {
  observer = new ResizeObserver(clearDock)
  for (let node = bar.value as Element | null; node; node = node.nextElementSibling) observer.observe(node)
  window.addEventListener('resize', clearDock)
  bar.value?.closest('.viewer')?.addEventListener('scroll', clearDock, { passive: true })
  clearDock()
})

onBeforeUnmount(() => {
  observer?.disconnect()
  window.removeEventListener('resize', clearDock)
  bar.value?.closest('.viewer')?.removeEventListener('scroll', clearDock)
  document.documentElement.style.removeProperty('--dock-clear')
})

watch(() => shell.narrow, clearDock)

function clickSound() {
  if (live.sound !== 'blocked') live.actions.toggleMute()
}

function pressTalk(event: PointerEvent) {
  if (event.button !== 0) return
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  live.actions.startTalk()
}

function keyTalk(event: KeyboardEvent, down: boolean) {
  if (event.key !== ' ' && event.key !== 'Enter') return
  event.preventDefault()
  if (!down) live.actions.stopTalk()
  else if (!event.repeat) live.actions.startTalk()
}
</script>

<template>
  <div id="controls" ref="bar" :class="['controls', 'dock', { 'dock-row': shell.narrow && !playback }]" data-testid="live-controls" role="toolbar" aria-label="Stream controls" tabindex="-1">
    <button
      id="talk"
      data-testid="ctl-talk"
      :class="['dk', { 'talk-on': live.talk.on }]"
      type="button"
      aria-keyshortcuts="T"
      :aria-pressed="live.talk.on"
      :aria-describedby="talkWhy"
      :disabled="talkOff"
      :title="playback ? PLAYBACK_NOTE : live.gate.intercom.ok ? 'Hold to talk (T)' : live.gate.intercom.why"
      @pointerdown="pressTalk"
      @pointerup="live.actions.stopTalk()"
      @pointercancel="live.actions.stopTalk()"
      @lostpointercapture="live.actions.stopTalk()"
      @keydown="keyTalk($event, true)"
      @keyup="keyTalk($event, false)"
    >
      <span class="dk-ico">
        <Icon name="mic" />
        <span id="level" class="level" aria-hidden="true" :hidden="!live.talk.on" :data-level="live.talk.level">
          <i v-for="bar in 5" :key="bar" :class="{ on: bar <= live.talk.level }" />
        </span>
      </span>
      <span class="dk-label">Talk</span>
    </button>
    <PlaybackControls v-if="playback" />
    <template v-else>
    <button id="mute" data-testid="ctl-mute" class="dk" type="button" aria-keyshortcuts="M" :aria-pressed="!live.muted" :data-sound="live.sound" :title="soundTitle" @click="clickSound">
      <Icon :name="live.muted ? 'speaker-off' : 'speaker'" /><span class="dk-label" data-testid="sound-label">{{ soundLabel }}</span>
    </button>
    <span class="sep" aria-hidden="true" />
    <button id="snapshot" data-testid="ctl-snapshot" class="dk" type="button" aria-keyshortcuts="S" title="Snapshot (S)" :disabled="live.snapshotDisabled" @click="live.actions.captureSnapshot()">
      <Icon name="camera" /><span class="dk-label">Snapshot</span>
    </button>
    <button
      v-if="!shell.narrow"
      id="record"
      data-testid="ctl-record"
      :class="['dk', 'rec', { 'rec-on': live.record.on }]"
      type="button"
      aria-keyshortcuts="R"
      title="Record a clip (R)"
      :aria-pressed="live.record.on"
      :aria-label="live.record.on ? 'Stop recording' : 'Record'"
      :disabled="live.record.disabled"
      @click="live.actions.toggleRecord()"
    >
      <Icon name="record" /><span :class="['dk-label', { mono: live.record.on }]">{{ live.record.on ? live.record.elapsed : 'Record' }}</span>
    </button>
    </template>
    <button
      id="move"
      data-testid="ctl-move"
      class="dk"
      type="button"
      aria-controls="pad"
      aria-keyshortcuts="V"
      :title="playback ? PLAYBACK_NOTE : live.gate.move.ok ? 'Move the camera (V)' : live.gate.move.why"
      :aria-pressed="live.padOpen"
      :disabled="moveOff"
      :aria-describedby="moveWhy"
      @click="live.actions.togglePad()"
    >
      <Icon name="move" /><span class="dk-label">Move</span>
    </button>
    <AlarmStop />
    <span class="spacer" />
    <DropdownMenu v-if="live.lens.shown && !shell.narrow" :modal="false">
      <DropdownMenuTrigger id="lens-pick" data-testid="lens-pick" class="dk" aria-keyshortcuts="L" title="Lens (L)">
        <Icon name="layers" /><span class="dk-label">Lens</span>
      </DropdownMenuTrigger>
      <DropdownMenuContent id="lens-menu" data-testid="lens-menu" aria-label="Lens">
        <DropdownMenuLabel class="menu-h">Lens · L cycles</DropdownMenuLabel>
        <DropdownMenuRadioGroup :model-value="live.lens.value" @update:model-value="live.actions.chooseLens(String($event))">
          <DropdownMenuRadioItem v-for="option in live.lens.options" :key="option.value" data-testid="lens-option" v-bind="option" />
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator class="menu-sep" />
        <p class="menu-note">{{ live.lens.note }}</p>
      </DropdownMenuContent>
    </DropdownMenu>
    <DropdownMenu v-if="!playback && !shell.narrow" :modal="false">
      <DropdownMenuTrigger id="quality-pick" data-testid="quality-pick" class="dk" aria-keyshortcuts="Q" title="Quality (Q)">
        <span class="dk-label">Quality</span> <span class="dk-q" data-testid="quality-value">{{ quality }}</span>
      </DropdownMenuTrigger>
      <DropdownMenuContent id="quality-menu" data-testid="quality-menu" aria-label="Quality">
        <DropdownMenuLabel class="menu-h">Quality · Q switches</DropdownMenuLabel>
        <DropdownMenuRadioGroup :model-value="live.quality" @update:model-value="live.actions.setQuality(String($event))">
          <DropdownMenuRadioItem v-for="option in QUALITIES" :key="option.value" data-testid="quality-option" v-bind="option" />
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
    <MoreMenu />
    <button v-if="!shell.narrow" id="fullscreen" data-testid="ctl-fullscreen" class="dk" type="button" title="Full screen (F)" aria-keyshortcuts="F" @click="live.actions.toggleFullscreen()">
      <Icon name="expand" /><span class="dk-label">Full screen</span>
    </button>
  </div>
</template>
