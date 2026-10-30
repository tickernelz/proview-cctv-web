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
import { live } from '../store'
import { imageLive, toggleLowLux } from './image-live'
import { pip } from './pip'

function flip() {
  setPref('flip', !live.settings.prefs.flip)
}
</script>

<template>
  <DropdownMenu :modal="false">
    <DropdownMenuTrigger id="more" data-testid="ctl-more" class="dk">
      <Icon name="more" /><span class="dk-label">More</span>
    </DropdownMenuTrigger>
    <DropdownMenuContent id="more-menu" data-testid="more-menu" aria-label="More">
      <DropdownMenuLabel class="menu-h">This camera</DropdownMenuLabel>
      <DropdownMenuSub v-if="live.lens.shown">
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
      <DropdownMenuItem icon="info" data-testid="more-camera-info" @select="openSettings('camera')">Camera info</DropdownMenuItem>
      <DropdownMenuItem icon="gear" data-testid="more-camera-settings" @select="openSettings('device')">Camera settings</DropdownMenuItem>
      <DropdownMenuSeparator class="menu-sep" />
      <p class="menu-note more-keys">Keyboard: <Kbd>F</Kbd> full screen · <Kbd>G</Kbd> grid · <Kbd>?</Kbd> every key</p>
    </DropdownMenuContent>
  </DropdownMenu>
</template>
