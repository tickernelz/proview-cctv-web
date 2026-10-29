<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { DropdownMenuItem, DropdownMenuRadioItem } from 'reka-ui'
import Icon from '../components/Icon.vue'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu'
import { openSettings } from '../app/shell'
import { state } from '../app/state'
import { toggleTheme } from '../app/theme'
import { loadDevices, openCamera, requestStream, setView } from '../legacy/app.js'
import { live } from './store'

const KEYS: [string[], string][] = [
  [['1', '9'], 'Switch to camera n'],
  [['G'], 'Live or every camera'],
  [['F'], 'Full screen'],
  [['M'], 'Sound on or off'],
  [['T', 'held'], 'Talk through the camera'],
  [['R'], 'Start or stop recording'],
  [['S'], 'Snapshot'],
  [['L'], 'Both, top, bottom lens'],
  [['Q'], 'HD or SD'],
  [['V'], 'Open or close the move pad'],
  [['Arrows'], 'Move the lens (pad open) or the tile focus (grid)'],
  [['Space'], 'Stop moving'],
  [[','], 'Settings'],
  [['Esc'], 'Close menu, then pad, then full screen'],
  [['?'], 'This list'],
]

const top = live.top
const keys = ref<HTMLElement>()
const keysClose = ref<HTMLButtonElement>()
const count = computed(() => Math.min(9, top.cameras.length))
const gridLabel = computed(() => (top.inGrid ? 'Back to the camera' : 'Show every camera'))
const bannerIcon = computed(() => (live.banner.tone === 'list' ? 'warning' : 'disconnect'))
let previous: HTMLElement | null = null

function pick(index: number) {
  openCamera(index)
}

function onMenuKey(event: KeyboardEvent) {
  if (!/^[1-9]$/.test(event.key) || event.ctrlKey || event.metaKey || event.altKey) return
  const index = Number(event.key) - 1
  if (index >= top.cameras.length) return
  event.preventDefault()
  event.stopPropagation()
  top.menuOpen = false
  openCamera(index)
}

function focusChecked(event: Event) {
  const menu = document.getElementById('camera-menu')
  const item = menu?.querySelector<HTMLElement>('[role="menuitemradio"][aria-checked="true"]') || menu?.querySelector<HTMLElement>('[role^="menuitem"]')
  if (!item) return
  event.preventDefault()
  item.focus()
}

function addCamera() {
  openSettings('config', { focus: true })
}

function act() {
  if (live.banner.action === 'retry') requestStream()
  else loadDevices()
}

function closeKeys() {
  top.restoreFocus = true
  top.keysOpen = false
}

function tabOut() {
  if (previous && previous.isConnected) previous.focus()
  top.restoreFocus = false
  top.keysOpen = false
}

function outside(event: PointerEvent) {
  if (keys.value && !keys.value.contains(event.target as Node)) {
    top.restoreFocus = false
    top.keysOpen = false
  }
}

watch(
  () => top.keysOpen,
  async (open) => {
    if (open) {
      previous = document.activeElement as HTMLElement | null
      await nextTick()
      keysClose.value?.focus()
      document.addEventListener('pointerdown', outside, true)
      return
    }
    document.removeEventListener('pointerdown', outside, true)
    if (top.restoreFocus && previous && previous.isConnected) previous.focus()
    top.restoreFocus = false
    previous = null
  },
)

onBeforeUnmount(() => document.removeEventListener('pointerdown', outside, true))
</script>

<template>
  <header class="topbar">
    <DropdownMenu v-model:open="top.menuOpen" :modal="false">
      <DropdownMenuTrigger id="switcher" :class="['switcher', { static: !top.several }]" data-testid="camera-switcher" :disabled="!top.several">
        <span id="switcher-dot" :class="['dot', { off: top.dotOff }]" role="img" :aria-label="top.dotLabel" />
        <span id="switcher-name" class="name" data-testid="camera-name">{{ top.name }}</span>
        <span id="switcher-meta" class="meta">{{ top.meta }}</span>
        <Icon name="chevron-down" small class="chev" />
      </DropdownMenuTrigger>
      <DropdownMenuContent id="camera-menu" data-testid="camera-menu" aria-label="Cameras" side="bottom" align="start" :side-offset="6" loop @keydown="onMenuKey" @open-auto-focus="focusChecked">
        <DropdownMenuLabel class="menu-h">Cameras · 1-{{ count }} on the keyboard</DropdownMenuLabel>
        <DropdownMenuRadioGroup :model-value="top.selected">
          <DropdownMenuRadioItem v-for="camera in top.cameras" :key="camera.key" :value="camera.key" class="menu-row" data-testid="camera-option" @select="pick(camera.index)">
            <span :class="['dot', { off: !camera.online }]" />
            <span class="col"><span class="name">{{ camera.name }}</span><span class="sub">{{ camera.sub }}</span></span>
            <span class="meta">{{ camera.index < 9 ? camera.index + 1 : '' }}</span>
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator class="menu-sep" />
        <DropdownMenuItem class="menu-row" @select="loadDevices()"><Icon name="refresh" small /><span class="name">Refresh the list</span></DropdownMenuItem>
        <DropdownMenuItem class="menu-row" @select="addCamera"><Icon name="sliders" small /><span class="name">Add a camera</span><span class="meta">Settings</span></DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <div class="spacer" />
    <button id="grid-toggle" class="iconbtn" data-testid="grid-toggle" type="button" :disabled="top.gridDisabled" :aria-label="gridLabel" :title="gridLabel + ' (G)'" @click="setView(top.inGrid ? 'live' : 'grid')">
      <Icon :name="top.inGrid ? 'frame' : 'grid'" />
    </button>
    <button id="open-settings" class="iconbtn" data-testid="open-settings" type="button" aria-label="Settings" title="Settings (,)" @click="openSettings(state.section)">
      <Icon name="sliders" />
    </button>
    <button class="iconbtn" type="button" data-testid="theme-toggle" :aria-label="live.theme === 'dark' ? 'Switch to the light theme' : 'Switch to the dark theme'" title="Theme" @click="toggleTheme">
      <Icon :name="live.theme === 'dark' ? 'sun' : 'moon'" />
    </button>
  </header>
  <div v-if="live.banner.title" id="banner" class="banner" data-testid="live-banner" :data-tone="live.banner.tone" role="alert">
    <Icon :name="bannerIcon" small />
    <span><strong id="banner-title">{{ live.banner.title }}</strong> <span id="banner-text">{{ live.banner.text }}</span></span>
    <div class="spacer" />
    <button id="banner-action" class="btn btn-line" type="button" data-testid="banner-action" @click="act">
      <Icon name="refresh" small />{{ live.banner.action === 'retry' ? 'Retry now' : 'Refresh the list' }}
    </button>
  </div>
  <section id="keys" ref="keys" class="menu sheet keys-sheet" data-testid="keys-sheet" aria-labelledby="keys-title" :hidden="!top.keysOpen" @keydown.tab="tabOut">
    <div class="sheet-h">
      <h2 id="keys-title">Keyboard</h2>
      <button id="keys-close" ref="keysClose" class="iconbtn" type="button" aria-label="Close the keyboard list" @click="closeKeys"><Icon name="x" small /></button>
    </div>
    <dl class="keys">
      <div v-for="[combo, meaning] in KEYS" :key="meaning">
        <dt>
          <template v-if="combo[0] === '1'"><kbd>1</kbd>–<kbd>9</kbd></template>
          <template v-else-if="combo[1] === 'held'"><kbd>{{ combo[0] }}</kbd> held</template>
          <template v-else-if="combo[0] === 'Arrows'">Arrows</template>
          <kbd v-else>{{ combo[0] }}</kbd>
        </dt>
        <dd>{{ meaning }}</dd>
      </div>
    </dl>
  </section>
</template>
