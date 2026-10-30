<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '../components/ui/sheet'
import { shell } from '../shell/layout'
import CameraPick from './CameraPick.vue'
import DayPicker from './DayPicker.vue'
import EventPreview from './EventPreview.vue'
import EventThumb from './EventThumb.vue'
import EventsManage from './EventsManage.vue'
import EventsNotify from './EventsNotify.vue'
import EventsState from './EventsState.vue'
import KindChips from './KindChips.vue'
import { KIND, ago, channelOf, clockOf, ev, groups, isNew, kindOf, nameOf, plural, rows, setManage, shown, summary, togglePick, type Alarm } from './events'

const wideQuery = window.matchMedia('(min-width: 1024px)')
const wide = ref(wideQuery.matches)
const onWide = (event: MediaQueryListEvent) => {
  wide.value = event.matches
  if (event.matches) sheetOpen.value = false
}
wideQuery.addEventListener('change', onWide)
onBeforeUnmount(() => wideQuery.removeEventListener('change', onWide))

const list = ref<HTMLElement>()
const sheetOpen = ref(false)
const current = computed(() => shown.value.find((entry) => entry.id === ev.selected) || null)
const compact = computed(() => shell.narrow)

function choose(entry: Alarm) {
  ev.selected = entry.id
  if (!wide.value) sheetOpen.value = true
}

function label(entry: Alarm) {
  return KIND[kindOf(entry)].label + ' at ' + clockOf(entry, true) + ', ' + ago(entry)
}

watch(
  () => [shown.value, ev.selected] as const,
  () => {
    if (!shown.value.length) return
    if (!shown.value.some((entry) => entry.id === ev.selected)) ev.selected = shown.value[0].id
  },
  { immediate: true },
)

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
  <div class="ev-page-main">
    <div class="ev-page-top">
      <div class="ev-page-title">
        <h1 id="alarm-title">Events</h1>
        <span class="ev-page-status" data-testid="alarm-status" role="status" :title="ev.timing">{{ summary }}</span>
        <span class="spacer" />
        <span v-if="ev.checked" class="ev-updated" data-testid="alarm-updated">Updated <span class="ev-mono">{{ ev.checked }}</span></span>
        <Button size="sm" variant="ghost" :icon="ev.manage.on ? 'check' : 'trash'" data-testid="alarm-manage" :aria-pressed="ev.manage.on ? 'true' : 'false'" :disabled="!ev.query.serial" @click="setManage(!ev.manage.on)">{{ ev.manage.on ? 'Done' : 'Select' }}</Button>
      </div>
      <div class="ev-page-tools">
        <KindChips />
        <CameraPick />
        <DayPicker />
      </div>
      <EventsManage v-if="ev.manage.on" />
    </div>
    <div ref="list" class="ev-page-scroll" :aria-busy="ev.phase === 'loading'">
      <EventsState :skeletons="compact ? 5 : 6" />
      <section v-for="group in groups" :key="group.key" class="ev-section" :aria-labelledby="'ev-g-' + group.key.replace(/[^a-z0-9]/gi, '')">
        <h2 :id="'ev-g-' + group.key.replace(/[^a-z0-9]/gi, '')" class="ev-group">{{ group.label }} <span class="ev-group-n">{{ plural(group.entries.length) }}</span></h2>
        <ul :class="compact ? 'ev-rows ev-rows-page' : 'ev-cards'" role="list">
          <li v-for="entry in group.entries" :key="entry.id">
            <div :class="['alarm-row', compact ? 'ev-row ev-row-page' : 'ev-card', { focus: ev.focus.id && ev.focus.id === entry.id }]" data-testid="alarm-row" :data-id="entry.id" :data-selected="entry.id === ev.selected && !compact ? 'true' : undefined">
              <label v-if="ev.manage.on" class="alarm-pick">
                <input type="checkbox" data-testid="alarm-select" :checked="ev.manage.picked.has(entry.id)" :aria-label="'Select ' + label(entry)" @change="togglePick(entry.id)">
              </label>
              <button type="button" class="ev-hit" :aria-label="label(entry)" :aria-current="entry.id === ev.selected ? 'true' : undefined" @click="choose(entry)">
                <EventThumb :entry="entry" :large="!compact" />
                <span class="ev-row-text">
                  <span class="ev-kind">
                    <Icon :name="KIND[kindOf(entry)].icon" small :class="'ev-ink-' + kindOf(entry)" />
                    <span class="ev-kind-word">{{ KIND[kindOf(entry)].label }}</span>
                    <span v-if="compact && isNew(entry)" class="ev-new">New</span>
                    <time v-if="!compact" class="ev-card-time" :datetime="entry.time">{{ clockOf(entry, true) }}</time>
                  </span>
                  <span v-if="compact" class="ev-when"><time :datetime="entry.time">{{ clockOf(entry, true) }}</time> · {{ ago(entry) }}</span>
                  <span v-if="compact" class="ev-sub">{{ nameOf(ev.asked.serial) }}{{ channelOf(entry) ? ' · ' + channelOf(entry) : '' }}</span>
                </span>
              </button>
            </div>
          </li>
        </ul>
      </section>
      <EventsNotify v-if="!wide" class="ev-notify-inline" />
    </div>
  </div>
  <aside v-if="wide" class="ev-page-side" aria-label="Event details">
    <EventPreview :entry="current" />
    <EventsNotify />
  </aside>
  <Sheet v-else v-model:open="sheetOpen">
    <SheetContent :side="compact ? 'bottom' : 'right'" class="ev-sheet" data-testid="alarm-preview-sheet">
      <SheetTitle class="sr">Event</SheetTitle>
      <SheetDescription class="sr">The picture and details of the chosen event.</SheetDescription>
      <EventPreview :entry="current" />
    </SheetContent>
  </Sheet>
</template>
