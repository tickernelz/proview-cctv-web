<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { mb, modeOf, span, type Clip } from './playback-clips'

type Thumb = { name: string; image?: string | null; error?: string | null; source?: string | null }

const props = defineProps<{ files: Clip[]; current: Clip | null; thumbs?: Record<string, Thumb> }>()
const emit = defineEmits<{ play: [file: Clip]; need: [files: Clip[]] }>()

const box = ref<HTMLElement>()
let watcher: IntersectionObserver | null = null

const rows = computed(() =>
  props.files.map((file) => {
    const mode = modeOf(file)
    return { file, key: file.name + file.begin, mode, when: span(file), size: mb(file.bytes) + (mode === 'continuous' ? '' : ' · ' + mode) }
  }),
)

function thumbOf(file: Clip) {
  return props.thumbs ? props.thumbs[file.name] : undefined
}

function observe() {
  if (!watcher || !box.value) return
  watcher.disconnect()
  for (const element of Array.from(box.value.querySelectorAll('.pb-row'))) watcher.observe(element)
}

onMounted(() => {
  if (typeof IntersectionObserver === 'undefined') return
  watcher = new IntersectionObserver(
    (entries) => {
      const seen: Clip[] = []
      for (const entry of entries) {
        if (!entry.isIntersecting) continue
        const index = Number((entry.target as HTMLElement).dataset.index)
        const file = props.files[index]
        if (file && !thumbOf(file)) seen.push(file)
      }
      if (seen.length) emit('need', seen)
    },
    { root: box.value, rootMargin: '120px 0px' },
  )
  observe()
})

watch(() => props.files, () => requestAnimationFrame(observe))

onBeforeUnmount(() => watcher?.disconnect())
</script>

<template>
  <div id="pb-files" ref="box" class="pb-list" data-testid="pb-files">
    <button
      v-for="(row, index) in rows"
      :key="row.key"
      v-memo="[row.file === current, thumbOf(row.file)?.image, thumbOf(row.file)?.error]"
      class="pb-row"
      type="button"
      :data-index="index"
      :data-mode="row.mode"
      :aria-pressed="row.file === current"
      @click="$emit('play', row.file)"
    >
      <span class="pb-thumb" :data-state="thumbOf(row.file)?.image ? 'ok' : thumbOf(row.file)?.error ? 'none' : 'wait'" :title="thumbOf(row.file)?.error || ''"><img v-if="thumbOf(row.file)?.image" :src="thumbOf(row.file)?.image || ''" alt="" loading="lazy" decoding="async"></span>
      <span class="pb-when">{{ row.when }}</span><span class="pb-size">{{ row.size }}</span>
    </button>
  </div>
</template>
