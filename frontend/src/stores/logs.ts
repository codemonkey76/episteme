import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api'

export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

export interface LogEntry {
  id: string
  ts: number
  category: string
  level: LogLevel
  message: string
}

const MAX = 2000

export const useLogsStore = defineStore('logs', () => {
  const entries = ref<LogEntry[]>([])
  const connected = ref(false)
  let sse: EventSource | null = null
  const seen = new Set<string>()

  function _push(entry: LogEntry) {
    if (seen.has(entry.id)) return
    seen.add(entry.id)
    if (entries.value.length >= MAX) {
      const removed = entries.value.shift()
      if (removed) seen.delete(removed.id)
    }
    entries.value.push(entry)
  }

  async function init() {
    if (sse) return

    // Load persisted entries
    try {
      const res = await api.logs.list({ limit: MAX })
      for (const e of res.entries) _push(e as LogEntry)
    } catch {}

    // Open SSE stream for real-time updates
    sse = new EventSource(api.logs.streamUrl)
    sse.onopen = () => { connected.value = true }
    sse.onerror = () => { connected.value = false }
    sse.onmessage = (e) => {
      try { _push(JSON.parse(e.data) as LogEntry) } catch {}
    }
  }

  async function add(level: LogLevel, category: string, message: string) {
    const entry: LogEntry = {
      id: crypto.randomUUID(),
      ts: Date.now(),
      category,
      level,
      message,
    }
    _push(entry)
    // Fire-and-forget to backend; SSE will echo it back but dedup handles that
    api.logs.create(entry).catch(() => {})
  }

  const debug = (cat: string, msg: string) => add('debug', cat, msg)
  const info  = (cat: string, msg: string) => add('info',  cat, msg)
  const warn  = (cat: string, msg: string) => add('warn',  cat, msg)
  const error = (cat: string, msg: string) => add('error', cat, msg)

  async function clear() {
    entries.value = []
    seen.clear()
    api.logs.clear().catch(() => {})
  }

  return { entries, connected, init, debug, info, warn, error, clear }
})
