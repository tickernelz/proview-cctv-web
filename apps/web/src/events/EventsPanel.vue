<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { IconButton } from '../components/ui/icon-button'
import { EVENTS_HASH } from '../shell/router'
import CameraPick from './CameraPick.vue'
import DayPicker from './DayPicker.vue'
import EventThumb from './EventThumb.vue'
import EventsState from './EventsState.vue'
import KindChips from './KindChips.vue'
import { KIND, ago, channelOf, clockOf, ev, groups, isNew, kindOf, playReason, playEvent, rows, summary, type Alarm } from './events'

const emit = defineEmits<{ close: [] }>()
const list = ref<HTMLElement>()

function label(entry: Alarm) {
  const kind = KIND[kindOf(entry)].label
  const reason = playReason(entry)
  return kind + ' at ' + clockOf(entry, true) + ', ' + ago(entry) + (reason ? '. ' + reason : '. Play from 5 s before')
}

watch(
  () => [rows.value, ev.focus.id] as const,
  async () => {
    if (!ev.focus.id || ev.focus.shown === ev.focus.id) return
    await nextTick()
    const row = list.value?.querySelector('[data-id="' + CSS.escape(ev.focus.id) + '"]')
    if (!row) return
    ev.focus.shown = ev.focus.id
    requestAnimationFrame(() => row.scrollIntoView({ block: 'nearest' }))
  },
)
</script>

<template>
  <div class="ev-head">
    <h2 id="alarm-title">Events</h2>
    <IconButton data-testid="alarm-close" data-close label="Close" icon="x" size="sm" side="left" @click="emit('close')" />
  </div>
  <div class="ev-tools">
    <CameraPick />
    <DayPicker />
  </div>
  <KindChips />
  <div ref="list" class="ev-list" :aria-busy="ev.phase === 'loading'">
    <EventsState compact />
    <section v-for="group in groups" :key="group.key" class="ev-section" :aria-label="group.label">
      <h3 class="ev-group">{{ group.label }}</h3>
      <ul class="ev-rows" role="list">
        <li v-for="entry in group.entries" :key="entry.id">
          <button
            type="button"
            :class="['alarm-row', 'ev-row', { focus: ev.focus.id && ev.focus.id === entry.id }]"
            data-testid="alarm-row"
            :data-id="entry.id"
            :aria-label="label(entry)"
            :aria-disabled="playReason(entry) ? 'true' : undefined"
            @click="playEvent(entry)"
          >
            <EventThumb :entry="entry" />
            <span class="ev-row-text">
              <span class="ev-kind">
                <Icon :name="KIND[kindOf(entry)].icon" small :class="'ev-ink-' + kindOf(entry)" />
                <span class="ev-kind-word">{{ KIND[kindOf(entry)].label }}</span>
                <span v-if="isNew(entry)" class="ev-new" data-testid="alarm-new">New</span>
              </span>
              <span class="ev-when"><time :datetime="entry.time">{{ clockOf(entry) }}</time> · {{ ago(entry) }}</span>
              <span v-if="channelOf(entry)" class="ev-sub">{{ channelOf(entry) }}</span>
              <span v-if="playReason(entry)" class="ev-sub">{{ playReason(entry) }}</span>
            </span>
          </button>
        </li>
      </ul>
    </section>
  </div>
  <div class="ev-foot">
    <span data-testid="alarm-status" role="status" :title="ev.timing">{{ summary }}</span>
    <a class="ev-open" data-testid="alarm-open-all" :href="EVENTS_HASH">Open all events<Icon name="chevron-right" small /></a>
  </div>
</template>
