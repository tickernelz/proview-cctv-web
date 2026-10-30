<script setup lang="ts">
import Icon from '../../components/Icon.vue'
import { Kbd } from '../../components/ui/kbd'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu'
import { toggleWhiteLight } from '../../app/controls'
import { swapLenses } from '../../app/lens'
import { openSettings, setPref } from '../../app/shell'
import { openDevices } from '../../settings/ui'
import { shell } from '../../shell/layout'
import { live } from '../store'
import { openRecordings } from '../timeline/engine'
import { imageLive, toggleLowLux } from './image-live'
import { QUALITIES } from './notes'
import { pip } from './pip'

let handedOff = false

function flip() {
  setPref('flip', !live.settings.prefs.flip)
}

function recordings(tab: 'camera' | 'cloud') {
  handedOff = true
  openRecordings(tab, 'more')
}

function cameras() {
  handedOff = true
  openDevices('more')
}

function keepSheetFocus(event: Event) {
  if (handedOff) event.preventDefault()
  handedOff = false
}
</script>

<template>
  <DropdownMenu :modal="false">
    <DropdownMenuTrigger id="more" data-testid="ctl-more" class="dk">
      <Icon name="more" /><span class="dk-label">More</span>
    </DropdownMenuTrigger>
    <DropdownMenuContent id="more-menu" data-testid="more-menu" aria-label="More" @close-auto-focus="keepSheetFocus">
      <DropdownMenuLabel class="menu-h">This camera</DropdownMenuLabel>
      <template v-if="shell.narrow">
        <DropdownMenuItem v-if="live.mode !== 'playback'" icon="record" shortcut="R" data-testid="more-record" :disabled="live.record.disabled" @select="live.actions.toggleRecord()">
          {{ live.record.on ? 'Stop recording' : 'Record' }}<span v-if="live.record.on" class="item-state mono">{{ live.record.elapsed }}</span>
        </DropdownMenuItem>
        <DropdownMenuItem icon="expand" shortcut="F" data-testid="more-fullscreen" @select="live.actions.toggleFullscreen()">Full screen</DropdownMenuItem>
        <template v-if="live.mode !== 'playback'">
          <DropdownMenuSeparator class="menu-sep" />
          <DropdownMenuLabel class="menu-h">Quality · Q switches</DropdownMenuLabel>
          <DropdownMenuRadioGroup data-testid="more-quality" :model-value="live.quality" @update:model-value="live.actions.setQuality(String($event))">
            <DropdownMenuRadioItem v-for="option in QUALITIES" :key="option.value" data-testid="more-quality-option" v-bind="option" />
          </DropdownMenuRadioGroup>
        </template>
        <template v-if="live.lens.shown">
          <DropdownMenuSeparator class="menu-sep" />
          <DropdownMenuLabel class="menu-h">Lens · L cycles</DropdownMenuLabel>
          <DropdownMenuRadioGroup data-testid="more-lens" :model-value="live.lens.value" @update:model-value="live.actions.chooseLens(String($event))">
            <DropdownMenuRadioItem v-for="option in live.lens.options" :key="option.value" data-testid="more-lens-option" v-bind="option" />
          </DropdownMenuRadioGroup>
        </template>
        <DropdownMenuSeparator class="menu-sep" />
      </template>
      <DropdownMenuSub v-if="live.lens.shown && !shell.narrow">
        <DropdownMenuSubTrigger icon="layers" data-testid="more-lens">Lens<span class="item-state">{{ live.lens.label }}</span></DropdownMenuSubTrigger>
        <DropdownMenuSubContent data-testid="more-lens-menu" aria-label="Lens">
          <DropdownMenuRadioGroup :model-value="live.lens.value" @update:model-value="live.actions.chooseLens(String($event))">
            <DropdownMenuRadioItem v-for="option in live.lens.options" :key="option.value" data-testid="more-lens-option" v-bind="option" />
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      <DropdownMenuItem v-if="pip.shown" icon="swap" data-testid="more-swap" @select="swapLenses()">Swap lenses</DropdownMenuItem>
      <DropdownMenuItem icon="flip" data-testid="more-flip" @select="flip">Flip picture<span class="item-state">{{ live.settings.prefs.flip ? 'On' : 'Off' }}</span></DropdownMenuItem>
      <DropdownMenuItem v-if="live.settings.light.ok" icon="lightbulb" data-testid="more-light" @select="toggleWhiteLight()">White light</DropdownMenuItem>
      <DropdownMenuItem v-if="imageLive.view?.low_lux.supported" id="low-lux" icon="low-light" data-testid="ctl-low-lux" :disabled="imageLive.busy" @select="toggleLowLux()">Low light<span class="item-state">{{ imageLive.view.low_lux.value ? 'On' : 'Off' }}</span></DropdownMenuItem>
      <DropdownMenuSeparator class="menu-sep" />
      <DropdownMenuItem icon="history" data-testid="more-recordings" @select="recordings('camera')">Recordings</DropdownMenuItem>
      <DropdownMenuItem icon="cloud" data-testid="more-cloud" @select="recordings('cloud')">Cloud recordings</DropdownMenuItem>
      <DropdownMenuSeparator class="menu-sep" />
      <DropdownMenuItem icon="info" data-testid="more-camera-info" @select="openSettings('camera')">Camera info</DropdownMenuItem>
      <DropdownMenuItem icon="gear" data-testid="more-camera-settings" @select="openSettings('device')">Camera settings</DropdownMenuItem>
      <DropdownMenuItem icon="cctv" data-testid="more-manage-cameras" @select="cameras">Manage cameras</DropdownMenuItem>
      <DropdownMenuSeparator class="menu-sep" />
      <p class="menu-note more-keys">Keyboard: <Kbd>F</Kbd> full screen · <Kbd>G</Kbd> grid · <Kbd>?</Kbd> every key</p>
    </DropdownMenuContent>
  </DropdownMenu>
</template>
