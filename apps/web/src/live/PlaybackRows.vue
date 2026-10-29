<script setup lang="ts">
import { computed } from 'vue'
import { mb, modeOf, span, type Clip } from './playback-clips'

const props = defineProps<{ files: Clip[]; current: Clip | null }>()
defineEmits<{ play: [file: Clip] }>()

const rows = computed(() =>
  props.files.map((file) => {
    const mode = modeOf(file)
    return { file, key: file.name + file.begin, mode, when: span(file), size: mb(file.bytes) + (mode === 'motion' ? ' · motion' : '') }
  }),
)
</script>

<template>
  <div id="pb-files" class="pb-list" data-testid="pb-files">
    <button
      v-for="row in rows"
      :key="row.key"
      v-memo="[row.file === current]"
      class="pb-row"
      type="button"
      :data-mode="row.mode"
      :aria-pressed="row.file === current"
      @click="$emit('play', row.file)"
    >
      <span class="pb-when">{{ row.when }}</span><span class="pb-size">{{ row.size }}</span>
    </button>
  </div>
</template>
