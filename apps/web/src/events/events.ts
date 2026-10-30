import { computed, reactive, watch } from 'vue'
import { toast } from '../components/ui/toast'
import { state as page } from '../app/state'
import { openCamera } from '../app/view'
import { live } from '../live/store'
import { closeEvents, eventsOpen, openEvents, shell } from '../shell/layout'
import { go, liveHash } from '../shell/router'

export type Picture = { object?: string; bucket?: string }
export type Alarm = { id: string; name?: string; event?: string; eventName?: string; time?: string; utcTime?: number; channel?: string; picture?: Picture }
export type Device = { id: string; name?: string; channels?: number }
export type Kind = 'motion' | 'human' | 'alarm'
export type KindFilter = 'all' | Kind
export type Range = 'today' | 'yesterday' | '24h' | '7d' | 'day'
export type Filter = { id: string; label: string }
export type Problem = { title: string; text: string; detail: string }
export type Group = { key: string; label: string; entries: Alarm[] }
export type Chip = { id: KindFilter; label: string; icon: string; count: number | undefined }
type Listing = { entries?: Alarm[]; total?: number; finished?: boolean; pollSeconds?: number; latencyMs?: number; message?: string; canDelete?: boolean; deleteReason?: string }
type Query = { serial: string; range: Range; day: string; event: string }
type Phase = 'idle' | 'loading' | 'ok' | 'error'
type Day = { day: string; count: number }

const KNOWN: Filter[] = [
  { id: 'VideoMotion', label: 'Motion detection' },
  { id: 'appEventHumanDetectAlarm', label: 'Human detection' },
]
const SEEN_KEY = 'pvw_events_seen'
const NOTIFY_KEY = 'pvw_events_notify'
const LOCALE = 'en-GB'
export const PLAY_LEAD_MS = 5000
export const desktop = Boolean((window as unknown as { __TAURI__?: unknown }).__TAURI__)

export const KIND: Record<Kind, { label: string; icon: string; word: string }> = {
  motion: { label: 'Motion', icon: 'motion', word: 'motion' },
  human: { label: 'Human', icon: 'human', word: 'human' },
  alarm: { label: 'Alarm', icon: 'siren', word: 'alarm' },
}

export const RANGES: { id: Exclude<Range, 'day'>; label: string }[] = [
  { id: 'today', label: 'Today' },
  { id: 'yesterday', label: 'Yesterday' },
  { id: '24h', label: 'Last 24 hours' },
  { id: '7d', label: 'Last 7 days' },
]

function readJsonStore<T>(key: string, fallback: T): T {
  try {
    const value = JSON.parse(localStorage.getItem(key) || 'null')
    return value && typeof value === 'object' ? (value as T) : fallback
  } catch {
    return fallback
  }
}

function writeStore(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    return
  }
}

function storedNotify() {
  try {
    return localStorage.getItem(NOTIFY_KEY) === 'on' && typeof Notification !== 'undefined' && Notification.permission === 'granted'
  } catch {
    return false
  }
}

export const ev = reactive({
  watching: false,
  devices: [] as Device[],
  extra: null as Device | null,
  query: { serial: '', range: 'today', day: '', event: '' } as Query,
  kind: 'all' as KindFilter,
  filters: KNOWN.slice(),
  phase: 'idle' as Phase,
  asked: { serial: '', range: 'today', day: '', event: '' } as Query,
  tally: { key: '', counts: {} as Partial<Record<KindFilter, number>> },
  entries: [] as Alarm[],
  problem: null as Problem | null,
  total: 0,
  finished: true,
  checked: '',
  timing: '',
  pollSeconds: 20,
  pictures: 'unknown' as 'unknown' | boolean,
  missing: new Set<string>(),
  focus: { id: '', shown: '' },
  selected: '',
  now: Date.now(),
  seen: readJsonStore<Record<string, number>>(SEEN_KEY, {}),
  since: {} as Record<string, number>,
  month: { month: '', phase: 'idle' as Phase, error: '', days: [] as Day[] },
  manage: { on: false, picked: new Set<string>(), busy: false, note: '', refused: '', confirm: '' as '' | 'picked' | 'all' },
  push: { phase: 'idle' as Phase, text: '' },
  notify: { on: storedNotify(), note: '' },
})

let notified = new Set<string>()
let primed = ''
let timer = 0
let clockTimer = 0
let lastCheck = 0
let ticket = 0
let inflight: Query | null = null
let started = false
const opened = new Set<string>()

const two = (value: number) => String(value).padStart(2, '0')
export const stamp = (when: Date) => dayOf(when) + ' ' + two(when.getHours()) + ':' + two(when.getMinutes()) + ':' + two(when.getSeconds())
export const dayOf = (when: Date) => when.getFullYear() + '-' + two(when.getMonth() + 1) + '-' + two(when.getDate())
export const monthOf = (when: Date) => when.getFullYear() + '-' + two(when.getMonth() + 1)
const same = (left: Query, right: Query) => left.serial === right.serial && left.range === right.range && left.day === right.day && left.event === right.event
const windowKey = (query: Query) => query.serial + '|' + query.range + '|' + query.day
const capital = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)
export const plural = (count: number, word = 'event') => count + ' ' + word + (count === 1 ? '' : 's')

function parts(text?: string) {
  const match = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})$/.exec(text || '')
  if (!match) return null
  return { day: match[1] + '-' + match[2] + '-' + match[3], hour: Number(match[4]), hm: match[4] + ':' + match[5], hms: match[4] + ':' + match[5] + ':' + match[6] }
}

export function atOf(entry: Alarm) {
  if (entry.utcTime) return entry.utcTime * 1000
  return atOfLocal(entry)
}

function atOfLocal(entry: Alarm) {
  const match = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})$/.exec(entry.time || '')
  if (!match) return entry.utcTime ? entry.utcTime * 1000 : 0
  const [, year, month, date, hour, minute, second] = match.map(Number)
  return new Date(year, month - 1, date, hour, minute, second).getTime()
}

export function clockOf(entry: Alarm, seconds = false) {
  const found = parts(entry.time)
  if (found) return seconds ? found.hms : found.hm
  return entry.time || '-'
}

export function ago(entry: Alarm) {
  const at = atOf(entry)
  if (!at) return ''
  const minutes = Math.floor((ev.now - at) / 60000)
  if (minutes < 1) return 'just now'
  if (minutes < 60) return minutes + ' min ago'
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return hours + ' h' + (minutes % 60 ? ' ' + (minutes % 60) + ' min' : '') + ' ago'
  return dayLabel(dayOfEntry(entry))
}

export function dayLabel(day: string) {
  const [year, month, date] = day.split('-').map(Number)
  if (!year || !month || !date) return day
  const now = new Date(ev.now)
  if (day === dayOf(now)) return 'Today'
  if (day === dayOf(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1))) return 'Yesterday'
  return new Date(year, month - 1, date).toLocaleDateString(LOCALE, { weekday: 'short', day: 'numeric', month: 'short' })
}

export function dayLong(day: string) {
  const [year, month, date] = day.split('-').map(Number)
  if (!year || !month || !date) return day
  return new Date(year, month - 1, date).toLocaleDateString(LOCALE, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })
}

export function dayOfEntry(entry: Alarm) {
  return parts(entry.time)?.day || ''
}

export function monthLabel(month: string) {
  const [year, number] = month.split('-').map(Number)
  if (!year || !number) return ''
  return new Date(year, number - 1, 1).toLocaleDateString(LOCALE, { month: 'long', year: 'numeric' })
}

function kindOfFilter(id: string): Kind {
  if (/motion/i.test(id)) return 'motion'
  if (/human/i.test(id)) return 'human'
  return 'alarm'
}

export function kindOf(entry: Alarm): Kind {
  const base = (entry.event || '').split(':')[0]
  const filter = ev.filters.find((item) => item.id === base)
  return filter ? kindOfFilter(filter.id) : 'alarm'
}

export const cameras = computed(() => (ev.extra && !ev.devices.some((device) => device.id === ev.extra!.id) ? [...ev.devices, ev.extra] : ev.devices))
export const nameOf = (serial: string) => cameras.value.find((device) => device.id === serial)?.name || serial
export const rows = computed(() => [...ev.entries].sort((a, b) => (b.utcTime || 0) - (a.utcTime || 0)))
export function serverEvent(kind: KindFilter) {
  if (kind === 'all' || kind === 'alarm') return ''
  return ev.filters.find((filter) => kindOfFilter(filter.id) === kind)?.id || ''
}

export const counts = computed<Partial<Record<KindFilter, number>>>(() => (ev.tally.key === windowKey(ev.query) ? ev.tally.counts : {}))

function tallyAnswer(asked: Query, entries: Alarm[], total: number, finished: boolean) {
  const key = windowKey(asked)
  if (ev.tally.key !== key) ev.tally = { key, counts: {} }
  const found = ev.tally.counts
  if (asked.event) {
    found[kindOfFilter(asked.event)] = total
    return
  }
  found.all = total
  if (!finished) return
  const kinds: Record<Kind, number> = { motion: 0, human: 0, alarm: 0 }
  for (const entry of entries) kinds[kindOf(entry)] += 1
  Object.assign(found, kinds)
}
export const shown = computed(() => (ev.kind === 'all' ? rows.value : rows.value.filter((entry) => kindOf(entry) === ev.kind)))
export const chips = computed<Chip[]>(() => {
  const kinds = new Set<Kind>(ev.filters.map((filter) => kindOfFilter(filter.id)))
  kinds.add('alarm')
  const order: Kind[] = ['motion', 'alarm', 'human']
  const list: Chip[] = [{ id: 'all', label: 'All', icon: '', count: counts.value.all }]
  for (const kind of order) {
    if (!kinds.has(kind)) continue
    if (counts.value[kind] === 0 && ev.kind !== kind) continue
    list.push({ id: kind, label: KIND[kind].label, icon: KIND[kind].icon, count: counts.value[kind] })
  }
  return list
})

const PART = (hour: number) => (hour >= 17 ? 'Evening' : hour >= 12 ? 'Afternoon' : 'Morning')

export const groups = computed<Group[]>(() => {
  const list = shown.value
  const days = new Set(list.map((entry) => dayOfEntry(entry)))
  const oneDay = days.size <= 1 && ev.asked.range !== '7d' && ev.asked.range !== '24h'
  const out: Group[] = []
  for (const entry of list) {
    const found = parts(entry.time)
    const part = found ? PART(found.hour) : 'Earlier'
    const key = (found?.day || '') + '|' + part
    let group = out[out.length - 1]
    if (!group || group.key !== key) {
      group = { key, label: oneDay || !found ? part : dayLabel(found.day) + ', ' + part.toLowerCase(), entries: [] }
      out.push(group)
    }
    group.entries.push(entry)
  }
  return out
})

export const unread = computed(() => {
  if (ev.phase === 'idle' || ev.asked.serial !== ev.query.serial) return 0
  const mark = ev.seen[ev.asked.serial] || 0
  return rows.value.filter((entry) => atOf(entry) > mark).length
})

export const isNew = (entry: Alarm) => atOf(entry) > (ev.since[ev.asked.serial] ?? ev.seen[ev.asked.serial] ?? 0)

export function rangeLabel(query: Query = ev.query) {
  if (query.range === 'day') return dayLabel(query.day)
  return RANGES.find((item) => item.id === query.range)?.label || 'Today'
}

export function rangePhrase(query: Query = ev.asked) {
  if (query.range === 'today') return 'today'
  if (query.range === 'yesterday') return 'yesterday'
  if (query.range === '24h') return 'in the last 24 hours'
  if (query.range === '7d') return 'in the last 7 days'
  const label = dayLabel(query.day)
  return label === 'Today' || label === 'Yesterday' ? label.toLowerCase() : 'on ' + label
}

export const summary = computed(() => {
  if (ev.phase === 'idle') return 'Not checked yet'
  if (ev.phase === 'loading') return 'Checking for events…'
  if (ev.phase === 'error') return ev.checked ? 'Last checked ' + ev.checked : 'Not checked'
  if (ev.kind !== 'all') {
    const word = KIND[ev.kind].word + ' event'
    if (ev.asked.event) return ev.finished ? plural(ev.total || rows.value.length, word) + ' ' + rangePhrase() : 'Showing ' + rows.value.length + ' of ' + plural(ev.total, word) + ' ' + rangePhrase()
    return plural(shown.value.length, word) + ' ' + rangePhrase() + (ev.finished ? '' : ', in the first ' + rows.value.length + ' of ' + plural(ev.total))
  }
  if (!ev.finished) return 'Showing ' + rows.value.length + ' of ' + plural(ev.total) + ' ' + rangePhrase()
  return plural(ev.total || rows.value.length) + ' ' + rangePhrase()
})

export const deleteReason = computed(() => {
  if (ev.manage.refused) return ev.manage.refused
  if (ev.manage.busy) return 'Deleting…'
  if (!ev.manage.picked.size) return 'Select events to delete them.'
  return ''
})

export const clearReason = computed(() => {
  if (ev.manage.refused) return ev.manage.refused
  if (!ev.query.serial) return 'Choose a camera first.'
  return ''
})

export function channelOf(entry: Alarm) {
  const device = cameras.value.find((item) => item.id === ev.asked.serial)
  if (!device || (device.channels || 1) < 2) return ''
  return 'Channel ' + ((Number(entry.channel) || 0) + 1)
}

function pictureOf(object: string, bucket?: string) {
  return '/api/cloud/alarms/picture?sn=' + encodeURIComponent(ev.asked.serial) + '&object=' + encodeURIComponent(object) + '&bucket=' + encodeURIComponent(bucket || '')
}

export function srcOf(entry: Alarm) {
  if (ev.pictures !== true || ev.missing.has(entry.id)) return ''
  if (!entry.picture || !entry.picture.object) return ''
  return pictureOf(entry.picture.object, entry.picture.bucket)
}

export function pictureMissing(entry: Alarm) {
  ev.missing.add(entry.id)
}

async function probePictures() {
  const serial = ev.asked.serial
  if (ev.pictures !== 'unknown' || !serial) return
  const candidates = rows.value.filter((entry) => entry.picture && entry.picture.object && !ev.missing.has(entry.id)).slice(0, 3)
  if (!candidates.length) return
  ev.pictures = false
  for (const entry of candidates) {
    let reply: Response
    try {
      reply = await fetch(pictureOf(entry.picture!.object!, entry.picture!.bucket), { credentials: 'same-origin' })
    } catch {
      if (ev.asked.serial === serial) ev.pictures = 'unknown'
      return
    }
    if (ev.asked.serial !== serial) return
    if (reply.ok) {
      ev.pictures = true
      return
    }
    ev.missing.add(entry.id)
  }
}

function cameraFor(serial: string, entry: Alarm) {
  const channel = (Number(entry.channel) || 0) + 1
  return page.cameras.find((camera) => camera.device.id === serial && camera.channel === channel) || page.cameras.find((camera) => camera.device.id === serial) || null
}

export function playReason(entry: Alarm | null | undefined) {
  if (!entry) return 'Choose an event first.'
  if (!atOf(entry)) return 'This event has no time to play from.'
  if (!cameraFor(ev.asked.serial, entry)) return 'This camera is not in your camera list, so its recordings cannot be opened.'
  return ''
}

export function playFromLabel(entry: Alarm) {
  const found = /(\d{2}):(\d{2}):(\d{2})$/.exec(entry.time || '')
  if (!found) return ''
  const seconds = (Number(found[1]) * 3600 + Number(found[2]) * 60 + Number(found[3]) - PLAY_LEAD_MS / 1000 + 86400) % 86400
  return [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60].map(two).join(':')
}

export function playEvent(entry: Alarm) {
  if (playReason(entry)) return
  const serial = ev.asked.serial
  const camera = cameraFor(serial, entry)!
  markSeen()
  live.seek = { serial, at: atOfLocal(entry), kind: kindOf(entry), id: entry.id, token: live.seek.token + 1 }
  const wasPage = shell.view === 'events'
  if (wasPage || shell.mode !== 'single') go(liveHash('single'))
  if (page.selectedId !== serial || page.channel !== camera.channel || page.view !== 'live') openCamera(page.cameras.indexOf(camera))
  if (!wasPage && !shell.wide) closeEvents()
}

function loadSeenMark(serial: string) {
  return ev.seen[serial] || 0
}

export function beginViewing() {
  const serial = ev.query.serial
  if (!serial) return
  if (!opened.has(serial)) {
    opened.add(serial)
    ev.since[serial] = loadSeenMark(serial)
  }
  markSeen()
}

export function markSeen() {
  const serial = ev.asked.serial
  if (!serial || ev.asked.serial !== ev.query.serial || ev.phase !== 'ok') return
  const newest = rows.value.reduce((most, entry) => Math.max(most, atOf(entry)), 0)
  if (newest <= loadSeenMark(serial)) return
  if (ev.since[serial] === undefined) ev.since[serial] = loadSeenMark(serial)
  ev.seen[serial] = newest
  writeStore(SEEN_KEY, JSON.stringify(ev.seen))
}

function notify(entries: Alarm[]) {
  if (desktop || !ev.notify.on || typeof Notification === 'undefined') return
  if (Notification.permission !== 'granted' || document.visibilityState === 'visible') return
  const entry = entries.find((item) => !notified.has(item.id))
  if (!entry) return
  try {
    const note = new Notification(KIND[kindOf(entry)].label + ' · ' + nameOf(ev.asked.serial), {
      body: (entry.time || '') + (entry.name ? ' · ' + entry.name : ''),
      tag: 'pvw-alarm-' + entry.id,
    })
    note.onclick = () => window.focus()
  } catch {
    return
  }
}

function fail(status: number, message: string) {
  ev.phase = 'error'
  const detail = message ? capital(message) : status ? 'The alarm centre answered HTTP ' + status + '.' : 'No answer.'
  if (status === 401) {
    ev.problem = { title: 'Sign in with the cloud account to see events', text: 'Events come from the PROVIEW cloud, which the sign-in on this network cannot read.', detail }
    return
  }
  ev.problem = { title: 'Events could not be loaded', text: 'The alarm centre did not answer. PROVIEW tries again in ' + Math.max(5, ev.pollSeconds) + ' s.', detail }
}

function windowOf(query: Query): [string, string] {
  const now = new Date()
  if (query.range === 'day') return [query.day + ' 00:00:00', query.day + ' 23:59:59']
  if (query.range === 'today') return [dayOf(now) + ' 00:00:00', dayOf(now) + ' 23:59:59']
  if (query.range === 'yesterday') {
    const before = dayOf(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1))
    return [before + ' 00:00:00', before + ' 23:59:59']
  }
  const hours = query.range === '7d' ? 168 : 24
  return [stamp(new Date(now.getTime() - hours * 3600 * 1000)), stamp(now)]
}

export async function poll() {
  if (!ev.watching || !ev.query.serial) return
  if (document.hidden && !ev.notify.on) return
  const asked: Query = { serial: ev.query.serial, range: ev.query.range, day: ev.query.day, event: ev.query.event }
  if (inflight && same(inflight, asked)) return
  const mine = ++ticket
  inflight = asked
  clearTimeout(timer)
  timer = 0
  if (!same(ev.asked, asked) || ev.phase === 'idle') {
    ev.asked = asked
    ev.entries = []
    ev.checked = ''
    ev.problem = null
    ev.phase = 'loading'
  }
  const begun = performance.now()
  try {
    const [start, end] = windowOf(asked)
    let url = '/api/cloud/alarms?sn=' + encodeURIComponent(asked.serial) + '&start=' + encodeURIComponent(start) + '&end=' + encodeURIComponent(end)
    if (asked.event) url += '&event=' + encodeURIComponent(asked.event)
    const reply = await fetch(url, { credentials: 'same-origin' })
    const body: Listing = await reply.json().catch(() => ({}))
    if (mine !== ticket) return
    lastCheck = Date.now()
    if (!reply.ok) {
      fail(reply.status, body.message || '')
      return
    }
    ev.pollSeconds = body.pollSeconds || ev.pollSeconds
    const entries = body.entries || []
    const first = primed !== asked.serial
    primed = asked.serial
    if (!first) notify(entries)
    for (const entry of entries) notified.add(entry.id)
    ev.entries = entries
    tallyAnswer(asked, entries, body.total || entries.length, body.finished !== false)
    if (!asked.event && asked.range !== '24h' && asked.range !== '7d') {
      live.alarmDay = { serial: asked.serial, day: start.slice(0, 10), entries: entries.map((entry) => ({ id: entry.id, at: atOfLocal(entry), event: entry.event || '' })), token: live.alarmDay.token + 1 }
    }
    ev.manage.refused = body.canDelete === false ? body.deleteReason || 'This account may not delete the events of this camera.' : ''
    ev.total = body.total || entries.length
    ev.finished = body.finished !== false
    ev.now = Date.now()
    ev.checked = stamp(new Date()).slice(11, 16)
    ev.timing = 'vendor ' + Math.round(body.latencyMs || 0) + ' ms · here ' + Math.round(performance.now() - begun) + ' ms · every ' + ev.pollSeconds + ' s'
    ev.problem = null
    ev.phase = 'ok'
    if (!ev.selected || !entries.some((entry) => entry.id === ev.selected)) ev.selected = rows.value[0]?.id || ''
    if (eventsOpen.value && document.visibilityState === 'visible') markSeen()
    if (ev.pictures === 'unknown') probePictures()
  } catch (error) {
    if (mine === ticket) {
      lastCheck = Date.now()
      fail(0, 'the alarm centre could not be reached: ' + (error instanceof Error ? error.message : String(error)))
    }
  } finally {
    if (mine === ticket) {
      inflight = null
      schedule()
    }
  }
}

function schedule(delay = Math.max(5, ev.pollSeconds) * 1000) {
  clearTimeout(timer)
  timer = 0
  if (!ev.watching || !ev.query.serial) return
  if (document.hidden && !ev.notify.on) return
  timer = window.setTimeout(poll, delay)
}

export function retry() {
  ticket += 1
  inflight = null
  poll()
}

function resume() {
  if (timer || inflight) return
  const due = lastCheck + Math.max(5, ev.pollSeconds) * 1000 - Date.now()
  if (!lastCheck || due <= 0) poll()
  else schedule(due)
}

function forget() {
  notified = new Set()
  primed = ''
  ev.missing.clear()
  ev.pictures = 'unknown'
  ev.selected = ''
  ev.manage.picked.clear()
  ev.manage.confirm = ''
  ev.manage.note = ''
  ev.manage.refused = ''
  ev.month.days = []
  ev.month.phase = 'idle'
  ev.push.phase = 'idle'
  ev.push.text = ''
}

async function readJson(reply: Response) {
  return (await reply.json().catch(() => ({}))) as Record<string, unknown>
}

export async function loadFilters() {
  try {
    const reply = await fetch('/api/cloud/alarms/filters', { credentials: 'same-origin' })
    if (!reply.ok) return
    const body = (await reply.json()) as Filter[]
    if (Array.isArray(body) && body.every((item) => item && typeof item.id === 'string')) ev.filters = body.length ? body : KNOWN.slice()
  } catch {
    return
  }
}

export async function loadMonth() {
  const serial = ev.query.serial
  const month = ev.month.month
  if (!serial || !month) return
  ev.month.phase = 'loading'
  ev.month.error = ''
  try {
    const reply = await fetch('/api/cloud/alarms/calendar?sn=' + encodeURIComponent(serial) + '&month=' + encodeURIComponent(month), { credentials: 'same-origin' })
    const body = await readJson(reply)
    if (serial !== ev.query.serial || month !== ev.month.month) return
    if (!reply.ok) {
      ev.month.phase = 'error'
      ev.month.error = capital(String(body.message || 'the alarm centre answered HTTP ' + reply.status))
      return
    }
    ev.month.days = (body.days as Day[]) || []
    ev.month.phase = 'ok'
  } catch (error) {
    if (serial !== ev.query.serial || month !== ev.month.month) return
    ev.month.phase = 'error'
    ev.month.error = 'The alarm centre could not be reached: ' + (error instanceof Error ? error.message : String(error))
  }
}

export function openMonth() {
  const shownDay = ev.query.range === 'day' ? ev.query.day : dayOf(new Date())
  ev.month.month = shownDay.slice(0, 7)
  loadMonth()
}

export function stepMonth(delta: number) {
  const [year, month] = ev.month.month.split('-').map(Number)
  ev.month.month = monthOf(new Date(year, month - 1 + delta, 1))
  ev.month.days = []
  loadMonth()
}

export function setRange(range: Range, day = '') {
  ev.query.day = range === 'day' ? day : ''
  ev.query.range = range
}

export async function loadPush() {
  const serial = ev.query.serial
  if (!serial) return
  ev.push.phase = 'loading'
  ev.push.text = 'Checking phone push for this camera…'
  try {
    const reply = await fetch('/api/cloud/alarms/subscription?sn=' + encodeURIComponent(serial), { credentials: 'same-origin' })
    const body = await readJson(reply)
    if (serial !== ev.query.serial) return
    if (!reply.ok) {
      ev.push.phase = 'error'
      ev.push.text = 'Phone push is unknown: ' + String(body.message || 'the alarm centre answered HTTP ' + reply.status) + '.'
      return
    }
    const phones = Number(body.count) || 0
    ev.push.phase = 'ok'
    ev.push.text =
      (phones ? 'The alarm centre pushes this camera to ' + phones + (phones === 1 ? ' phone.' : ' phones.') : 'No phone gets push alarms for this camera.') +
      ' This app checks every ' + ev.pollSeconds + ' s instead.'
  } catch {
    if (serial !== ev.query.serial) return
    ev.push.phase = 'error'
    ev.push.text = 'Phone push is unknown: the alarm centre could not be reached.'
  }
}

export function setManage(on: boolean) {
  ev.manage.on = on
  ev.manage.picked.clear()
  ev.manage.confirm = ''
  ev.manage.note = ''
}

export function togglePick(id: string) {
  if (ev.manage.picked.has(id)) ev.manage.picked.delete(id)
  else ev.manage.picked.add(id)
}

export async function remove(all: boolean) {
  const serial = ev.asked.serial
  const ids = all ? [] : [...ev.manage.picked]
  if (!serial || (!all && !ids.length)) return
  ev.manage.busy = true
  ev.manage.note = ''
  try {
    const reply = await fetch('/api/cloud/alarms/delete', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ sn: serial, ids, all }),
    })
    const body = await readJson(reply)
    if (!reply.ok) {
      ev.manage.note = 'Nothing was deleted. ' + capital(String(body.message || 'the alarm centre answered HTTP ' + reply.status)) + '.'
      return
    }
    toast({ title: all ? 'Every event of ' + nameOf(serial) + ' was deleted' : plural(ids.length) + ' deleted', tone: 'ok' })
    ev.manage.picked.clear()
    ev.manage.confirm = ''
    ev.entries = all ? [] : ev.entries.filter((entry) => !ids.includes(entry.id))
    ev.month.days = []
    poll()
  } catch (error) {
    ev.manage.note = 'Nothing was deleted. The alarm centre could not be reached: ' + (error instanceof Error ? error.message : String(error))
  } finally {
    ev.manage.busy = false
    ev.manage.confirm = ev.manage.note ? ev.manage.confirm : ''
  }
}

export async function setNotify(on: boolean) {
  if (!on) {
    ev.notify.on = false
    ev.notify.note = ''
    writeStore(NOTIFY_KEY, 'off')
    schedule()
    return
  }
  if (typeof Notification === 'undefined') {
    ev.notify.on = false
    ev.notify.note = 'This browser cannot show notifications.'
    return
  }
  const permission = await Notification.requestPermission()
  ev.notify.on = permission === 'granted'
  ev.notify.note = ev.notify.on ? 'On. A notification shows when an event arrives while this tab is in the background.' : 'The browser blocks notifications for this site. Allow them in the browser settings.'
  writeStore(NOTIFY_KEY, ev.notify.on ? 'on' : 'off')
  schedule()
}

function syncDevices() {
  ev.devices = (page.devices || []) as Device[]
  if (cameras.value.some((device) => device.id === ev.query.serial)) return
  const wanted = ev.devices.find((device) => device.id === page.selectedId) || ev.devices[0]
  ev.query.serial = wanted ? wanted.id : ''
}

const bell = () => document.getElementById('alarm-bell')

export async function show() {
  openEvents()
  syncDevices()
  beginViewing()
  await new Promise((resolve) => requestAnimationFrame(resolve))
  document.querySelector<HTMLElement>('#alarm-sheet [data-device]')?.focus()
  if (ev.phase !== 'ok') poll()
}

export function close(returnFocus = true) {
  if (eventsOpen.value) closeEvents()
  if (returnFocus) bell()?.focus()
}

export function focusAlarm(detail: { id?: string; serial?: string; device?: string } = {}) {
  if (detail.serial) {
    ev.extra = { id: detail.serial, name: detail.device || detail.serial }
    ev.query.serial = detail.serial
  }
  ev.focus.id = detail.id || ''
  ev.focus.shown = ''
  if (detail.id) ev.selected = detail.id
  show()
}

function reset() {
  ticket += 1
  inflight = null
  lastCheck = 0
  ev.devices = []
  ev.extra = null
  ev.query.serial = ''
  ev.phase = 'idle'
  ev.asked = { serial: '', range: ev.query.range, day: ev.query.day, event: ev.query.event }
  ev.tally = { key: '', counts: {} }
  ev.entries = []
  ev.checked = ''
  ev.problem = null
  setManage(false)
  forget()
}

function syncScreen() {
  if (!page.signedIn) reset()
  const watchView = document.getElementById('watch')
  ev.watching = Boolean(watchView) && !watchView!.hidden
  clearInterval(clockTimer)
  clockTimer = 0
  if (ev.watching) {
    ev.now = Date.now()
    clockTimer = window.setInterval(() => (ev.now = Date.now()), 30000)
    syncDevices()
    resume()
    return
  }
  clearTimeout(timer)
  timer = 0
}

export function startEvents() {
  if (started) return
  started = true
  Object.assign(window, { ProviewEvents: { seek: () => ({ ...live.seek }) } })
  const screens = new MutationObserver(syncScreen)
  for (const id of ['watch', 'signin']) {
    const screen = document.getElementById(id)
    if (screen) screens.observe(screen, { attributes: true, attributeFilter: ['hidden'] })
  }
  window.addEventListener('proview:devices', () => {
    if (!ev.watching) return
    syncDevices()
    resume()
  })
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && ev.watching) {
      if (eventsOpen.value) markSeen()
      resume()
    } else schedule()
  })
  window.addEventListener('desktop-alarm', (event) => focusAlarm((event as CustomEvent).detail || {}))
  ev.notify.on = storedNotify()
  loadFilters()
  syncScreen()
}

watch(
  () => ev.query.serial,
  (serial, before) => {
    if (serial === before) return
    forget()
    poll()
    if (eventsOpen.value) beginViewing()
    if (shell.view === 'events') loadPush()
  },
)

watch(
  () => [ev.kind, ev.filters] as const,
  () => {
    ev.query.event = serverEvent(ev.kind)
  },
)

watch(
  () => [ev.query.range, ev.query.day, ev.query.event],
  () => poll(),
)

watch(
  () => live.top.selected,
  (key) => {
    const serial = key ? key.slice(0, key.lastIndexOf(':')) : ''
    if (serial && ev.devices.some((device) => device.id === serial)) ev.query.serial = serial
  },
)

watch(eventsOpen, (open) => {
  if (!open) {
    opened.clear()
    return
  }
  syncDevices()
  beginViewing()
})

watch(
  () => shell.view === 'events' && eventsOpen.value,
  (onPage) => {
    if (onPage) loadPush()
  },
)
