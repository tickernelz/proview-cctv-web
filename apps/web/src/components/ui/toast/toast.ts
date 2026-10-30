import { reactive } from 'vue'

export type ToastTone = 'neutral' | 'ok' | 'warn' | 'bad'

export interface ToastAction {
  label: string
  onClick: () => void
}

export interface ToastOptions {
  title: string
  description?: string
  tone?: ToastTone
  action?: ToastAction
  duration?: number
}

export interface ToastItem {
  id: number
  title: string
  description?: string
  tone: ToastTone
  action?: ToastAction
  duration: number
}

interface Timer {
  left: number
  started: number
  handle?: ReturnType<typeof setTimeout>
}

export const toasts = reactive<ToastItem[]>([])
const timers = new Map<number, Timer>()
let nextId = 0
let paused = false

function arm(id: number, timer: Timer): void {
  timer.started = Date.now()
  timer.handle = setTimeout(() => dismiss(id), timer.left)
}

export function toast(options: ToastOptions): number {
  const id = ++nextId
  const duration = options.duration ?? 5000
  toasts.push({ id, title: options.title, description: options.description, tone: options.tone ?? 'neutral', action: options.action, duration })
  if (duration > 0) {
    const timer: Timer = { left: duration, started: Date.now() }
    timers.set(id, timer)
    if (!paused) arm(id, timer)
  }
  return id
}

export function dismiss(id: number): void {
  const timer = timers.get(id)
  if (timer) clearTimeout(timer.handle)
  timers.delete(id)
  const index = toasts.findIndex((item) => item.id === id)
  if (index >= 0) toasts.splice(index, 1)
}

export function pauseToasts(): void {
  if (paused) return
  paused = true
  const now = Date.now()
  for (const timer of timers.values()) {
    clearTimeout(timer.handle)
    timer.left = Math.max(0, timer.left - (now - timer.started))
  }
}

export function resumeToasts(): void {
  if (!paused) return
  paused = false
  for (const [id, timer] of timers) arm(id, timer)
}
