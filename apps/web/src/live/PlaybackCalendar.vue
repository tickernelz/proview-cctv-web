<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'

type Month = { phase: 'loading' | 'ok' | 'error'; days: number[]; error: string }

const props = defineProps<{ device: string; day: string; open: boolean }>()
const emit = defineEmits<{ pick: [day: string]; close: [] }>()

const NAMES = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
const WEEK = ['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su']

const shown = ref(props.day.slice(0, 7))
const months = reactive<Record<string, Month>>({})

const pad = (value: number) => String(value).padStart(2, '0')
const keyOf = (device: string, month: string) => device + '|' + month

const parts = computed(() => {
  const [year, month] = shown.value.split('-').map(Number)
  return { year, month }
})
const label = computed(() => NAMES[parts.value.month - 1] + ' ' + parts.value.year)
const current = computed(() => months[keyOf(props.device, shown.value)])
const cells = computed(() => {
  const { year, month } = parts.value
  const first = (new Date(year, month - 1, 1).getDay() + 6) % 7
  const count = new Date(year, month, 0).getDate()
  const marked = new Set(current.value && current.value.phase === 'ok' ? current.value.days : [])
  const out: { key: string; day: number; date: string; has: boolean }[] = []
  for (let blank = 0; blank < first; blank += 1) out.push({ key: 'b' + blank, day: 0, date: '', has: false })
  for (let day = 1; day <= count; day += 1) {
    const date = year + '-' + pad(month) + '-' + pad(day)
    out.push({ key: date, day, date, has: marked.has(day) })
  }
  return out
})
const note = computed(() => {
  const month = current.value
  if (!month || month.phase === 'loading') return 'Asking the camera which days hold recordings…'
  if (month.phase === 'error') return month.error
  if (!month.days.length) return 'No recordings on the camera in ' + label.value + '.'
  return month.days.length + (month.days.length === 1 ? ' day' : ' days') + ' with recordings in ' + label.value + '.'
})

async function load(force = false) {
  if (!props.device || !props.open) return
  const key = keyOf(props.device, shown.value)
  if (!force && months[key] && months[key].phase !== 'error') return
  months[key] = { phase: 'loading', days: [], error: '' }
  const { year, month } = parts.value
  try {
    const body = await api<{ days?: number[] }>(
      '/api/playback/calendar?device=' + encodeURIComponent(props.device) + '&year=' + year + '&month=' + month,
    )
    months[key] = { phase: 'ok', days: (body && body.days) || [], error: '' }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    months[key] = { phase: 'error', days: [], error: 'The calendar did not load: ' + message }
  }
}

function move(delta: number) {
  const { year, month } = parts.value
  const moved = new Date(year, month - 1 + delta, 1)
  shown.value = moved.getFullYear() + '-' + pad(moved.getMonth() + 1)
}

function pick(date: string) {
  emit('pick', date)
  emit('close')
}

watch(() => [props.open, props.device, shown.value], () => load())
watch(
  () => props.day,
  (day) => {
    if (day) shown.value = day.slice(0, 7)
  },
)
</script>

<template>
  <div v-if="open" id="pb-calendar" class="pb-calendar" role="dialog" aria-label="Days with recordings" data-testid="pb-calendar" @keydown.esc.stop.prevent="emit('close')">
    <div class="pb-cal-head">
      <button class="iconbtn" type="button" aria-label="Previous month" title="Previous month" @click="move(-1)"><Icon name="chevron-left" /></button>
      <strong class="pb-cal-month" data-testid="pb-cal-month">{{ label }}</strong>
      <button class="iconbtn" type="button" aria-label="Next month" title="Next month" @click="move(1)"><Icon name="chevron-right" /></button>
    </div>
    <div class="pb-cal-grid" role="grid">
      <span v-for="name in WEEK" :key="name" class="pb-cal-week" aria-hidden="true">{{ name }}</span>
      <template v-for="cell in cells" :key="cell.key">
        <span v-if="!cell.day" aria-hidden="true" />
        <button
          v-else
          class="pb-cal-day"
          type="button"
          :data-has="cell.has || null"
          :aria-pressed="cell.date === day"
          :aria-label="cell.date + (cell.has ? ', has recordings' : ', no recordings')"
          @click="pick(cell.date)"
        >{{ cell.day }}</button>
      </template>
    </div>
    <p :class="['pb-cal-note', { bad: current && current.phase === 'error' }]" data-testid="pb-cal-note" role="status">{{ note }}</p>
  </div>
</template>
