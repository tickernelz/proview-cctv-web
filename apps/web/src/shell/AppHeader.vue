<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { DropdownMenuItem } from 'reka-ui'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { IconButton } from '../components/ui/icon-button'
import { Kbd } from '../components/ui/kbd'
import { SegmentedControl } from '../components/ui/segmented-control'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu'
import { DropdownMenuRadioItem } from 'reka-ui'
import AlarmCenter from '../live/AlarmCenter.vue'
import { live } from '../live/store'
import { openSettings } from '../app/shell'
import { state } from '../app/state'
import { toggleTheme } from '../app/theme'
import { loadDevices, openCamera, setView } from '../app/view'
import { shell } from './layout'
import { go, liveHash } from './router'

const KEYS: [string[], string][] = [
  [['1', '9'], 'Switch to camera n'],
  [['G'], 'Single camera or grid'],
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

const LAYOUTS = [
  { value: 'single', label: 'Single', icon: 'frame', testid: 'view-single' },
  { value: 'grid', label: 'Grid', icon: 'grid', testid: 'view-grid' },
]

const top = live.top
const keys = ref<HTMLElement>()
const keysClose = ref<HTMLElement>()
const count = computed(() => Math.min(9, top.cameras.length))
const layout = computed(() => (top.inGrid ? 'grid' : 'single'))
const themeLabel = computed(() => (live.theme === 'dark' ? 'Switch to the light theme' : 'Switch to the dark theme'))
let previous: HTMLElement | null = null

function pickLayout(value: string) {
  if (shell.screen === 'watch' && shell.view === 'live') {
    if (value !== layout.value) setView(value === 'grid' ? 'grid' : 'live')
    return
  }
  go(liveHash(value === 'grid' ? 'grid' : 'single'))
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
      keysClose.value?.querySelector('button')?.focus()
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
  <header class="app-header" data-testid="app-header">
    <span class="brand"><span class="mark" aria-hidden="true" /><span class="brand-name">PROVIEW</span></span>
    <DropdownMenu v-model:open="top.menuOpen" :modal="false">
      <DropdownMenuTrigger id="switcher" :class="['switcher', { static: !top.several }]" data-testid="camera-switcher" :disabled="!top.several">
        <span v-if="top.dotShown" id="switcher-dot" :class="['dot', { off: top.dotOff }]" role="img" :aria-label="top.dotLabel" />
        <span id="switcher-name" class="name" data-testid="camera-name">{{ top.name }}</span>
        <span id="switcher-meta" class="meta">{{ top.meta }}</span>
        <Icon name="chevron-down" small class="chev" />
      </DropdownMenuTrigger>
      <DropdownMenuContent id="camera-menu" data-testid="camera-menu" aria-label="Cameras" side="bottom" align="start" :side-offset="6" loop @keydown="onMenuKey" @open-auto-focus="focusChecked">
        <DropdownMenuLabel class="menu-h">Cameras · 1-{{ count }} on the keyboard</DropdownMenuLabel>
        <DropdownMenuRadioGroup :model-value="top.selected">
          <DropdownMenuRadioItem v-for="camera in top.cameras" :key="camera.key" :value="camera.key" class="menu-row" data-testid="camera-option" @select="openCamera(camera.index)">
            <span :class="['dot', { off: !camera.online }]" />
            <span class="col"><span class="name">{{ camera.name }}</span><span class="sub">{{ camera.sub }}</span></span>
            <span class="meta">{{ camera.index < 9 ? camera.index + 1 : '' }}</span>
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator class="menu-sep" />
        <DropdownMenuItem class="menu-row" @select="loadDevices()"><Icon name="refresh" small /><span class="name">Refresh the list</span></DropdownMenuItem>
        <DropdownMenuItem class="menu-row" @select="openSettings('config', { focus: true })"><Icon name="plus" small /><span class="name">Add a camera</span><span class="meta">Settings</span></DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <div class="spacer" />
    <SegmentedControl
      class="hdr-layout"
      :model-value="shell.screen === 'watch' && shell.view === 'live' ? layout : ''"
      :options="LAYOUTS"
      label="Live layout"
      size="sm"
      :icon-only="shell.narrow"
      :disabled="top.gridDisabled"
      @update:model-value="pickLayout"
    />
    <div class="hdr-actions">
      <AlarmCenter />
      <Button id="open-settings" class="max-md:w-(--hit) max-md:px-0 max-sm:hidden" data-testid="open-settings" variant="ghost" icon="gear" title="Settings (,)" :aria-pressed="shell.view === 'settings' ? 'true' : undefined" @click="openSettings(state.section)">
        <span class="hdr-label">Settings</span>
      </Button>
      <IconButton v-if="shell.view !== 'settings'" data-testid="theme-toggle" :label="themeLabel" :icon="live.theme === 'dark' ? 'sun' : 'moon'" side="bottom" @click="toggleTheme" />
    </div>
  </header>
  <section id="keys" ref="keys" class="menu sheet keys-sheet" data-testid="keys-sheet" aria-labelledby="keys-title" :hidden="!top.keysOpen" @keydown.tab="tabOut">
    <div class="sheet-h">
      <h2 id="keys-title">Keyboard</h2>
      <span ref="keysClose"><IconButton id="keys-close" label="Close the keyboard list" icon="x" size="sm" side="left" @click="closeKeys" /></span>
    </div>
    <dl class="keys">
      <div v-for="[combo, meaning] in KEYS" :key="meaning">
        <dt>
          <template v-if="combo[0] === '1'"><Kbd>1</Kbd>–<Kbd>9</Kbd></template>
          <template v-else-if="combo[1] === 'held'"><Kbd>{{ combo[0] }}</Kbd> held</template>
          <template v-else-if="combo[0] === 'Arrows'">Arrows</template>
          <Kbd v-else>{{ combo[0] }}</Kbd>
        </dt>
        <dd>{{ meaning }}</dd>
      </div>
    </dl>
  </section>
</template>
