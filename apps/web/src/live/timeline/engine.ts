import { nextTick, reactive, watch } from 'vue'
import { api } from '../../lib/api'
import { AVC_TYPE, decodes, mayAskForHevc, rememberHevc, watchFirstFrame, type StreamShape } from '../../lib/hevc'
import { sessionLost } from '../../app/shell'
import { state as page } from '../../app/state'
import { shell } from '../../shell/layout'
import { decodeAlaw, PlaybackSound } from '../playback-audio'
import { deviceTime, fileAtTime, mb, stamp, type Clip } from '../playback-clips'
import { live } from '../store'
import { bridgeStage } from './stage-bridge'

export type Kind = 'all' | 'regular' | 'alarm'
export type Phase = 'idle' | 'loading' | 'ok' | 'error'
export type Marker = { id: string; at: number; kind: 'motion' | 'alarm'; label: string }
export type Thumb = { name: string; image?: string | null; error?: string | null; source?: string | null }
type Listing = { files?: Clip[]; truncated?: boolean; cap?: number }
type Alarm = { id?: string; event?: string; name?: string; time?: string; utcTime?: number }
type Frame = { type?: string; message?: string; [key: string]: unknown }
type Mark = { frame: number; at: number }
type Seek = { serial: string; at: number; kind: string; id: string; token: number }

export const SPEEDS = [0.5, 1, 2, 4, 8]
export const LEAD_MS = 5000
export const DAY_MS = 86400000
const TIME_WINDOW = 3600
const THUMB_BATCH = 6

const pad2 = (value: number) => String(value).padStart(2, '0')

export function dayOf(ms: number) {
  const at = new Date(ms)
  return at.getFullYear() + '-' + pad2(at.getMonth() + 1) + '-' + pad2(at.getDate())
}

export const today = () => dayOf(Date.now())

export function dayStart(day: string) {
  return stamp(day + ' 00:00:00') ?? 0
}

export function shiftDay(day: string, delta: number) {
  const at = new Date(dayStart(day))
  at.setDate(at.getDate() + delta)
  return dayOf(at.getTime())
}

const capital = (text: string) => (text ? text.charAt(0).toUpperCase() + text.slice(1) : '')
const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error))

export const tl = reactive({
  camera: '',
  device: '',
  day: today(),
  kind: 'all' as Kind,
  files: { phase: 'idle' as Phase, list: [] as Clip[], error: '', cap: 0, key: '', asked: { device: '', day: '', kind: 'all' as Kind } },
  events: { phase: 'idle' as Phase, list: [] as Marker[], error: '', key: '' },
  thumbs: {} as Record<string, Thumb>,
  status: { text: '', bad: false },
  sound: false,
  file: null as Clip | null,
  now: Date.now(),
  pending: false,
  tab: 'camera' as 'camera' | 'cloud',
})

let opener = ''

export function openRecordings(tab: 'camera' | 'cloud', from = '') {
  opener = from
  tl.tab = tab
  live.sheet = 'recordings'
}

export function takeOpener() {
  const element = opener ? document.getElementById(opener) : null
  opener = ''
  return element
}

const sound = new PlaybackSound()
const stream = {
  socket: null as WebSocket | null,
  media: null as MediaSource | null,
  buffer: null as SourceBuffer | null,
  queue: [] as Uint8Array[],
  objectUrl: '',
  bytes: 0,
  startedAt: 0,
  frameAt: null as number | null,
  status: {} as Record<string, any>,
  stats: null as Record<string, any> | null,
  marks: [] as Mark[],
  origin: null as number | null,
  fps: 0,
  closed: false,
  transport: '' as '' | 'hevc' | 'avc',
  stopHevcWatch: null as (() => void) | null,
  video: null as HTMLVideoElement | null,
}

let fileTicket = 0
let eventTicket = 0
let thumbTicket = 0
let playTicket = 0
let wanted: Clip[] = []
let thumbBusy = false
let retryAvc: (() => void) | null = null

export const device = () => page.selectedId || ''
const channel = () => Math.max(0, (page.channel || 1) - 1)
const element = () => document.getElementById('video') as HTMLVideoElement | null

export function say(text: string, bad = false) {
  tl.status.text = capital(text)
  tl.status.bad = bad
}

function stageMessage(title: string, text = '', code = '') {
  if (live.mode !== 'playback') return
  live.stage.title = title
  live.stage.text = text
  live.stage.code = code
}

function describeStats() {
  const parts: string[] = []
  const s = stream.status
  if (s.openMs !== undefined) parts.push('tunnel ' + (s.openMs / 1000).toFixed(2) + 's')
  if (s.primeMs !== undefined) parts.push('prime ' + (s.primeMs / 1000).toFixed(2) + 's')
  if (s.fps) parts.push(s.fps + ' fps' + (s.fpsSource && s.fpsSource !== 'settled' ? ' (' + s.fpsSource + ')' : ''))
  if (s.codec) parts.push(s.codec)
  if (stream.frameAt !== null) parts.push('first frame ' + (stream.frameAt / 1000).toFixed(2) + 's')
  else if (stream.stats && stream.stats.firstFrameMs !== null && stream.stats.firstFrameMs !== undefined) parts.push('muxed after ' + (stream.stats.firstFrameMs / 1000).toFixed(2) + 's')
  if (stream.bytes) parts.push(mb(stream.bytes))
  if (stream.stats && stream.stats.framesWritten) parts.push(stream.stats.framesWritten + ' frames out')
  if (s.windowSec) parts.push('window ' + s.windowSec + 's' + (stream.stats && stream.stats.capped ? ' (capped)' : ''))
  return parts.join(' · ')
}

function fileKey(asked: { device: string; day: string; kind: Kind }) {
  return asked.device + '|' + channel() + '|' + asked.day + '|' + asked.kind
}

let filesInFlight: Promise<void> = Promise.resolve()

export function loadFiles(force = false) {
  const asked = { device: device(), day: tl.day, kind: tl.kind }
  if (!asked.device || !page.signedIn) return Promise.resolve()
  const key = fileKey(asked)
  if (!force && tl.files.key === key && tl.files.phase !== 'error') return filesInFlight
  filesInFlight = fetchFiles(asked, key)
  return filesInFlight
}

async function fetchFiles(asked: { device: string; day: string; kind: Kind }, key: string) {
  const mine = ++fileTicket
  thumbTicket += 1
  wanted = []
  Object.assign(tl.files, { key, asked, phase: 'loading', list: [], error: '', cap: 0 })
  try {
    const started = performance.now()
    const body = await api<Listing>(
      '/api/playback/files?device=' + encodeURIComponent(asked.device) + '&channel=' + channel() + '&begin=' +
        encodeURIComponent(asked.day + ' 00:00:00') + '&end=' + encodeURIComponent(asked.day + ' 23:59:59') +
        (asked.kind !== 'all' ? '&event=' + asked.kind : ''),
    )
    if (mine !== fileTicket) return
    tl.files.list = (body && body.files) || []
    tl.files.cap = body && body.truncated ? body.cap || 0 : 0
    tl.files.phase = 'ok'
    if (live.playback.at !== null) tl.file = fileAtTime(tl.files.list, live.playback.at)
    if (performance.measure) performance.measure('playback-files', { start: started, end: performance.now() })
  } catch (error) {
    if (mine !== fileTicket) return
    if (sessionLost(error)) {
      tl.files.phase = 'idle'
      tl.files.key = ''
      return
    }
    tl.files.error = capital(messageOf(error))
    tl.files.phase = 'error'
  }
}

function kindOf(event: string | undefined): 'motion' | 'alarm' {
  return /motion/i.test(event || '') ? 'motion' : 'alarm'
}

export async function loadEvents(force = false) {
  const serial = device()
  const day = tl.day
  if (!serial || !page.signedIn) return
  const key = serial + '|' + day
  if (!force && tl.events.key === key && tl.events.phase !== 'error') return
  const mine = ++eventTicket
  Object.assign(tl.events, { key, phase: 'loading', list: [], error: '' })
  try {
    const body = await api<{ entries?: Alarm[] }>(
      '/api/cloud/alarms?sn=' + encodeURIComponent(serial) + '&start=' + encodeURIComponent(day + ' 00:00:00') + '&end=' + encodeURIComponent(day + ' 23:59:59'),
    )
    if (mine !== eventTicket) return
    const list: Marker[] = []
    for (const entry of (body && body.entries) || []) {
      const at = (entry.time ? stamp(entry.time) : null) ?? (entry.utcTime ? entry.utcTime * 1000 : null)
      if (at === null || dayOf(at) !== day) continue
      const kind = kindOf(entry.event)
      list.push({ id: entry.id || String(at), at, kind, label: kind === 'motion' ? 'Motion' : entry.name || 'Alarm' })
    }
    tl.events.list = list.sort((a, b) => a.at - b.at)
    tl.events.phase = 'ok'
  } catch (error) {
    if (mine !== eventTicket) return
    tl.events.error = capital(messageOf(error))
    tl.events.phase = 'error'
  }
}

export function loadDay(force = false) {
  void loadFiles(force)
  void loadEvents(force)
}

export function needThumbs(files: Clip[]) {
  for (const file of files) {
    if (!tl.thumbs[file.name] && !wanted.includes(file)) wanted.push(file)
  }
  void fetchThumbs()
}

async function fetchThumbs() {
  const owner = tl.files.asked.device
  if (thumbBusy || !wanted.length || !owner) return
  thumbBusy = true
  const mine = thumbTicket
  const batch = wanted.splice(0, THUMB_BATCH)
  for (const file of batch) tl.thumbs[file.name] = { name: file.name }
  try {
    const body = await api<{ thumbs?: Thumb[] }>('/api/playback/thumbnails', {
      method: 'POST',
      body: JSON.stringify({ device: owner, channel: channel(), files: batch.map((file) => ({ name: file.name, begin: file.begin, end: file.end })) }),
    })
    if (mine === thumbTicket) for (const thumb of (body && body.thumbs) || []) tl.thumbs[thumb.name] = thumb
  } catch (error) {
    if (mine === thumbTicket) for (const file of batch) tl.thumbs[file.name] = { name: file.name, error: messageOf(error) }
  } finally {
    if (mine !== thumbTicket) for (const file of batch) if (!tl.thumbs[file.name]?.image && !tl.thumbs[file.name]?.error) delete tl.thumbs[file.name]
    thumbBusy = false
  }
  void fetchThumbs()
}

function evict() {
  const buffer = stream.buffer
  const video = stream.video
  if (!buffer || buffer.updating || !video || !video.buffered.length) return false
  const behind = video.currentTime - 20
  if (video.buffered.start(0) < behind - 10) {
    try {
      buffer.remove(0, behind)
      return true
    } catch {
      return false
    }
  }
  return false
}

function pump() {
  const buffer = stream.buffer
  if (!buffer || buffer.updating || !stream.queue.length) return
  try {
    buffer.appendBuffer(stream.queue.shift() as BufferSource)
  } catch (error) {
    if (error instanceof DOMException && error.name === 'QuotaExceededError' && evict()) return
    if (stream.transport === 'hevc' && stream.stopHevcWatch) return fallBack('the H.265 fragment was refused')
    console.error('playback append failed', error)
    stream.queue = []
  }
}

function mediaNow() {
  const video = stream.video
  if (!video || stream.origin === null) return null
  return Math.max(0, video.currentTime - stream.origin)
}

function realAt(seconds: number) {
  if (!stream.fps || !stream.marks.length) return null
  const frame = seconds * stream.fps
  let mark = stream.marks[0]
  for (const candidate of stream.marks) {
    if (candidate.frame <= frame) mark = candidate
    else break
  }
  return mark.at + ((frame - mark.frame) / stream.fps) * 1000
}

function follow() {
  const video = stream.video
  const now = mediaNow()
  if (!video) return
  live.playback.paused = video.paused
  if (now === null) return
  sound.tick(now, !video.paused && video.playbackRate === 1 && tl.sound)
  const at = realAt(now)
  if (at === null) return
  live.playback.at = at
  const inside = fileAtTime(tl.files.list, at)
  if (inside) tl.file = inside
}

function report() {
  const socket = stream.socket
  const now = mediaNow()
  if (!socket || socket.readyState !== WebSocket.OPEN || !stream.fps) return
  socket.send(JSON.stringify({ type: 'position', frame: Math.floor((now || 0) * stream.fps) }))
}

function onFrame(frame: Frame) {
  if (frame.type === 'clock') {
    const at = stamp(String(frame.at || ''))
    if (at !== null && typeof frame.frame === 'number') stream.marks.push({ frame: frame.frame, at })
    return true
  }
  if (frame.type === 'audio') {
    if (typeof frame.data === 'string' && typeof frame.frame === 'number' && stream.fps) {
      sound.enqueue(frame.frame / stream.fps, decodeAlaw(frame.data), Number(frame.rate) || 8000)
    }
    return true
  }
  return false
}

function continueAfterEnd() {
  if (!stream.closed || !stream.stats || !stream.stats.capped || live.playback.at === null || live.mode !== 'playback') return
  start(deviceTime(live.playback.at + 1000))
}

function fallBack(reason: string) {
  rememberHevc('no')
  console.warn('H.265 playback failed, switching to the H.264 transcode:', reason)
  const again = retryAvc
  if (again) again()
}

function shown() {
  if (live.playback.picture) return
  live.playback.picture = true
  tl.pending = false
  stageMessage('')
}

function openBuffer(media: MediaSource, type: string, transport: 'hevc' | 'avc', shape?: StreamShape) {
  if (stream.buffer || stream.media !== media) return
  stream.transport = transport
  let buffer: SourceBuffer
  try {
    buffer = media.addSourceBuffer(type)
  } catch (error) {
    if (transport === 'hevc') return fallBack('the browser refused ' + type)
    const text = 'this browser cannot decode the clip: ' + messageOf(error)
    say(text, true)
    stageMessage('The recording cannot play here', capital(text))
    return
  }
  stream.buffer = buffer
  buffer.addEventListener('updateend', () => {
    const video = stream.video
    if (!evict()) pump()
    if (video && video.buffered.length && stream.origin === null) stream.origin = video.buffered.start(0)
    if (video && video.buffered.length && stream.frameAt === null) {
      stream.frameAt = performance.now() - stream.startedAt
      shown()
      say(describeStats())
    }
  })
  buffer.addEventListener('error', () => {
    if (stream.transport === 'hevc' && stream.buffer === buffer) fallBack('the source buffer reported an error')
  })
  if (transport === 'hevc' && shape && stream.video) {
    stream.stopHevcWatch = watchFirstFrame(
      stream.video,
      () => stream.media === media && stream.transport === 'hevc',
      (ok, reason) => {
        stream.stopHevcWatch = null
        if (ok) rememberHevc('yes', shape)
        else fallBack(reason)
      },
    )
  }
  pump()
}

function chooseBuffer(media: MediaSource, frame: Frame) {
  if (typeof frame.height === 'number' && frame.height > 0) live.playback.quality = frame.height >= 720 ? 'HD' : 'SD'
  if (stream.buffer || typeof frame.mime !== 'string') return
  if (frame.transport !== 'hevc') return openBuffer(media, frame.mime, 'avc')
  const shape: StreamShape = { codec: String(frame.codecString || ''), width: Number(frame.width) || null, height: Number(frame.height) || null }
  decodes(shape).then((ok) => {
    if (stream.media !== media) return
    if (!ok) return fallBack('the browser cannot decode ' + shape.codec + ' at ' + shape.width + 'x' + shape.height)
    openBuffer(media, String(frame.mime), 'hevc', shape)
  })
}

function onVideoEnd() {
  continueAfterEnd()
}

function release() {
  if (stream.objectUrl) {
    URL.revokeObjectURL(stream.objectUrl)
    stream.objectUrl = ''
  }
  if (stream.socket) {
    stream.socket.onclose = null
    stream.socket.onerror = null
    stream.socket.onmessage = null
    stream.socket.close()
    stream.socket = null
  }
  if (stream.media) {
    try {
      if (stream.media.readyState === 'open') stream.media.endOfStream()
    } catch {}
    stream.media = null
  }
  if (stream.video) {
    stream.video.removeEventListener('ended', onVideoEnd)
    stream.video.removeEventListener('waiting', onVideoEnd)
  }
  stream.buffer = null
  stream.queue = []
  stream.bytes = 0
  stream.stats = null
  stream.frameAt = null
  stream.transport = ''
  if (stream.stopHevcWatch) stream.stopHevcWatch()
  stream.stopHevcWatch = null
  stream.marks = []
  stream.origin = null
  stream.fps = 0
  stream.closed = false
  sound.clear()
  live.playback.picture = false
}

function start(begin: string, askHevc = mayAskForHevc()) {
  release()
  retryAvc = askHevc ? () => start(begin, false) : null
  stream.status = {}
  stream.startedAt = performance.now()
  const video = element()
  stream.video = video
  if (!video) return
  const at = stamp(begin)
  live.playback.at = at
  tl.file = at === null ? null : fileAtTime(tl.files.list, at)
  tl.pending = true
  if (!window.MediaSource) {
    const text = 'this browser has no MediaSource, so the recording cannot play here.'
    say(text, true)
    stageMessage('The recording cannot play here', capital(text))
    return
  }
  say('opening the cloud tunnel…')
  stageMessage('Opening the recording', 'At ' + begin.slice(11) + ' on ' + begin.slice(0, 10) + '. The server is asking the camera for its recording.')
  const media = new MediaSource()
  stream.media = media
  stream.objectUrl = URL.createObjectURL(media)
  video.addEventListener('ended', onVideoEnd)
  video.addEventListener('waiting', onVideoEnd)
  video.src = stream.objectUrl
  video.defaultPlaybackRate = live.playback.speed
  video.playbackRate = live.playback.speed
  const serial = device()
  const lane = channel()
  const day = begin.slice(0, 10)
  media.addEventListener(
    'sourceopen',
    () => {
      if (stream.media !== media) return
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const params =
        'device=' + encodeURIComponent(serial) +
        '&channel=' + lane +
        '&name=' +
        '&begin=' + encodeURIComponent(begin) +
        '&end=' + encodeURIComponent(day + ' 23:59:59') +
        '&start=0' +
        '&window=' + TIME_WINDOW +
        '&mode=time' +
        (askHevc ? '&codec=hevc' : '')
      const socket = new WebSocket(scheme + '://' + location.host + '/ws/playback?' + params)
      socket.binaryType = 'arraybuffer'
      stream.socket = socket
      socket.onmessage = (event) => {
        if (typeof event.data === 'string') {
          let frame: Frame | null = null
          try {
            frame = JSON.parse(event.data)
          } catch {
            frame = null
          }
          if (!frame) return
          if (onFrame(frame)) return
          if (frame.type === 'status') {
            Object.assign(stream.status, frame)
            chooseBuffer(media, frame)
            if (typeof frame.fps === 'number' && frame.fps > 0) {
              stream.fps = frame.fps
              report()
            }
            say(describeStats() || frame.message || '')
          } else if (frame.type === 'error') {
            const text = frame.message || 'the playback failed'
            say(text, true)
            tl.pending = false
            stageMessage('The recording did not open', 'At ' + begin.slice(11) + ' on ' + begin.slice(0, 10) + '.', text)
          } else if (frame.type === 'stats') {
            stream.stats = frame
            say(describeStats())
          }
          return
        }
        const bytes = new Uint8Array(event.data)
        stream.bytes += bytes.length
        stream.queue.push(bytes)
        if (!stream.buffer && !stream.transport) openBuffer(media, AVC_TYPE, 'avc')
        pump()
        if (stream.status.openMs !== undefined) say(describeStats())
      }
      socket.onerror = () => {
        say('the playback socket failed to connect', true)
        tl.pending = false
        stageMessage('The recording did not open', 'The playback socket failed to connect.')
      }
      socket.onclose = () => {
        stream.closed = true
        if (stream.media && stream.media.readyState === 'open') {
          try {
            stream.media.endOfStream()
          } catch {}
        }
      }
      video.play().catch(() => {})
    },
    { once: true },
  )
}

export async function playAt(ms: number) {
  if (!device() || !page.signedIn) return
  const mine = ++playTicket
  const at = Math.floor(ms / 1000) * 1000
  if (dayOf(at) !== tl.day) {
    tl.day = dayOf(at)
    loadDay()
  }
  live.playback.at = at
  if (live.mode !== 'playback') {
    live.mode = 'playback'
    await nextTick()
    bridgeStage('playback')
    if (mine !== playTicket || live.mode !== 'playback') return
  }
  start(deviceTime(at))
}

export function playEvent(at: number) {
  return playAt(at - LEAD_MS)
}

export function backToLive() {
  if (live.mode !== 'playback') return
  playTicket += 1
  const video = stream.video
  const owned = Boolean(video && stream.objectUrl && video.src === stream.objectUrl)
  release()
  if (video && owned) {
    video.removeAttribute('src')
    video.load()
  }
  stream.video = null
  if (tl.sound) toggleSound()
  live.stage.title = ''
  live.stage.text = ''
  live.stage.code = ''
  live.playback.at = null
  live.playback.paused = false
  tl.pending = false
  tl.file = null
  say('')
  live.mode = 'live'
  bridgeStage('live')
}

export function setSpeed(value: number) {
  live.playback.speed = value
  const video = stream.video
  if (video) {
    video.defaultPlaybackRate = value
    video.playbackRate = value
  }
  if (value !== 1 && tl.sound) sound.clear()
}

export function togglePause() {
  const video = stream.video
  if (!video || live.mode !== 'playback') return
  if (video.paused) video.play().catch(() => {})
  else video.pause()
  live.playback.paused = video.paused
}

export function stepFrame(direction: 1 | -1) {
  const video = stream.video
  if (!video || !live.playback.picture) return
  if (!video.paused) video.pause()
  live.playback.paused = true
  const step = 1 / (stream.fps || 25)
  const target = video.currentTime + direction * step
  const buffered = video.buffered
  if (!buffered.length) return
  const lowest = buffered.start(0)
  const highest = buffered.end(buffered.length - 1)
  video.currentTime = Math.min(Math.max(target, lowest), highest)
  window.setTimeout(follow, 50)
}

export function toggleSound() {
  if (tl.sound) {
    tl.sound = false
    sound.disable()
    return
  }
  if (!sound.enable()) return say('this browser has no Web Audio, so the recording plays without sound', true)
  tl.sound = true
}

export function snapshot() {
  const video = stream.video
  if (!video || !video.videoWidth) return say('the snapshot needs a picture on screen', true)
  const canvas = document.createElement('canvas')
  canvas.width = video.videoWidth
  canvas.height = video.videoHeight
  const context = canvas.getContext('2d')
  if (!context) return
  context.drawImage(video, 0, 0)
  const at = live.playback.at
  const label = at === null ? 'playback' : deviceTime(at).replace(/[: ]/g, '-')
  canvas.toBlob((blob) => {
    if (!blob) return say('the snapshot could not be encoded', true)
    const link = document.createElement('a')
    link.href = URL.createObjectURL(blob)
    link.download = device() + '_playback_' + label + '.png'
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(link.href), 4000)
    say('saved a snapshot of ' + (at === null ? 'the playback' : deviceTime(at)))
  }, 'image/png')
}

export function downloadFile(file: Clip) {
  const owner = tl.files.asked.device || device()
  const anchor = document.createElement('a')
  anchor.href =
    '/api/playback/download?device=' + encodeURIComponent(owner) +
    '&channel=' + channel() +
    '&name=' + encodeURIComponent(file.name) +
    '&begin=' + encodeURIComponent(file.begin) +
    '&end=' + encodeURIComponent(file.end) +
    '&bytes=' + encodeURIComponent(String(file.bytes || 0))
  anchor.download = ''
  document.body.append(anchor)
  anchor.click()
  anchor.remove()
  say('the device is streaming ' + mb(file.bytes) + '; the browser saves the clip when it lands')
}

export function setDay(day: string) {
  if (!day || day === tl.day) return
  tl.day = day
  loadDay()
}

export function setKind(kind: Kind) {
  if (tl.kind === kind) return
  tl.kind = kind
  void loadFiles()
}

function reset() {
  fileTicket += 1
  eventTicket += 1
  thumbTicket += 1
  wanted = []
  tl.thumbs = {}
  Object.assign(tl.files, { phase: 'idle', list: [], error: '', cap: 0, key: '' })
  Object.assign(tl.events, { phase: 'idle', list: [], error: '', key: '' })
  tl.file = null
}

const watching = () => live.settings.signedIn && shell.screen === 'watch' && shell.view === 'live' && shell.mode === 'single' && Boolean(tl.device)
let pendingSeek: Seek | null = null

function trySeek() {
  const seek = pendingSeek
  if (!seek || !watching() || device() !== seek.serial) return
  pendingSeek = null
  void playEvent(seek.at)
}

watch(
  () => live.top.selected,
  (camera) => {
    if (camera === tl.camera) return
    tl.camera = camera
    tl.device = device()
    if (live.mode === 'playback') backToLive()
    reset()
    tl.day = today()
    if (watching()) loadDay()
    trySeek()
  },
)

watch(watching, (on) => {
  if (on) {
    loadDay()
    trySeek()
    return
  }
  if (live.mode === 'playback') backToLive()
  if (live.sheet === 'recordings') live.sheet = ''
})

window.addEventListener(
  'keydown',
  (event) => {
    if (live.sheet !== 'recordings' || event.key.length !== 1 || event.key === ' ') return
    if (event.target instanceof Element && event.target.closest('input, textarea, select')) return
    event.stopPropagation()
  },
  true,
)

window.addEventListener('desktop-live', () => {
  if (live.sheet === 'recordings') live.sheet = ''
  backToLive()
})

watch(
  () => live.settings.signedIn,
  (signed) => {
    if (signed) return
    if (live.mode === 'playback') backToLive()
    reset()
    tl.camera = ''
    tl.device = ''
  },
)

watch(
  () => (live as unknown as { seek?: Seek }).seek?.token,
  () => {
    const seek = (live as unknown as { seek?: Seek }).seek
    if (!seek || !seek.serial) return
    pendingSeek = { ...seek }
    trySeek()
  },
)

window.setInterval(() => {
  if (live.mode === 'playback') follow()
}, 100)
window.setInterval(() => {
  if (live.mode === 'playback') report()
}, 500)
window.setInterval(() => {
  tl.now = Date.now()
  if (live.mode === 'live' && tl.day !== today() && tl.files.phase === 'ok' && dayOf(tl.now - 2000) === tl.day && watching()) setDay(today())
}, 1000)
