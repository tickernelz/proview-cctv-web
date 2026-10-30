<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import AlarmStop from './AlarmStop.vue'
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

const start = computed(() => live.launchers.filter((item) => item.slot === 'start'))
const end = computed(() => live.launchers.filter((item) => item.slot === 'end'))
const qualityLabel = computed(() => 'Quality ' + (live.quality === 'sub' ? 'SD' : 'HD'))
const talkNote = computed(() => live.talk.note || (live.gate.intercom.ok ? '' : live.gate.intercom.why))
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

function clickSound() {
  if (live.sound !== 'blocked') live.actions.toggleMute()
}
const QUALITIES = [
  { value: 'main', name: 'HD', sub: 'the main stream' },
  { value: 'sub', name: 'SD', sub: 'the sub stream, lighter on the network' },
]

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
  <div class="well-meta">
    <p id="caption" data-testid="live-caption" role="status">
      <strong id="caption-name">{{ live.camera }}</strong><span id="caption-state" data-testid="live-state">{{ live.state }}</span>
    </p>
    <div class="right">
      <span id="caption-flag" data-testid="live-lag" :class="live.flag.tone"><span v-if="live.flag.tone === 'rec'" class="dot rec" />{{ live.flag.text }}</span>
      <span id="clock" class="mono" data-testid="live-clock">{{ live.clock }}</span>
    </div>
  </div>
  <div id="controls" class="controls" data-testid="live-controls" role="toolbar" aria-label="Stream controls" tabindex="-1">
    <button
      id="talk"
      data-testid="ctl-talk"
      :class="['ctl', { 'talk-on': live.talk.on }]"
      type="button"
      aria-keyshortcuts="T"
      :aria-pressed="live.talk.on"
      :disabled="!live.gate.intercom.ok"
      :title="live.gate.intercom.why"
      @pointerdown="pressTalk"
      @pointerup="live.actions.stopTalk()"
      @pointercancel="live.actions.stopTalk()"
      @lostpointercapture="live.actions.stopTalk()"
      @keydown="keyTalk($event, true)"
      @keyup="keyTalk($event, false)"
    >
      <Icon name="mic" /><span>Talk</span>
      <span id="level" class="level" aria-hidden="true" :hidden="!live.talk.on" :data-level="live.talk.level">
        <i v-for="bar in 5" :key="bar" :class="{ on: bar <= live.talk.level }" />
      </span>
    </button>
    <button
      id="mute"
      data-testid="ctl-mute"
      class="ctl"
      type="button"
      aria-keyshortcuts="M"
      :aria-pressed="!live.muted"
      :data-sound="live.sound"
      :title="soundTitle"
      @click="clickSound"
    >
      <Icon :name="live.muted ? 'speaker-off' : 'speaker'" /><span data-testid="sound-label">{{ soundLabel }}</span>
    </button>
    <span class="sep" />
    <button
      id="record"
      data-testid="ctl-record"
      :class="['ctl', { 'rec-on': live.record.on }]"
      type="button"
      aria-keyshortcuts="R"
      :aria-pressed="live.record.on"
      :aria-label="live.record.on ? 'Stop recording' : 'Record'"
      :disabled="live.record.disabled"
      @click="live.actions.toggleRecord()"
    >
      <Icon name="record" /><span :class="{ mono: live.record.on }">{{ live.record.on ? live.record.elapsed : 'Record' }}</span>
    </button>
    <button id="snapshot" data-testid="ctl-snapshot" class="ctl" type="button" aria-keyshortcuts="S" :disabled="live.snapshotDisabled" @click="live.actions.captureSnapshot()">
      <Icon name="camera" /><span>Snapshot</span>
    </button>
    <AlarmStop />
    <button
      v-for="item in start"
      :id="item.id"
      :key="item.id"
      :data-testid="item.id"
      class="ctl"
      type="button"
      :title="item.title"
      :aria-pressed="item.pressed ?? undefined"
      :aria-controls="item.controls"
      @click="item.onClick()"
    >
      <Icon :name="item.icon" /><span>{{ item.label }}</span>
    </button>
    <span class="spacer" />
    <button
      v-for="item in end"
      :id="item.id"
      :key="item.id"
      :data-testid="item.id"
      class="ctl"
      type="button"
      :title="item.title"
      :aria-pressed="item.pressed ?? undefined"
      :aria-controls="item.controls"
      @click="item.onClick()"
    >
      <Icon :name="item.icon" /><span>{{ item.label }}</span>
    </button>
    <button
      id="move"
      data-testid="ctl-move"
      class="ctl"
      type="button"
      aria-controls="pad"
      aria-keyshortcuts="V"
      :aria-pressed="live.padOpen"
      :disabled="!live.gate.move.ok"
      :title="live.gate.move.why"
      :aria-description="live.gate.move.why"
      @click="live.actions.togglePad()"
    >
      <Icon name="move" /><span>Move</span>
    </button>
    <DropdownMenu v-if="live.lens.shown" :modal="false">
      <DropdownMenuTrigger id="lens-pick" data-testid="lens-pick" class="pick" aria-keyshortcuts="L">
        <span id="lens-pick-label" data-testid="lens-label">{{ live.lens.label }}</span><Icon name="chevron-down" small />
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
    <DropdownMenu :modal="false">
      <DropdownMenuTrigger id="quality-pick" data-testid="quality-pick" class="pick" aria-keyshortcuts="Q">
        <span id="quality-pick-label">{{ qualityLabel }}</span><Icon name="chevron-down" small />
      </DropdownMenuTrigger>
      <DropdownMenuContent id="quality-menu" data-testid="quality-menu" aria-label="Quality">
        <DropdownMenuLabel class="menu-h">Quality · Q switches</DropdownMenuLabel>
        <DropdownMenuRadioGroup :model-value="live.quality" @update:model-value="live.actions.setQuality(String($event))">
          <DropdownMenuRadioItem v-for="option in QUALITIES" :key="option.value" data-testid="quality-option" v-bind="option" />
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
    <span class="sep" />
    <button id="fullscreen" data-testid="ctl-fullscreen" class="iconbtn" type="button" aria-label="Full screen" title="Full screen (F)" aria-keyshortcuts="F" @click="live.actions.toggleFullscreen()">
      <Icon name="expand" />
    </button>
  </div>
  <p v-if="talkNote" id="talk-note" data-testid="talk-note" :class="['talk-note', live.talk.phase === 'error' ? 'bad' : live.talk.phase === 'sending' ? 'live' : '']" role="status">{{ talkNote }}</p>
</template>
