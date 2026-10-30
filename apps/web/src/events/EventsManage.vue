<script setup lang="ts">
import { computed } from 'vue'
import { Button } from '../components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog'
import { clearReason, deleteReason, ev, nameOf, plural, remove } from './events'

const confirmOpen = computed({
  get: () => Boolean(ev.manage.confirm),
  set: (open: boolean) => {
    if (!open && !ev.manage.busy) ev.manage.confirm = ''
  },
})
const title = computed(() => (ev.manage.confirm === 'all' ? 'Delete every event of ' + nameOf(ev.asked.serial) + '?' : 'Delete ' + plural(ev.manage.picked.size) + '?'))
</script>

<template>
  <div class="ev-manage" data-testid="alarm-manage-panel">
    <span class="ev-manage-count" role="status">{{ ev.manage.picked.size ? ev.manage.picked.size + ' selected' : 'None selected' }}</span>
    <Button size="sm" variant="danger" icon="trash" data-testid="alarm-delete" :disabled="Boolean(deleteReason)" :loading="ev.manage.busy && ev.manage.confirm === 'picked'" @click="ev.manage.confirm = 'picked'">Delete selected</Button>
    <Button size="sm" variant="ghost" data-testid="alarm-clear" :disabled="Boolean(clearReason) || ev.manage.busy" @click="ev.manage.confirm = 'all'">Clear all…</Button>
    <p v-if="deleteReason && !ev.manage.busy" class="ev-reason" data-testid="alarm-delete-reason">{{ deleteReason }}</p>
    <p v-if="clearReason && clearReason !== deleteReason" class="ev-reason" data-testid="alarm-clear-reason">{{ clearReason }}</p>
    <p v-if="ev.manage.note" class="err ev-manage-note" data-testid="alarm-manage-note" role="alert">{{ ev.manage.note }}</p>
  </div>
  <Dialog v-model:open="confirmOpen">
    <DialogContent size="sm" role="alertdialog" data-testid="alarm-confirm">
      <DialogHeader>
        <DialogTitle>{{ title }}</DialogTitle>
        <DialogDescription>They are removed from the PROVIEW cloud for every phone and computer on this account. This cannot be undone.</DialogDescription>
      </DialogHeader>
      <p v-if="ev.manage.note" class="err" role="alert">{{ ev.manage.note }}</p>
      <DialogFooter>
        <Button data-testid="alarm-clear-cancel" :disabled="ev.manage.busy" @click="ev.manage.confirm = ''">Cancel</Button>
        <Button variant="danger" icon="trash" data-testid="alarm-clear-confirm" :loading="ev.manage.busy" @click="remove(ev.manage.confirm === 'all')">{{ ev.manage.confirm === 'all' ? 'Delete all' : 'Delete' }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
