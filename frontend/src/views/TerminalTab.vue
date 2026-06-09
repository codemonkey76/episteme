<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import '@xterm/xterm/css/xterm.css'
import * as api from '../api'

type Shell = 'bash' | 'pwsh'

const props = defineProps<{ active: boolean; sidebarWidth: number; tabId: string; initialShell: Shell }>()
const emit = defineEmits<{
  (e: 'update:sidebarWidth', w: number): void
  (e: 'shell', s: Shell): void
}>()

const shell = ref<Shell>(props.initialShell)
// Stable across refresh: the tab's id is the terminal id, so the backend can
// restore this terminal's saved scrollback on reconnect.
const sessionId = ref(props.tabId)

const rootEl = ref<HTMLElement>()
const host = ref<HTMLElement>()
let term: Terminal | null = null
let fit: FitAddon | null = null
let search: SearchAddon | null = null
let ws: WebSocket | null = null
let ro: ResizeObserver | null = null
const enc = new TextEncoder()

// Auto-copy terminal selections, with a brief "Copied" toast.
const copied = ref(false)
let copyDebounce: ReturnType<typeof setTimeout> | null = null
let copiedTimer: ReturnType<typeof setTimeout> | null = null

// Terminal toolbar state
const showFind = ref(false)
const findQuery = ref('')
const showHistory = ref(false)
const historyQuery = ref('')
const historyItems = ref<api.TerminalHistoryEntry[]>([])
// Archive (full scrollback) search across all sessions and restarts.
const showLog = ref(false)
const logQuery = ref('')
const logHits = ref<api.TerminalOutputHit[]>([])
const logBusy = ref(false)

function sendResize() {
  if (!term || ws?.readyState !== WebSocket.OPEN) return
  ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }))
}

function teardown() {
  ro?.disconnect(); ro = null
  ws?.close(); ws = null
  term?.dispose(); term = null
  fit = null; search = null
  if (copyDebounce) { clearTimeout(copyDebounce); copyDebounce = null }
  if (copiedTimer) { clearTimeout(copiedTimer); copiedTimer = null }
  copied.value = false
}

// Flash the "Copied to clipboard" toast for ~1s.
function showCopied() {
  copied.value = true
  if (copiedTimer) clearTimeout(copiedTimer)
  copiedTimer = setTimeout(() => { copied.value = false }, 1000)
}

function connect() {
  teardown()
  if (!host.value) return
  const sid = props.tabId
  sessionId.value = sid

  term = new Terminal({
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
    fontSize: 13,
    cursorBlink: true,
    // Hold a generous scrollback so restored history stays scrollable; the full
    // archive lives in the backend and is reachable via the log search.
    scrollback: 10000,
    theme: { background: '#0d0d0d', foreground: '#d4d4d4', cursor: '#d4d4d4' },
  })
  fit = new FitAddon()
  search = new SearchAddon()
  term.loadAddon(fit)
  term.loadAddon(search)
  term.open(host.value)
  fit.fit()

  term.parser.registerOscHandler(633, (data: string) => {
    if (data.startsWith('E;')) {
      const cmd = data.slice(2)
      if (cmd.trim()) api.terminals.record(shell.value, cmd).catch(() => {})
    }
    return true
  })

  ws = new WebSocket(api.terminals.wsUrl(shell.value, sid, term.cols, term.rows))
  ws.binaryType = 'arraybuffer'
  ws.onmessage = (e) => {
    if (typeof e.data === 'string') term?.write(e.data)
    else term?.write(new Uint8Array(e.data))
  }
  ws.onclose = () => term?.write('\r\n\x1b[90m[session ended]\x1b[0m\r\n')

  term.onData((d) => {
    if (ws?.readyState === WebSocket.OPEN) ws.send(enc.encode(d))
  })

  // Auto-copy whatever the user selects. Debounced so a drag-select copies
  // once it settles, not on every intermediate change; clearing the selection
  // (empty) is ignored.
  term.onSelectionChange(() => {
    if (copyDebounce) clearTimeout(copyDebounce)
    copyDebounce = setTimeout(() => {
      const sel = term?.getSelection() ?? ''
      if (!sel) return
      navigator.clipboard?.writeText(sel).then(showCopied).catch(() => {})
    }, 120)
  })

  ro = new ResizeObserver(() => {
    try { fit?.fit(); sendResize() } catch {}
  })
  ro.observe(host.value)
  term.focus()
}

onMounted(() => {
  connect()
  emit('shell', shell.value)
})
watch(shell, () => { connect(); emit('shell', shell.value) })
// Refit + focus when this tab becomes the active one (it was hidden = 0-sized).
watch(() => props.active, (now) => {
  if (now) nextTick(() => { try { fit?.fit(); sendResize() } catch {}; term?.focus() })
})
onBeforeUnmount(() => { teardown(); agentAbort?.abort() })

// ── Divider drag (resize the AI sidebar) ──
function startDrag(e: MouseEvent) {
  e.preventDefault()
  const onMove = (ev: MouseEvent) => {
    if (!rootEl.value) return
    const rect = rootEl.value.getBoundingClientRect()
    const w = Math.max(260, Math.min(760, rect.right - ev.clientX))
    emit('update:sidebarWidth', w)
  }
  const onUp = () => {
    window.removeEventListener('mousemove', onMove)
    window.removeEventListener('mouseup', onUp)
    document.body.style.userSelect = ''
  }
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
  document.body.style.userSelect = 'none'
}

// ── Find in scrollback ──
function toggleFind() {
  showFind.value = !showFind.value
  if (!showFind.value) search?.clearDecorations()
}
function findNext() { if (findQuery.value) search?.findNext(findQuery.value) }
function findPrev() { if (findQuery.value) search?.findPrevious(findQuery.value) }

// ── Persistent history dropdown ──
async function toggleHistory() {
  showHistory.value = !showHistory.value
  if (showHistory.value) await loadHistory()
}
async function loadHistory() {
  try {
    historyItems.value = (await api.terminals.history(shell.value, historyQuery.value || undefined)).history
  } catch { historyItems.value = [] }
}
function useHistory(cmd: string) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(enc.encode(cmd))
  term?.focus()
  showHistory.value = false
}

// ── Archive search (full scrollback, all sessions, across restarts) ──
function toggleLog() {
  showLog.value = !showLog.value
  if (!showLog.value) { logHits.value = []; logQuery.value = '' }
}
async function searchLog() {
  const q = logQuery.value.trim()
  if (!q) { logHits.value = []; return }
  logBusy.value = true
  try {
    logHits.value = (await api.terminals.searchOutput(q)).hits
  } catch { logHits.value = [] }
  logBusy.value = false
}
function fmtTime(iso: string): string {
  const d = new Date(iso)
  return isNaN(d.getTime()) ? iso : d.toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })
}

// ── AI sidebar agent ──
type Entry =
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; text: string }
  | { kind: 'output'; command: string; exit: number | null }
const entries = ref<Entry[]>([])
const input = ref('')
const busy = ref(false)
// A command the agent has typed into the live terminal, awaiting the user to
// edit/run it (press Enter in the terminal) or Skip it here.
const proposed = ref<{ id: string; command: string } | null>(null)
const sidebarEl = ref<HTMLElement>()
let agentAbort: AbortController | null = null
let asstIdx: number | null = null

function scrollSidebar() {
  nextTick(() => { if (sidebarEl.value) sidebarEl.value.scrollTop = sidebarEl.value.scrollHeight })
}

async function ask() {
  const q = input.value.trim()
  if (!q || busy.value) return
  entries.value.push({ kind: 'user', text: q })
  input.value = ''
  busy.value = true
  asstIdx = null
  scrollSidebar()
  agentAbort = new AbortController()
  try {
    await api.streamTerminalAgent(sessionId.value, q, {
      onToken: (t) => {
        if (asstIdx === null) {
          entries.value.push({ kind: 'assistant', text: '' })
          asstIdx = entries.value.length - 1
        }
        const e = entries.value[asstIdx]
        if (e.kind === 'assistant') e.text += t
        scrollSidebar()
      },
      onProposed: (id, command) => {
        // The backend already typed the command into the terminal; focus it so
        // the user can edit and press Enter right away.
        proposed.value = { id, command }
        asstIdx = null
        term?.focus()
        scrollSidebar()
      },
      onOutput: (command, exit) => { proposed.value = null; entries.value.push({ kind: 'output', command, exit }); asstIdx = null; scrollSidebar() },
      onError: (m) => { entries.value.push({ kind: 'assistant', text: '⚠ ' + m }); asstIdx = null },
      onDone: () => {},
    }, undefined, agentAbort.signal)
  } catch (e) {
    if (!(e instanceof Error && e.name === 'AbortError')) {
      entries.value.push({ kind: 'assistant', text: '⚠ ' + (e instanceof Error ? e.message : 'agent error') })
    }
  }
  busy.value = false
  proposed.value = null
}

// Reject a command the agent typed into the terminal — the backend clears the
// prompt line and tells the agent to move on.
function skip() {
  if (!proposed.value) return
  api.terminals.decide(proposed.value.id, false).catch(() => {})
  proposed.value = null
}
function stop() { agentAbort?.abort() }
</script>

<template>
  <div ref="rootEl" class="flex h-full w-full bg-[#0d0d0d] overflow-hidden">
    <!-- Terminal column -->
    <div class="flex flex-col flex-1 min-w-0 relative">
      <div class="flex items-center gap-1.5 px-2 py-1 border-b border-[var(--c-1e1e1e)] shrink-0 text-[var(--c-808080)]">
        <select
          v-model="shell"
          class="text-[0.74rem] text-fg bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-1.5 py-0.5 focus:outline-none"
        >
          <option value="bash">bash</option>
          <option value="pwsh">PowerShell</option>
        </select>
        <button class="ml-auto text-[0.72rem] px-2 py-0.5 rounded hover:bg-[var(--c-222222)] hover:text-fg" :class="showFind ? 'bg-[var(--c-222222)] text-fg' : ''" @click="toggleFind">Find</button>
        <button class="text-[0.72rem] px-2 py-0.5 rounded hover:bg-[var(--c-222222)] hover:text-fg" :class="showHistory ? 'bg-[var(--c-222222)] text-fg' : ''" @click="toggleHistory">History</button>
        <button class="text-[0.72rem] px-2 py-0.5 rounded hover:bg-[var(--c-222222)] hover:text-fg" :class="showLog ? 'bg-[var(--c-222222)] text-fg' : ''" title="Search the full scrollback archive" @click="toggleLog">Archive</button>
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

      <div v-if="showLog" class="flex flex-col gap-1.5 px-2 py-1.5 border-b border-[var(--c-1e1e1e)] shrink-0 max-h-72 overflow-y-auto">
        <input
          v-model="logQuery"
          placeholder="Search all terminal output (every session, across restarts)…"
          class="text-[0.78rem] text-fg bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-2 py-1 focus:outline-none focus:border-[var(--c-3a3a3a)]"
          @keydown.enter.prevent="searchLog"
        />
        <p v-if="logBusy" class="text-[0.72rem] text-[var(--c-585858)] px-1">Searching…</p>
        <p v-else-if="logQuery && logHits.length === 0" class="text-[0.72rem] text-[var(--c-585858)] px-1">No matches in the archive.</p>
        <div
          v-for="(h, i) in logHits"
          :key="i"
          class="flex flex-col gap-0.5 px-2 py-1 rounded bg-[var(--c-141414)] border border-[var(--c-1e1e1e)]"
        >
          <div class="flex items-center gap-2 text-[0.66rem] text-[var(--c-707070)]">
            <span class="text-[var(--c-6ecf8e)]">{{ h.shell }}</span>
            <span>{{ fmtTime(h.created_at) }}</span>
          </div>
          <pre class="font-mono text-[0.72rem] text-[var(--c-b0b0b0)] whitespace-pre-wrap break-words m-0">{{ h.snippet }}</pre>
        </div>
      </div>

      <div ref="host" class="flex-1 min-h-0 px-1.5 pt-1"></div>

      <transition
        enter-active-class="transition-opacity duration-150" enter-from-class="opacity-0"
        leave-active-class="transition-opacity duration-300" leave-to-class="opacity-0"
      >
        <div
          v-if="copied"
          class="absolute bottom-3 right-3 pointer-events-none bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-2.5 py-1 text-[0.72rem] shadow-lg"
        >Copied to clipboard</div>
      </transition>
    </div>

    <!-- Drag handle -->
    <div
      class="w-1 shrink-0 cursor-col-resize bg-[var(--c-1e1e1e)] hover:bg-[var(--c-3a3a3a)] transition-colors"
      @mousedown="startDrag"
    ></div>

    <!-- AI sidebar -->
    <div class="flex flex-col shrink-0 bg-bg" :style="{ width: sidebarWidth + 'px' }">
      <div class="px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex items-center gap-2">
        <span class="text-[0.8rem] font-medium text-fg">Terminal AI</span>
        <span v-if="busy" class="text-[0.7rem] text-[var(--c-7ab0ff)] ml-auto">working…</span>
      </div>

      <div ref="sidebarEl" class="flex-1 overflow-y-auto p-2.5 flex flex-col gap-2.5 text-[0.8rem]">
        <p v-if="entries.length === 0 && !proposed" class="text-[var(--c-707070)] leading-relaxed">
          Ask me to do something in this {{ shell === 'pwsh' ? 'PowerShell' : 'bash' }} shell. I'll type each command
          into the terminal for you to edit (or not) and run by pressing Enter, then read the output to decide the next step.
        </p>

        <template v-for="(e, i) in entries" :key="i">
          <div v-if="e.kind === 'user'" class="self-end max-w-[90%] bg-[var(--c-2a4a7a)] text-fg rounded-lg px-2.5 py-1.5 whitespace-pre-wrap break-words">{{ e.text }}</div>
          <div v-else-if="e.kind === 'assistant'" class="self-start max-w-[95%] text-[var(--c-d0d0d0)] whitespace-pre-wrap break-words">{{ e.text }}</div>
          <div v-else class="self-start max-w-[95%] font-mono text-[0.72rem] text-[var(--c-9a9a9a)] bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-2 py-1 break-words">
            <span class="text-[var(--c-6ecf8e)]">$</span> {{ e.command }}
            <span :class="e.exit === 0 ? 'text-[var(--c-585858)]' : 'text-[var(--c-df7a7a)]'"> (exit {{ e.exit ?? '?' }})</span>
          </div>
        </template>

        <div v-if="proposed" class="self-start w-full flex flex-col gap-2 bg-[var(--c-1a1610)] border border-[var(--c-4a3a1a)] rounded-lg p-2.5">
          <div class="text-[0.7rem] uppercase tracking-[0.05em] text-[var(--c-e0b060)]">Typed into the terminal — edit if needed, press Enter to run</div>
          <div class="font-mono text-[0.78rem] text-[var(--c-d4d4d4)] bg-[#0d0d0d] border border-[var(--c-2a2a2a)] rounded p-2 break-words whitespace-pre-wrap">{{ proposed.command }}</div>
          <div class="flex items-center gap-2">
            <button class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-3 py-1 text-xs cursor-pointer hover:bg-[var(--c-254a35)]" @click="term?.focus()">Go to terminal</button>
            <button class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] rounded px-3 py-1 text-xs cursor-pointer hover:bg-[var(--c-4a2525)]" @click="skip">Skip</button>
          </div>
        </div>
      </div>

      <div class="border-t border-[var(--c-1e1e1e)] p-2 shrink-0 flex gap-2">
        <input
          v-model="input"
          :placeholder="busy ? 'Working…' : 'Ask the AI to do something…'"
          :disabled="busy"
          class="flex-1 text-[0.82rem] text-fg bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-3 py-2 focus:outline-none focus:border-[var(--c-3a3a3a)]"
          @keydown.enter.prevent="ask"
        />
        <button v-if="busy" class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] rounded-md py-2 px-3 cursor-pointer text-xs" @click="stop">Stop</button>
        <button v-else class="bg-[var(--c-2a4a7a)] text-fg border-none rounded-md py-2 px-4 cursor-pointer disabled:opacity-40" :disabled="!input.trim()" @click="ask">Ask</button>
      </div>
    </div>
  </div>
</template>
