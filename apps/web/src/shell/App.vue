<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { IconButton } from '../components/ui/icon-button'
import { Toaster } from '../components/ui/toast'
import { TooltipProvider } from '../components/ui/tooltip'
import DevicesSheet from '../live/DevicesSheet.vue'
import LiveMain from '../live/LiveMain.vue'
import PlaybackScreen from '../live/PlaybackScreen.vue'
import RecordingsPanel from '../live/RecordingsPanel.vue'
import SettingsScreen from '../live/SettingsScreen.vue'
import SignInScreen from '../live/SignInScreen.vue'
import { live } from '../live/store'
import { skipToControls } from '../app/keys'
import { requestStream } from '../app/stage'
import { toggleTheme } from '../app/theme'
import { loadDevices } from '../app/view'
import AppHeader from './AppHeader.vue'
import TabBar from './TabBar.vue'
import { eventsOpen, panelOpen, shell } from './layout'

const signedFrame = computed(() => shell.screen === 'watch' || (shell.screen === 'settings' && live.settings.signedIn))
const panel = computed(() => (shell.view === 'events' ? 'page' : panelOpen.value ? (shell.wide ? 'docked' : 'overlay') : 'closed'))
const themeLabel = computed(() => (live.theme === 'dark' ? 'Switch to the light theme' : 'Switch to the dark theme'))
const bannerIcon = computed(() => (live.banner.tone === 'list' ? 'warning' : 'disconnect'))

function act() {
  if (live.banner.action === 'retry') requestStream()
  else loadDevices()
}
</script>

<template>
  <TooltipProvider>
    <div class="app" :data-screen="shell.screen" :data-view="shell.view" :data-panel="panel">
      <a v-show="shell.screen === 'watch' && shell.view === 'live'" class="skip" href="#controls" @click.prevent="skipToControls">Skip to controls</a>
      <div id="boot" class="screen" aria-busy="true">
        <header class="app-header"><div class="sk" style="width: 220px; height: 20px" /><div class="spacer" /><div class="sk" style="width: 116px; height: 28px" /></header>
        <div class="viewer">
          <div class="well" />
          <div class="well-meta"><div class="sk" style="width: 260px; height: 14px" /></div>
          <div class="controls"><div class="sk" style="width: 300px; height: 28px" /><span class="spacer" /><div class="sk" style="width: 200px; height: 28px" /></div>
        </div>
        <p class="sr" role="status">Opening PROVIEW</p>
      </div>
      <div id="signin" data-testid="signin" class="screen" hidden>
        <header class="app-header">
          <span class="brand"><span class="mark" aria-hidden="true" /><span class="brand-name">PROVIEW</span></span>
          <div class="spacer" />
          <IconButton data-testid="theme-toggle" :label="themeLabel" :icon="live.theme === 'dark' ? 'sun' : 'moon'" side="bottom" @click="toggleTheme" />
        </header>
        <main id="signin-main" class="auth"><SignInScreen /></main>
      </div>
      <div v-show="signedFrame" class="contents"><AppHeader /></div>
      <div v-show="shell.screen === 'watch' || shell.screen === 'settings'" class="app-body">
        <div id="watch" data-testid="live" class="screen" hidden>
          <div v-if="live.banner.title" id="banner" class="banner" data-testid="live-banner" :data-tone="live.banner.tone" role="alert">
            <Icon :name="bannerIcon" small />
            <span><strong id="banner-title">{{ live.banner.title }}</strong> <span id="banner-text">{{ live.banner.text }}</span></span>
            <div class="spacer" />
            <Button id="banner-action" data-testid="banner-action" size="sm" icon="refresh" @click="act">{{ live.banner.action === 'retry' ? 'Retry now' : 'Refresh the list' }}</Button>
          </div>
          <main id="main" class="viewer"><LiveMain /></main>
        </div>
        <div id="settings" data-testid="settings" class="screen" hidden><SettingsScreen /></div>
        <aside v-show="eventsOpen" id="events-panel" class="events-panel" data-testid="events-panel" aria-label="Events">
          <div id="events-host" class="events-host" />
        </aside>
      </div>
      <TabBar v-if="shell.narrow" v-show="signedFrame" />
      <PlaybackScreen />
      <DevicesSheet />
      <RecordingsPanel />
      <Toaster />
    </div>
  </TooltipProvider>
</template>
