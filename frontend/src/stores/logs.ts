import { defineStore } from 'pinia'
import { ref } from 'vue'

export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

export interface LogEntry {
  id: string
  ts: Date
  category: string
  level: LogLevel
  message: string
}

const MAX = 2000

export const useLogsStore = defineStore('logs', () => {
  const entries = ref<LogEntry[]>([])

  function add(level: LogLevel, category: string, message: string) {
    if (entries.value.length >= MAX) entries.value.shift()
    entries.value.push({ id: crypto.randomUUID(), ts: new Date(), category, level, message })
  }

  const debug = (cat: string, msg: string) => add('debug', cat, msg)
  const info  = (cat: string, msg: string) => add('info',  cat, msg)
  const warn  = (cat: string, msg: string) => add('warn',  cat, msg)
  const error = (cat: string, msg: string) => add('error', cat, msg)
  const clear = () => { entries.value = [] }

  return { entries, debug, info, warn, error, clear }
})
