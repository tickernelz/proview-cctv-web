<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../../components/Icon.vue'
import { SegmentedControl } from '../../components/ui/segmented-control'
import { live } from '../store'
import { backToLive, downloadFile, setSpeed, snapshot, SPEEDS, stepFrame, tl, togglePause, toggleSound } from './engine'

const SPEED_OPTIONS = SPEEDS.map((value) => ({ value: String(value), label: value + 'x', testid: 'pb-speed-' + value }))

const pb = live.playback
const speedText = computed(() => pb.speed + 'x')

function cycleSpeed() {
  const index = SPEEDS.indexOf(pb.speed)
  setSpeed(SPEEDS[(index + 1) % SPEEDS.length])
}

function download() {
  if (tl.file) downloadFile(tl.file)
}
</script>

<template>
  <div class="pbc" data-testid="playback-controls" role="group" aria-label="Playback">
    <button class="dk" type="button" data-testid="pb-pause" aria-keyshortcuts="Space" :aria-pressed="pb.paused" @click="togglePause">
      <Icon :name="pb.paused ? 'play' : 'pause'" /><span class="dk-label">{{ pb.paused ? 'Play' : 'Pause' }}</span>
    </button>
    <button class="dk pbc-frame" type="button" data-testid="pb-frame-back" aria-label="Previous frame" aria-keyshortcuts="ArrowLeft" :disabled="!pb.picture" @click="stepFrame(-1)">
      <Icon name="step-back" /><span class="dk-label">Frame</span>
    </button>
    <button class="dk pbc-frame" type="button" data-testid="pb-frame-next" aria-label="Next frame" aria-keyshortcuts="ArrowRight" :disabled="!pb.picture" @click="stepFrame(1)">
      <Icon name="step-forward" /><span class="dk-label">Frame</span>
    </button>
    <span class="sep" aria-hidden="true" />
    <div class="pbc-speed">
      <SegmentedControl data-testid="pb-speed" :options="SPEED_OPTIONS" :model-value="String(pb.speed)" label="Playback speed" size="sm" @update:model-value="(value) => setSpeed(Number(value))" />
      <span class="dk-label pbc-caption" aria-hidden="true">Speed</span>
    </div>
    <button class="dk pbc-speed-cycle" type="button" data-testid="pb-speed-cycle" :aria-label="'Playback speed ' + speedText + ', change'" @click="cycleSpeed">
      <span class="pbc-speed-value">{{ speedText }}</span><span class="dk-label">Speed</span>
    </button>
    <span class="sep" aria-hidden="true" />
    <button class="dk" type="button" data-testid="pb-sound" :aria-pressed="tl.sound" :disabled="pb.speed !== 1" @click="toggleSound">
      <Icon :name="tl.sound ? 'speaker' : 'speaker-off'" /><span class="dk-label">{{ tl.sound ? 'Mute' : 'Sound' }}</span>
    </button>
    <button class="dk" type="button" data-testid="pb-snapshot" aria-keyshortcuts="S" :disabled="!pb.picture" @click="snapshot">
      <Icon name="camera" /><span class="dk-label">Snapshot</span>
    </button>
    <button id="pb-download" class="dk" type="button" data-testid="pb-download" :disabled="!tl.file" @click="download">
      <Icon name="download" /><span class="dk-label">Download clip</span>
    </button>
    <button class="dk pbc-live" type="button" data-testid="pb-back-live" @click="backToLive">
      <Icon name="radio" /><span class="dk-label">Back to live</span>
    </button>
  </div>
</template>
