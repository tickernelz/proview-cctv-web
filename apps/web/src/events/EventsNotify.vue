<script setup lang="ts">
import { desktop, ev, setNotify } from './events'
</script>

<template>
  <section class="ev-notify" aria-labelledby="ev-notify-title">
    <h2 id="ev-notify-title" class="ev-notify-title">Notifications</h2>
    <button v-if="!desktop" type="button" role="switch" class="ev-notify-row" data-testid="alarm-notify" :aria-checked="ev.notify.on ? 'true' : 'false'" aria-describedby="alarm-notify-note" @click="setNotify(!ev.notify.on)">
      <span class="ev-notify-label">Notify me</span>
      <span class="ev-toggle" aria-hidden="true"><span class="ev-toggle-thumb" /></span>
    </button>
    <p v-if="!desktop" id="alarm-notify-note" class="ev-note" data-testid="alarm-notify-note" role="status">{{ ev.notify.note || (ev.notify.on ? 'On. A notification shows when an event arrives while this tab is in the background.' : 'Off. Turn it on to get a browser notification for new events.') }}</p>
    <p v-else class="ev-note">The desktop app notifies you; set it under Settings.</p>
    <p class="ev-note" data-testid="alarm-push">{{ ev.push.text || 'Phone push has not been checked yet.' }}</p>
  </section>
</template>
