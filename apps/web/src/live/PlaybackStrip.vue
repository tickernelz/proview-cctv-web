<script setup lang="ts">
import { computed, ref } from 'vue'
import { mb, modeOf, secondsBetween, span, stamp, type Clip } from './playback-clips'

const props = defineProps<{ files: Clip[]; day: string; current: Clip | null; busy: boolean }>()
const emit = defineEmits<{ pick: [file: Clip, offset: number] }>()

const HOURS = [0, 3, 6, 9, 12, 15, 18, 21]
const track = ref<HTMLElement>()

const segments = computed(() => {
  const base = stamp(props.day + ' 00:00:00')
  if (base === null) return []
  const drawn: { file: Clip; key: string; style: string; mode: string }[] = []
  for (const file of props.files) {
    const from = stamp(file.begin)
    const to = stamp(file.end)
    if (from === null || to === null) continue
    const left = ((from - base) / 86400000) * 100
    if (left < 0 || left > 100) continue
    const width = Math.min(Math.max(((to - from) / 86400000) * 100, 0.06), 100 - left)
    drawn.push({ file, key: file.name + file.begin, style: 'left:' + left + '%;width:' + width + '%', mode: modeOf(file) })
  }
  return drawn
})

function fileAt(clientX: number) {
  const box = track.value?.getBoundingClientRect()
  if (!box || !box.width || !props.files.length) return null
  const base = stamp(props.day + ' 00:00:00')
  if (base === null) return null
  const at = base + Math.min(Math.max((clientX - box.left) / box.width, 0), 1) * 86400000
  let next: Clip | null = null
  let prior: Clip | null = null
  for (const file of props.files) {
    const from = stamp(file.begin)
    const to = stamp(file.end)
    if (from === null || to === null) continue
    if (at >= from && at <= to) return { file, from, at }
    if (from > at && (next === null || from < (stamp(next.begin) as number))) next = file
    if (to < at && (prior === null || to > (stamp(prior.end) as number))) prior = file
  }
  const near = next || prior
  if (!near) return null
  const from = stamp(near.begin)
  return from === null ? null : { file: near, from, at }
}

function onClick(event: MouseEvent) {
  const hit = fileAt(event.clientX)
  if (!hit) return
  const duration = secondsBetween(hit.file.begin, hit.file.end)
  emit('pick', hit.file, Math.min(Math.max(0, Math.round((hit.at - hit.from) / 1000)), Math.max(duration - 1, 0)))
}

function onMove(event: MouseEvent) {
  if (!track.value) return
  const hit = fileAt(event.clientX)
  track.value.title = hit ? span(hit.file) + ' · ' + mb(hit.file.bytes) + ' · ' + modeOf(hit.file) : ''
}

function onLeave() {
  if (track.value) track.value.title = ''
}
</script>

<template>
  <div class="pb-strip">
    <div
      id="pb-timeline"
      ref="track"
      class="pb-timeline"
      role="group"
      aria-label="Recordings across the day; click to play from that time"
      :aria-busy="busy"
      @click="onClick"
      @mousemove="onMove"
      @mouseleave="onLeave"
    >
      <span v-for="hour in HOURS" :key="'t' + hour" class="pb-tick" :style="{ left: (hour * 100) / 24 + '%' }" aria-hidden="true" />
      <span
        v-for="segment in segments"
        :key="segment.key"
        v-memo="[segment.file === current]"
        class="pb-seg"
        :data-mode="segment.mode"
        :data-current="segment.file === current || null"
        :style="segment.style"
        aria-hidden="true"
      />
    </div>
    <div class="pb-hours" aria-hidden="true">
      <span v-for="hour in HOURS" :key="'h' + hour" :style="{ left: (hour * 100) / 24 + '%' }">{{ String(hour).padStart(2, '0') }}:00</span>
    </div>
    <p class="pb-legend"><span class="pb-key" data-mode="continuous" />Continuous<span class="pb-key" data-mode="motion" />Motion</p>
  </div>
</template>
