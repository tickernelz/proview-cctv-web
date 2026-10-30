<script setup lang="ts">
import { computed, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
import EventsPage from '../events/EventsPage.vue'
import EventsPanel from '../events/EventsPanel.vue'
import { close, show, startEvents, unread } from '../events/events'
import { eventsOpen, shell } from '../shell/layout'
import { live } from './store'

const signedIn = computed(() => live.settings.signedIn)
const onPage = computed(() => shell.view === 'events')
const overlay = computed(() => !onPage.value && !shell.wide)
const badge = computed(() => (unread.value > 99 ? '99+' : String(unread.value)))
const bellLabel = computed(() => (unread.value ? 'Events, ' + unread.value + ' new' : 'Events'))

function toggle() {
  if (eventsOpen.value) close()
  else show()
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return
  event.preventDefault()
  close()
}

onMounted(startEvents)
</script>

<template>
  <button
    id="alarm-bell"
    class="iconbtn alarm-bell"
    type="button"
    data-testid="alarm-bell"
    :hidden="!signedIn"
    :aria-label="bellLabel"
    aria-controls="alarm-sheet"
    :aria-expanded="eventsOpen"
    title="Events"
    @click="toggle"
  >
    <Icon name="bell" />
    <span class="hdr-label">Events</span>
    <span class="badge" data-testid="alarm-badge" aria-hidden="true" :hidden="!unread">{{ badge }}</span>
  </button>
  <Teleport defer to="#events-host">
    <section
      id="alarm-sheet"
      :class="['alarm-sheet', onPage ? 'ev-page' : 'ev-panel']"
      data-testid="alarm-center"
      :data-mode="onPage ? 'page' : overlay ? 'overlay' : 'docked'"
      :role="overlay ? 'dialog' : 'region'"
      aria-labelledby="alarm-title"
      :hidden="!eventsOpen"
      @keydown="onKey"
    >
      <EventsPage v-if="onPage" />
      <EventsPanel v-else @close="close()" />
    </section>
  </Teleport>
</template>
