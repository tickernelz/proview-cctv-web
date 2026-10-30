<script setup lang="ts">
import Icon from '../../Icon.vue'
import Button from '../button/Button.vue'
import IconButton from '../icon-button/IconButton.vue'
import { dismiss, pauseToasts, resumeToasts, toasts, type ToastItem, type ToastTone } from './toast'

const ICONS: Record<ToastTone, { name: string; tone: string } | undefined> = {
  neutral: undefined,
  ok: { name: 'check', tone: 'text-ok' },
  warn: { name: 'warning', tone: 'text-warn' },
  bad: { name: 'alert', tone: 'text-bad' },
}

function runAction(item: ToastItem): void {
  item.action?.onClick()
  dismiss(item.id)
}

function onFocusOut(event: FocusEvent): void {
  const region = event.currentTarget as HTMLElement
  if (!region.contains(event.relatedTarget as Node | null)) resumeToasts()
}
</script>

<template>
  <div
    role="region"
    aria-label="Notifications"
    class="pointer-events-none fixed right-4 bottom-4 z-50 flex w-[360px] max-w-[calc(100vw-32px)] flex-col gap-2 max-sm:inset-x-2 max-sm:bottom-[calc(var(--tabbar)+8px)] max-sm:w-auto max-sm:max-w-none"
    @pointerenter="pauseToasts"
    @pointerleave="resumeToasts"
    @focusin="pauseToasts"
    @focusout="onFocusOut"
  >
    <div
      v-for="item in toasts"
      :key="item.id"
      :role="item.tone === 'bad' ? 'alert' : 'status'"
      aria-atomic="true"
      :data-tone="item.tone"
      class="pointer-events-auto flex items-start gap-3 rounded-lg border border-line bg-bg-2 py-3 pr-2 pl-3 text-ink shadow-panel transition-[opacity,translate] duration-(--dur-2) ease-standard starting:translate-y-2 starting:opacity-0"
    >
      <Icon v-if="ICONS[item.tone]" :name="ICONS[item.tone]!.name" small :class="['mt-0.5', ICONS[item.tone]!.tone]" />
      <div class="flex min-w-0 flex-1 flex-col gap-0.5 pt-px">
        <p class="text-14 font-medium text-pretty">{{ item.title }}</p>
        <p v-if="item.description" class="text-13 text-pretty text-ink-2">{{ item.description }}</p>
        <div v-if="item.action" class="mt-2">
          <Button size="sm" @click="runAction(item)">{{ item.action.label }}</Button>
        </div>
      </div>
      <IconButton label="Dismiss notification" icon="x" size="sm" class="-my-1" @click="dismiss(item.id)" />
    </div>
  </div>
</template>
