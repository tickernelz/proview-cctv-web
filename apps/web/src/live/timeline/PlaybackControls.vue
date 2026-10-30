<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../../components/Icon.vue'
import { SegmentedControl } from '../../components/ui/segmented-control'
import { live } from '../store'
import { downloadFile, setSpeed, snapshot, SPEEDS, stepFrame, tl, togglePause, toggleSound } from './engine'

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
    <button class="pbc-btn" type="button" data-testid="pb-pause" aria-keyshortcuts="Space" :aria-pressed="pb.paused" @click="togglePause">
      <Icon :name="pb.paused ? 'play' : 'pause'" /><span>{{ pb.paused ? 'Play' : 'Pause' }}</span>
    </button>
    <button class="pbc-btn pbc-frame" type="button" data-testid="pb-frame-back" aria-label="Previous frame" aria-keyshortcuts="ArrowLeft" :disabled="!pb.picture" @click="stepFrame(-1)">
      <Icon name="step-back" /><span>Frame</span>
    </button>
    <button class="pbc-btn pbc-frame" type="button" data-testid="pb-frame-next" aria-label="Next frame" aria-keyshortcuts="ArrowRight" :disabled="!pb.picture" @click="stepFrame(1)">
      <Icon name="step-forward" /><span>Frame</span>
    </button>
    <span class="pbc-sep" aria-hidden="true" />
    <div class="pbc-speed">
      <SegmentedControl data-testid="pb-speed" :options="SPEED_OPTIONS" :model-value="String(pb.speed)" label="Playback speed" size="sm" @update:model-value="(value) => setSpeed(Number(value))" />
      <span class="pbc-caption" aria-hidden="true">Speed</span>
    </div>
    <button class="pbc-btn pbc-speed-cycle" type="button" data-testid="pb-speed-cycle" :aria-label="'Playback speed ' + speedText + ', change'" @click="cycleSpeed">
      <span class="pbc-speed-value">{{ speedText }}</span><span>Speed</span>
    </button>
    <span class="pbc-sep" aria-hidden="true" />
    <button class="pbc-btn" type="button" data-testid="pb-sound" :aria-pressed="tl.sound" :disabled="pb.speed !== 1" @click="toggleSound">
      <Icon :name="tl.sound ? 'speaker' : 'speaker-off'" /><span>{{ tl.sound ? 'Mute' : 'Sound' }}</span>
    </button>
    <button class="pbc-btn" type="button" data-testid="pb-snapshot" aria-keyshortcuts="S" :disabled="!pb.picture" @click="snapshot">
      <Icon name="camera" /><span>Snapshot</span>
    </button>
    <button id="pb-download" class="pbc-btn" type="button" data-testid="pb-download" :disabled="!tl.file" @click="download">
      <Icon name="download" /><span>Download clip</span>
    </button>
  </div>
</template>
