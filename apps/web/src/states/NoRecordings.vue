<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import StageButton from './StageButton.vue'
import { dayLabel } from './time'

const props = withDefaults(defineProps<{ day: number | Date; variant?: 'stage' | 'track'; canNext?: boolean; canLive?: boolean }>(), { variant: 'stage', canNext: false, canLive: true })
const emit = defineEmits<{ nextDay: []; backToLive: [] }>()

const sentence = computed(() => 'No recording on ' + dayLabel(props.day) + '.')
</script>

<template>
  <p v-if="variant === 'track'" class="flex h-full items-center justify-center px-3 text-center text-13 text-ink-3" role="status">{{ sentence }}</p>
  <div v-else class="absolute inset-0 flex flex-col items-center justify-center gap-2.5 px-6 text-center text-stage-ink" role="status">
    <Icon name="calendar" class="size-8 text-stage-ink-2" />
    <p class="text-17 font-semibold text-balance">{{ sentence }}</p>
    <p class="text-14 text-stage-ink-2">{{ canNext ? 'Pick another day, or go back to live.' : 'Pick an earlier day, or go back to live.' }}</p>
    <div v-if="canNext || canLive" class="mt-1 flex flex-wrap justify-center gap-2">
      <StageButton v-if="canNext" icon-end="chevron-right" @click="emit('nextDay')">Next day</StageButton>
      <StageButton v-if="canLive" @click="emit('backToLive')">Back to live</StageButton>
    </div>
  </div>
</template>
