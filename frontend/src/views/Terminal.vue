<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import '@xterm/xterm/css/xterm.css'
import * as api from '../api'

type Shell = 'bash' | 'pwsh'
const shell = ref<Shell>('bash')
const sessionId = ref('')

const host = ref<HTMLElement>()
let term: Terminal | null = null
let fit: FitAddon | null = null
let search: SearchAddon | null = null
let ws: WebSocket | null = null
let ro: ResizeObserver | null = null
const enc = new TextEncoder()

// Terminal toolbar state
const showFind = ref(false)
const findQuery = ref('')
const showHistory = ref(false)
const historyQuery = ref('')
const historyItems = ref<api.TerminalHistoryEntry[]>([])

function sendResize() {
  if (!term || ws?.readyState !== WebSocket.OPEN) return
  ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }))
}

function teardown() {
  ro?.disconnect(); ro = null
  ws?.close(); ws = null
  term?.dispose(); term = null
  fit = null; search = null
}

function connect() {
  teardown()
  if (!host.value) return
  const sid = crypto.randomUUID()
  sessionId.value = sid

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

  // Swallow all OSC 633 markers (history E, agent C/D); record commands for history.
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

  ro = new ResizeObserver(() => {
    try { fit?.fit(); sendResize() } catch {}
  })
  ro.observe(host.value)
  term.focus()
}

onMounted(connect)
watch(shell, connect)
onBeforeUnmount(() => { teardown(); agentAbort?.abort() })

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

// ── AI sidebar agent ──
type Entry =
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; text: string }
  | { kind: 'output'; command: string; exit: number | null }
const entries = ref<Entry[]>([])
const input = ref('')
const busy = ref(false)
const approval = ref<{ id: string; command: string } | null>(null)
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
      onApprove: (id, command) => { approval.value = { id, command }; asstIdx = null; scrollSidebar() },
      onOutput: (command, exit) => { entries.value.push({ kind: 'output', command, exit }); asstIdx = null; scrollSidebar() },
      onError: (m) => { entries.value.push({ kind: 'assistant', text: '⚠ ' + m }); asstIdx = null },
      onDone: () => {},
    }, undefined, agentAbort.signal)
  } catch (e) {
    if (!(e instanceof Error && e.name === 'AbortError')) {
      entries.value.push({ kind: 'assistant', text: '⚠ ' + (e instanceof Error ? e.message : 'agent error') })
    }
  }
  busy.value = false
  approval.value = null
}

function approve() {
  if (!approval.value) return
  api.terminals.decide(approval.value.id, true, approval.value.command).catch(() => {})
  approval.value = null
}
function deny() {
  if (!approval.value) return
  api.terminals.decide(approval.value.id, false).catch(() => {})
  approval.value = null
}
function stop() { agentAbort?.abort() }
</script>

<template>
  <div class="flex h-full bg-[#0d0d0d] overflow-hidden">
    <!-- Terminal column -->
    <div class="flex flex-col flex-1 min-w-0">
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

    <!-- AI sidebar -->
    <div class="flex flex-col w-[340px] shrink-0 border-l border-[var(--c-1e1e1e)] bg-bg">
      <div class="px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex items-center gap-2">
        <span class="text-[0.8rem] font-medium text-fg">Terminal AI</span>
        <span v-if="busy" class="text-[0.7rem] text-[var(--c-7ab0ff)] ml-auto">working…</span>
      </div>

      <div ref="sidebarEl" class="flex-1 overflow-y-auto p-2.5 flex flex-col gap-2.5 text-[0.8rem]">
        <p v-if="entries.length === 0 && !approval" class="text-[var(--c-707070)] leading-relaxed">
          Ask me to do something in this {{ shell === 'pwsh' ? 'PowerShell' : 'bash' }} shell. I'll run commands here
          (you approve each one) and read the output to decide the next step.
        </p>

        <template v-for="(e, i) in entries" :key="i">
          <div v-if="e.kind === 'user'" class="self-end max-w-[90%] bg-[var(--c-2a4a7a)] text-fg rounded-lg px-2.5 py-1.5 whitespace-pre-wrap break-words">{{ e.text }}</div>
          <div v-else-if="e.kind === 'assistant'" class="self-start max-w-[95%] text-[var(--c-d0d0d0)] whitespace-pre-wrap break-words">{{ e.text }}</div>
          <div v-else class="self-start max-w-[95%] font-mono text-[0.72rem] text-[var(--c-9a9a9a)] bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-2 py-1 break-words">
            <span class="text-[var(--c-6ecf8e)]">$</span> {{ e.command }}
            <span :class="e.exit === 0 ? 'text-[var(--c-585858)]' : 'text-[var(--c-df7a7a)]'"> (exit {{ e.exit ?? '?' }})</span>
          </div>
        </template>

        <!-- Pending command approval -->
        <div v-if="approval" class="self-start w-full flex flex-col gap-2 bg-[var(--c-1a1610)] border border-[var(--c-4a3a1a)] rounded-lg p-2.5">
          <div class="text-[0.7rem] uppercase tracking-[0.05em] text-[var(--c-e0b060)]">Run this command?</div>
          <textarea
            v-model="approval.command"
            rows="2"
            class="font-mono text-[0.78rem] text-[var(--c-d4d4d4)] bg-[#0d0d0d] border border-[var(--c-2a2a2a)] rounded p-2 resize-y focus:outline-none focus:border-[var(--c-3a3a3a)]"
          />
          <div class="flex items-center gap-2">
            <button class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-3 py-1 text-xs cursor-pointer hover:bg-[var(--c-254a35)]" @click="approve">Approve &amp; run</button>
            <button class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] rounded px-3 py-1 text-xs cursor-pointer hover:bg-[var(--c-4a2525)]" @click="deny">Deny</button>
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
