<script setup lang="ts">
import { ref, onMounted, nextTick, computed } from 'vue'
import { useSessionsStore } from '../stores/sessions'
import { useApprovalsStore } from '../stores/approvals'
import { useLogsStore } from '../stores/logs'
import { useCalendarStore } from '../stores/calendar'
import { useTasksStore } from '../stores/tasks'
import * as api from '../api'

const logs = useLogsStore()

const store = useSessionsStore()
const approvalsStore = useApprovalsStore()
const calStore = useCalendarStore()
const tasksStore = useTasksStore()

// Tool-result messages are model context, not user-facing — hide them.
const visibleMessages = computed(() => store.messages.filter(m => m.role !== 'tool'))

async function newSession() {
  const s = await store.createSession()
  await store.loadSession(s.id)
  logs.info('Chat', 'Started new session')
}

const input = ref('')
const providers = ref<api.ProviderConfig[]>([])
const provider = ref('')
const sending = ref(false)
const error = ref<string | null>(null)
const messagesEl = ref<HTMLElement>()

let abortController: AbortController | null = null

onMounted(async () => {
  const [, pRes] = await Promise.all([
    store.fetchSessions(),
    api.settings.listProviders(),
  ])
  providers.value = pRes.providers
  if (pRes.providers.length > 0) provider.value = pRes.providers[0].name

  // Keep a session set by a handoff (e.g. "Ask AI" from email); otherwise pick up
  // the most recent session, or start a fresh one.
  if (store.activeSession) {
    // already loaded by the handoff
  } else if (store.sessions.length === 0) {
    const s = await store.createSession()
    await store.loadSession(s.id)
  } else {
    await store.loadSession(store.sessions[0].id)
  }

  // Land at the latest message on (re)load. The extra frame ensures it happens
  // after the message list (incl. rendered markdown) has its final height.
  await scrollToBottom()
  requestAnimationFrame(() => {
    if (messagesEl.value) messagesEl.value.scrollTop = messagesEl.value.scrollHeight
  })
})

async function send() {
  if (!input.value.trim() || sending.value || !store.activeSession) return
  let calendarTouched = false
  let tasksTouched = false
  const text = input.value.trim()
  input.value = ''
  sending.value = true
  error.value = null

  store.appendMessage({
    id: crypto.randomUUID(),
    session_id: store.activeSession.id,
    role: 'user',
    content: text,
    created_at: new Date().toISOString(),
  })
  await scrollToBottom()

  abortController = new AbortController()
  logs.info('Chat', `Sending message via provider "${provider.value}"`)

  try {
    await api.streamChat(
      store.activeSession.id,
      text,
      provider.value,
      (tok) => { store.appendToken(tok); scrollToBottom() },
      () => {
        sending.value = false
        scrollToBottom()
        logs.info('Chat', 'Response complete')
        // If a calendar/task tool ran this turn, refresh any open window.
        if (calendarTouched) { calStore.notifyChanged(); calendarTouched = false }
        if (tasksTouched) { tasksStore.notifyChanged(); tasksTouched = false }
      },
      (actionId, toolName, toolArgs) => {
        logs.warn('Chat', `Tool approval required: ${toolName}`)
        approvalsStore.addPending({
          id: actionId,
          session_id: store.activeSession!.id,
          tool_name: toolName,
          tool_args: JSON.stringify(toolArgs),
          status: 'pending',
          created_at: new Date().toISOString(),
        })
      },
      (name) => {
        store.appendToolCall(name)
        scrollToBottom()
        if (name === 'create_calendar_event' || name === 'delete_calendar_event') calendarTouched = true
        if (name === 'create_task' || name === 'update_task' || name === 'complete_task' || name === 'delete_task') tasksTouched = true
      },
      abortController.signal,
    )
  } catch (e) {
    if (e instanceof Error && e.name !== 'AbortError') {
      error.value = e.message
      logs.error('Chat', `Stream error: ${e.message}`)
    } else {
      logs.info('Chat', 'Stream cancelled')
    }
    sending.value = false
  }
}

function cancel() {
  abortController?.abort()
  sending.value = false
}

async function scrollToBottom() {
  await nextTick()
  if (messagesEl.value) messagesEl.value.scrollTop = messagesEl.value.scrollHeight
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() }
}

// ── Minimal, XSS-safe markdown rendering for assistant messages ──────────────
function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
function inlineMd(s: string): string {
  const codes: string[] = []
  s = s.replace(/`([^`]+)`/g, (_m, c) => { codes.push(c); return `${codes.length - 1}` })
  s = s.replace(/\*\*([^*]+?)\*\*/g, '<strong>$1</strong>')
  s = s.replace(/__([^_]+?)__/g, '<strong>$1</strong>')
  s = s.replace(/(^|[^*\w])\*(?!\s)([^*\n]+?)\*/g, '$1<em>$2</em>')
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>')
  s = s.replace(/(\d+)/g, (_m, n) => `<code>${codes[+n]}</code>`)
  return s
}
function renderMarkdown(src: string): string {
  const lines = escapeHtml(src).split('\n')
  const out: string[] = []
  let i = 0
  let list: 'ul' | 'ol' | null = null
  const closeList = () => { if (list) { out.push(`</${list}>`); list = null } }
  while (i < lines.length) {
    const line = lines[i]
    if (/^\s*```/.test(line)) {
      closeList(); i++
      const code: string[] = []
      while (i < lines.length && !/^\s*```/.test(lines[i])) { code.push(lines[i]); i++ }
      i++
      out.push(`<pre class="md-pre"><code>${code.join('\n')}</code></pre>`)
      continue
    }
    const h = /^(#{1,6})\s+(.*)$/.exec(line)
    if (h) { closeList(); out.push(`<div class="md-h md-h${Math.min(h[1].length, 3)}">${inlineMd(h[2])}</div>`); i++; continue }
    const ul = /^\s*[-*+]\s+(.*)$/.exec(line)
    if (ul) { if (list !== 'ul') { closeList(); out.push('<ul class="md-ul">'); list = 'ul' } out.push(`<li>${inlineMd(ul[1])}</li>`); i++; continue }
    const ol = /^\s*\d+\.\s+(.*)$/.exec(line)
    if (ol) { if (list !== 'ol') { closeList(); out.push('<ol class="md-ol">'); list = 'ol' } out.push(`<li>${inlineMd(ol[1])}</li>`); i++; continue }
    if (/^\s*$/.test(line)) { closeList(); out.push('<br>'); i++; continue }
    closeList(); out.push(`<div>${inlineMd(line)}</div>`); i++
  }
  closeList()
  return out.join('\n')
}

// DB-stored content is JSON-encoded (a quoted string, or a multimodal object);
// live-streamed content is plain text. Normalize both for display.
function displayContent(c: string): string {
  try {
    const v = JSON.parse(c)
    if (typeof v === 'string') return v
    if (v && typeof v === 'object' && v.type === 'multimodal') return v.text ?? ''
  } catch { /* plain text */ }
  return c
}

const TOOL_LABELS: Record<string, string> = {
  create_calendar_event: 'Creating calendar event',
  list_calendar_events: 'Checking the calendar',
  delete_calendar_event: 'Deleting calendar event',
}
function toolLabel(name: string): string {
  return TOOL_LABELS[name] ?? `Using ${name}`
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="flex-1 overflow-y-auto p-4 flex flex-col gap-3" ref="messagesEl">
      <template v-for="msg in visibleMessages" :key="msg.id">
        <!-- Tool activity indicator -->
        <div v-if="msg.role === 'tool_call'" class="self-start flex items-center gap-2 text-[0.78rem] text-[#7a9ec0] py-0.5">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2-2 2.6-2.6z"/>
          </svg>
          <span>{{ toolLabel(msg.content) }}…</span>
        </div>
        <!-- Normal message -->
        <div v-else :class="['flex flex-col gap-1 max-w-3xl', msg.role === 'user' ? 'self-end' : 'self-start']">
          <span class="text-[0.7rem] uppercase text-[#606060]">{{ msg.role }}</span>
          <!-- Assistant: rendered markdown. User: plain text. -->
          <div
            v-if="msg.role === 'assistant'"
            class="md-body bg-surface py-[0.6rem] px-[0.8rem] rounded-lg text-[0.9rem] leading-[1.5] break-words"
            v-html="renderMarkdown(displayContent(msg.content))"
          />
          <pre
            v-else
            :class="['whitespace-pre-wrap font-[inherit] text-[0.9rem]', msg.role === 'user' ? 'bg-[#1e2a3a] py-[0.6rem] px-[0.8rem] rounded-lg' : '']"
          >{{ displayContent(msg.content) }}</pre>
        </div>
      </template>
    </div>
    <div v-if="error" class="bg-[#5a2a2a] text-[#e0c0c0] text-[0.8rem] py-[0.4rem] px-4">{{ error }}</div>
    <div class="flex gap-2 p-3 border-t border-raised items-end">
      <button class="bg-surface text-[#5a8adf] border border-raised rounded-md py-2 px-[0.6rem] cursor-pointer flex items-center flex-shrink-0 transition-colors duration-100 hover:not-disabled:bg-[#222] hover:not-disabled:text-[#7ab0ff] disabled:opacity-40" title="New chat" :disabled="sending" @click="newSession">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
        </svg>
      </button>
      <select
        v-if="providers.length"
        v-model="provider"
        class="bg-surface text-[inherit] border border-raised rounded-md p-2 text-[0.8rem] cursor-pointer"
        :disabled="sending"
      >
        <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
      </select>
      <span v-else class="text-[0.75rem] text-[#585858] whitespace-nowrap">No providers configured</span>
      <textarea
        v-model="input"
        @keydown="onKeydown"
        placeholder="Message… (Enter to send, Shift+Enter for newline)"
        rows="3"
        :disabled="sending"
        class="flex-1 bg-surface text-[inherit] border border-raised rounded-md p-2 font-[inherit] text-[0.9rem] resize-none"
      />
      <div class="flex flex-col gap-1">
        <button @click="send" :disabled="sending || !input.trim()" class="bg-[#2a4a7a] text-fg border-none rounded-md py-2 px-4 cursor-pointer whitespace-nowrap disabled:opacity-40 disabled:cursor-not-allowed">Send</button>
        <button v-if="sending" @click="cancel" class="bg-[#5a2a2a] text-fg border-none rounded-md py-2 px-4 cursor-pointer whitespace-nowrap disabled:opacity-40 disabled:cursor-not-allowed">Stop</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Markdown rendered into assistant messages (v-html → needs :deep). */
.md-body :deep(strong) { font-weight: 600; color: #e8e8e8; }
.md-body :deep(em) { font-style: italic; }
.md-body :deep(a) { color: #7ab0ff; text-decoration: underline; }
.md-body :deep(code) {
  background: #0f0f0f; border: 1px solid #262626; border-radius: 4px;
  padding: 0.05rem 0.3rem; font-size: 0.85em;
  font-family: ui-monospace, 'Cascadia Code', monospace;
}
.md-body :deep(pre.md-pre) {
  background: #0d0d0d; border: 1px solid #1e1e1e; border-radius: 6px;
  padding: 0.6rem 0.8rem; overflow-x: auto; margin: 0.4rem 0;
}
.md-body :deep(pre.md-pre code) { background: none; border: none; padding: 0; }
.md-body :deep(ul.md-ul), .md-body :deep(ol.md-ol) { margin: 0.3rem 0; padding-left: 1.3rem; }
.md-body :deep(ul.md-ul) { list-style: disc; }
.md-body :deep(ol.md-ol) { list-style: decimal; }
.md-body :deep(li) { margin: 0.1rem 0; }
.md-body :deep(.md-h) { font-weight: 600; color: #e8e8e8; margin: 0.35rem 0 0.15rem; }
.md-body :deep(.md-h1) { font-size: 1.12em; }
.md-body :deep(.md-h2) { font-size: 1.06em; }
/* Collapse the leading/trailing spacer <br>s the renderer can emit. */
.md-body :deep(br:first-child), .md-body :deep(br:last-child) { display: none; }
</style>
