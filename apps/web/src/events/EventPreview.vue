<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import EventThumb from './EventThumb.vue'
import { KIND, ago, channelOf, clockOf, dayLabel, dayOfEntry, ev, kindOf, nameOf, playEvent, playFromLabel, playReason, type Alarm } from './events'

const props = defineProps<{ entry: Alarm | null }>()
const kind = computed(() => (props.entry ? kindOf(props.entry) : 'alarm'))
const reason = computed(() => playReason(props.entry))
const start = computed(() => (props.entry ? playFromLabel(props.entry) : ''))
</script>

<template>
  <div v-if="entry" class="ev-preview" data-testid="alarm-preview">
    <div class="ev-preview-picture">
      <EventThumb :entry="entry" large />
      <p v-if="!entry.picture || !entry.picture.object || ev.missing.has(entry.id) || ev.pictures === false" class="ev-preview-none">No picture for this event</p>
    </div>
    <div class="ev-preview-head">
      <h2 class="ev-preview-title" data-testid="alarm-preview-title"><Icon :name="KIND[kind].icon" :class="'ev-ink-' + kind" />{{ KIND[kind].label }}</h2>
      <time class="ev-preview-time" :datetime="entry.time">{{ clockOf(entry, true) }}</time>
    </div>
    <p class="ev-preview-sub">{{ ago(entry) }} · {{ nameOf(ev.asked.serial) }}</p>
    <dl class="ev-facts">
      <div><dt>Camera</dt><dd>{{ nameOf(ev.asked.serial) }}</dd></div>
      <div v-if="channelOf(entry)"><dt>Channel</dt><dd>{{ channelOf(entry).replace('Channel ', '') }}</dd></div>
      <div><dt>Day</dt><dd>{{ dayLabel(dayOfEntry(entry)) }}</dd></div>
      <div><dt>Reported as</dt><dd class="ev-mono">{{ entry.name || entry.event || 'alarm' }}</dd></div>
      <div><dt>Reported by</dt><dd>PROVIEW cloud alarm</dd></div>
    </dl>
    <Button variant="primary" icon="play" block data-testid="alarm-play" :disabled="Boolean(reason)" @click="playEvent(entry)">Play from 5 s before</Button>
    <p v-if="reason" class="ev-reason" data-testid="alarm-play-reason">{{ reason }}</p>
    <p v-else class="ev-hint"><Icon name="info" small /><span>Opens Live on the timeline at <span class="ev-mono">{{ start }}</span>, 5 s before the event.</span></p>
  </div>
  <div v-else class="ev-preview ev-preview-empty" data-testid="alarm-preview">
    <p class="ev-empty-title">No event chosen</p>
    <p class="ev-empty-text">Choose an event to see its picture and play it.</p>
  </div>
</template>
