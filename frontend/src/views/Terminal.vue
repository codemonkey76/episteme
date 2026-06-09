<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import '@xterm/xterm/css/xterm.css'
import * as api from '../api'
import { useTerminalsStore } from '../stores/terminals'

const props = withDefaults(defineProps<{ shell?: 'bash' | 'pwsh' }>(), { shell: 'bash' })

const store = useTerminalsStore()
const id = crypto.randomUUID()

const host = ref<HTMLElement>()
let term: Terminal | null = null
let fit: FitAddon | null = null
let search: SearchAddon | null = null
let ws: WebSocket | null = null
let ro: ResizeObserver | null = null
const enc = new TextEncoder()

// Find-in-scrollback bar
const showFind = ref(false)
const findQuery = ref('')
// Persistent history panel
const showHistory = ref(false)
const historyQuery = ref('')
const historyItems = ref<api.TerminalHistoryEntry[]>([])

function sendResize() {
  if (!term || ws?.readyState !== WebSocket.OPEN) return
  ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }))
}

function paste(cmd: string) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(enc.encode(cmd))
  term?.focus()
}

function scrollback(): string {
  if (!term) return ''
  const buf = term.buffer.active
  const lines: string[] = []
  const start = Math.max(0, buf.length - 60)
  for (let i = start; i < buf.length; i++) {
    const line = buf.getLine(i)
    if (line) lines.push(line.translateToString(true))
  }
  return lines.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

onMounted(async () => {
  await nextTick()
  if (!host.value) return

  term = new Terminal({
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
    fontSize: 13,
    cursorBlink: true,
    theme: { background: '#0d0d0d', foreground: '#d4d4d4', cursor: '#d4d4d4' },
  })
  fit = new FitAddon()
  search = new SearchAddon()
  term.loadAddon(fit)
  term.loadAddon(search)
  term.open(host.value)
  fit.fit()

  // Capture commands via the shell-integration OSC 633;E sequence and store
  // them for the searchable history; swallow the sequence so it never renders.
  term.parser.registerOscHandler(633, (data: string) => {
    if (data.startsWith('E;')) {
      const cmd = data.slice(2)
      if (cmd.trim()) api.terminals.record(props.shell, cmd).catch(() => {})
    }
    return true
  })

  ws = new WebSocket(api.terminals.wsUrl(props.shell, term.cols, term.rows))
  ws.binaryType = 'arraybuffer'
  ws.onmessage = (e) => {
    if (typeof e.data === 'string') term?.write(e.data)
    else term?.write(new Uint8Array(e.data))
  }
  ws.onclose = () => term?.write('\r\n\x1b[90m[session ended]\x1b[0m\r\n')
  ws.onerror = () => term?.write('\r\n\x1b[31m[connection error]\x1b[0m\r\n')

  term.onData((d) => {
    if (ws?.readyState === WebSocket.OPEN) ws.send(enc.encode(d))
  })

  ro = new ResizeObserver(() => {
    try {
      fit?.fit()
      sendResize()
    } catch {}
  })
  ro.observe(host.value)
  host.value.addEventListener('focusin', () => store.setActive(id))

  store.register(id, props.shell, { shell: props.shell, paste, scrollback })
  term.focus()
})

onBeforeUnmount(() => {
  store.unregister(id)
  ro?.disconnect()
  ws?.close()
  term?.dispose()
})

// ── Find in scrollback ──
function toggleFind() {
  showFind.value = !showFind.value
  if (!showFind.value) search?.clearDecorations()
}
function findNext() {
  if (findQuery.value) search?.findNext(findQuery.value)
}
function findPrev() {
  if (findQuery.value) search?.findPrevious(findQuery.value)
}

// ── Persistent history ──
async function toggleHistory() {
  showHistory.value = !showHistory.value
  if (showHistory.value) await loadHistory()
}
async function loadHistory() {
  try {
    const res = await api.terminals.history(props.shell, historyQuery.value || undefined)
    historyItems.value = res.history
  } catch {
    historyItems.value = []
  }
}
function useHistory(cmd: string) {
  paste(cmd)
  showHistory.value = false
}
</script>

<template>
  <div class="flex flex-col h-full bg-[#0d0d0d] overflow-hidden">
    <div class="flex items-center gap-1.5 px-2 py-1 border-b border-[var(--c-1e1e1e)] shrink-0 text-[var(--c-808080)]">
      <span class="text-[0.7rem] uppercase tracking-[0.06em] mr-1">{{ shell === 'pwsh' ? 'PowerShell' : 'bash' }}</span>
      <button class="ml-auto text-[0.72rem] px-2 py-0.5 rounded hover:bg-[var(--c-222222)] hover:text-fg" :class="showFind ? 'bg-[var(--c-222222)] text-fg' : ''" @click="toggleFind">Find</button>
      <button class="text-[0.72rem] px-2 py-0.5 rounded hover:bg-[var(--c-222222)] hover:text-fg" :class="showHistory ? 'bg-[var(--c-222222)] text-fg' : ''" @click="toggleHistory">History</button>
    </div>

    <div v-if="showFind" class="flex items-center gap-1.5 px-2 py-1 border-b border-[var(--c-1e1e1e)] shrink-0">
      <input
        v-model="findQuery"
        placeholder="Find in terminal…"
        class="flex-1 text-[0.78rem] text-fg bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-2 py-1 focus:outline-none focus:border-[var(--c-3a3a3a)]"
        @keydown.enter.exact.prevent="findNext"
        @keydown.shift.enter.prevent="findPrev"
      />
      <button class="text-[0.72rem] px-2 py-1 rounded bg-[var(--c-222222)] text-fg hover:bg-[var(--c-2a2a2a)]" @click="findPrev">↑</button>
      <button class="text-[0.72rem] px-2 py-1 rounded bg-[var(--c-222222)] text-fg hover:bg-[var(--c-2a2a2a)]" @click="findNext">↓</button>
    </div>

    <div v-if="showHistory" class="flex flex-col gap-1 px-2 py-1.5 border-b border-[var(--c-1e1e1e)] shrink-0 max-h-52 overflow-y-auto">
      <input
        v-model="historyQuery"
        placeholder="Search past commands…"
        class="text-[0.78rem] text-fg bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-2 py-1 focus:outline-none focus:border-[var(--c-3a3a3a)]"
        @input="loadHistory"
      />
      <p v-if="historyItems.length === 0" class="text-[0.72rem] text-[var(--c-585858)] px-1 py-1">No matching commands.</p>
      <button
        v-for="h in historyItems"
        :key="h.id"
        class="text-left font-mono text-[0.74rem] text-[var(--c-b0b0b0)] px-2 py-1 rounded hover:bg-[var(--c-1e1e1e)] hover:text-fg truncate"
        :title="h.command"
        @click="useHistory(h.command)"
      >{{ h.command }}</button>
    </div>

    <div ref="host" class="flex-1 min-h-0 px-1.5 pt-1"></div>
  </div>
</template>
