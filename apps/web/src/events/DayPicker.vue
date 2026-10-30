<script setup lang="ts">
import { computed, ref } from 'vue'
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from 'reka-ui'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { IconButton } from '../components/ui/icon-button'
import { RANGES, dayLong, ev, monthLabel, openMonth, plural, rangeLabel, setRange, stepMonth, type Range } from './events'

const open = ref(false)
const label = computed(() => rangeLabel())
const cells = computed(() => {
  const [year, month] = ev.month.month.split('-').map(Number)
  if (!year || !month) return [] as { key: string; day: string; label: number; count: number }[]
  const lead = (new Date(year, month - 1, 1).getDay() + 6) % 7
  const length = new Date(year, month, 0).getDate()
  const counts = new Map(ev.month.days.map((item) => [item.day, item.count]))
  const list: { key: string; day: string; label: number; count: number }[] = []
  for (let index = 0; index < lead; index += 1) list.push({ key: 'lead-' + index, day: '', label: 0, count: 0 })
  for (let date = 1; date <= length; date += 1) {
    const day = ev.month.month + '-' + String(date).padStart(2, '0')
    list.push({ key: day, day, label: date, count: counts.get(day) || 0 })
  }
  return list
})

function toggle(next: boolean) {
  open.value = next
  if (next) openMonth()
}

function choose(range: Range, day = '') {
  setRange(range, day)
  open.value = false
}
</script>

<template>
  <PopoverRoot :open="open" @update:open="toggle">
    <PopoverTrigger as-child>
      <Button size="sm" icon="calendar" icon-end="chevron-down" data-testid="alarm-day-picker" :disabled="!ev.query.serial" :aria-label="'Day: ' + label">{{ label }}</Button>
    </PopoverTrigger>
    <PopoverPortal>
      <PopoverContent class="menu-panel ev-pop" data-testid="alarm-day-menu" align="end" :side-offset="6" :collision-padding="8">
        <div class="ev-ranges" role="group" aria-label="Range">
          <button
            v-for="range in RANGES"
            :key="range.id"
            type="button"
            class="ev-range"
            data-testid="alarm-range"
            :data-range="range.id"
            :aria-pressed="ev.query.range === range.id ? 'true' : 'false'"
            @click="choose(range.id)"
          >
            <span>{{ range.label }}</span>
            <Icon v-if="ev.query.range === range.id" name="check" small />
          </button>
        </div>
        <div id="alarm-month" class="alarm-month" data-testid="alarm-month">
          <div class="alarm-month-head">
            <IconButton label="Previous month" icon="chevron-left" size="sm" data-testid="alarm-month-prev" @click="stepMonth(-1)" />
            <strong>{{ monthLabel(ev.month.month) }}</strong>
            <IconButton label="Next month" icon="chevron-right" size="sm" data-testid="alarm-month-next" @click="stepMonth(1)" />
          </div>
          <p v-if="ev.month.phase === 'error'" class="err" role="alert"><Icon name="warning" small /><span>{{ ev.month.error }}</span></p>
          <p v-else-if="ev.month.phase === 'loading'" class="alarm-month-note">Loading the days with events…</p>
          <p v-else-if="ev.month.phase === 'ok' && !ev.month.days.length" class="alarm-month-note" data-testid="alarm-month-empty">No events in {{ monthLabel(ev.month.month) }}.</p>
          <p v-else class="alarm-month-note">Days with events show their count.</p>
          <div class="alarm-days" :aria-busy="ev.month.phase === 'loading'">
            <span v-for="name in ['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su']" :key="name" class="alarm-dow" aria-hidden="true">{{ name }}</span>
            <template v-for="cell in cells" :key="cell.key">
              <span v-if="!cell.day" aria-hidden="true" />
              <button
                v-else
                type="button"
                :class="['alarm-day', { has: cell.count, on: ev.query.range === 'day' && cell.day === ev.query.day }]"
                data-testid="alarm-day"
                :data-day="cell.day"
                :disabled="!cell.count"
                :aria-label="dayLong(cell.day) + ', ' + (cell.count ? plural(cell.count) : 'no events')"
                :aria-pressed="ev.query.range === 'day' && cell.day === ev.query.day ? 'true' : 'false'"
                @click="choose('day', cell.day)"
              >
                <span>{{ cell.label }}</span><small v-if="cell.count">{{ cell.count }}</small>
              </button>
            </template>
          </div>
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
