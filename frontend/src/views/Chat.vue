<script setup lang="ts">
import { ref, onMounted, nextTick, computed, watch } from 'vue'
import { useSessionsStore } from '../stores/sessions'
import { useApprovalsStore } from '../stores/approvals'
import { useLogsStore } from '../stores/logs'
import { useCalendarStore } from '../stores/calendar'
import { useTasksStore } from '../stores/tasks'
import { useNotesStore } from '../stores/notes'
import * as api from '../api'
import { renderMarkdown } from '../lib/markdown'

const logs = useLogsStore()

const store = useSessionsStore()
const approvalsStore = useApprovalsStore()
const calStore = useCalendarStore()
const tasksStore = useTasksStore()
const notesStore = useNotesStore()

// Tool-call/result messages are model context, not user-facing — hide them.
const visibleMessages = computed(() =>
  store.messages.filter(m => m.role !== 'tool'),
)

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
  // Restore the last-used provider if it still exists; else first in list.
  const saved = localStorage.getItem('chat-provider')
  if (saved && pRes.providers.some(p => p.name === saved)) {
    provider.value = saved
  } else if (pRes.providers.length > 0) {
    provider.value = pRes.providers[0].name
  }
  watch(provider, (v) => { if (v) localStorage.setItem('chat-provider', v) })

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

// Messages typed while a reply is streaming wait here and auto-send in
// order when the turn finishes.
interface QueuedMessage { text: string; images: api.ChatImage[] }
const queued = ref<QueuedMessage[]>([])

watch(sending, (now) => {
  if (!now && queued.value.length > 0) {
    const next = queued.value.shift()!
    // Next tick so the finished turn's UI settles before the new one starts.
    nextTick(() => sendText(next.text, next.images))
  }
})

// ── Image attachments (paste / drop) ─────────────────────────────────────────
const MAX_IMAGES = 4
const pendingImages = ref<api.ChatImage[]>([])

function addImageFiles(files: File[]) {
  for (const file of files) {
    if (!file.type.startsWith('image/')) continue
    if (pendingImages.value.length >= MAX_IMAGES) {
      error.value = `At most ${MAX_IMAGES} images per message`
      break
    }
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result as string
      pendingImages.value.push({ mime: file.type, b64: result.slice(result.indexOf(',') + 1) })
    }
    reader.readAsDataURL(file)
  }
}

function onPaste(e: ClipboardEvent) {
  const files = Array.from(e.clipboardData?.items ?? [])
    .filter(i => i.kind === 'file' && i.type.startsWith('image/'))
    .map(i => i.getAsFile())
    .filter((f): f is File => f !== null)
  if (files.length > 0) {
    e.preventDefault()
    addImageFiles(files)
  }
}

function onDrop(e: DragEvent) {
  if (e.dataTransfer?.files?.length) {
    e.preventDefault()
    addImageFiles(Array.from(e.dataTransfer.files))
  }
}

async function send() {
  const text = input.value.trim()
  const images = pendingImages.value
  if ((!text && images.length === 0) || !store.activeSession) return
  input.value = ''
  pendingImages.value = []
  if (sending.value) {
    queued.value.push({ text, images })
    return
  }
  await sendText(text, images)
}

async function sendText(text: string, images: api.ChatImage[] = []) {
  if (sending.value || !store.activeSession) return
  let calendarTouched = false
  let tasksTouched = false
  let notesTouched = false
  sending.value = true
  error.value = null

  store.appendMessage({
    id: crypto.randomUUID(),
    session_id: store.activeSession.id,
    role: 'user',
    // Mirror the DB shape so displayContent/displayImages handle both live
    // and reloaded messages identically.
    content: images.length
      ? JSON.stringify({ type: 'multimodal', text, images })
      : text,
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
        if (notesTouched) { notesStore.notifyChanged(); notesTouched = false }
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
        scrollToBottom()
      },
      (name) => {
        store.appendToolCall(name)
        scrollToBottom()
        if (name === 'create_calendar_event' || name === 'delete_calendar_event') calendarTouched = true
        if (name === 'create_task' || name === 'update_task' || name === 'complete_task' || name === 'delete_task') tasksTouched = true
        if (name === 'create_note' || name === 'update_note' || name === 'delete_note') notesTouched = true
      },
      abortController.signal,
      images,
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

/// Data URIs for a multimodal user message's images (empty for plain text).
function displayImages(c: string): string[] {
  try {
    const v = JSON.parse(c)
    if (v && typeof v === 'object' && v.type === 'multimodal' && Array.isArray(v.images)) {
      return v.images.map((i: api.ChatImage) => `data:${i.mime};base64,${i.b64}`)
    }
  } catch { /* plain text */ }
  return []
}

const TOOL_LABELS: Record<string, string> = {
  create_calendar_event: 'Creating calendar event',
  list_calendar_events: 'Checking the calendar',
  delete_calendar_event: 'Deleting calendar event',
  list_tasks: 'Checking the to-do list',
  create_task: 'Adding task',
  update_task: 'Updating task',
  complete_task: 'Completing task',
  delete_task: 'Deleting task',
  list_notes: 'Searching notes',
  get_note: 'Reading note',
  create_note: 'Saving note',
  update_note: 'Updating note',
  delete_note: 'Deleting note',
}
function toolLabel(name: string): string {
  return TOOL_LABELS[name] ?? `Using ${name}`
}

// Pending approvals for the active session — rendered as inline cards.
const sessionApprovals = computed(() =>
  approvalsStore.pending.filter(a => a.session_id === store.activeSession?.id),
)

function prettyArgs(args: string): string {
  try {
    return JSON.stringify(JSON.parse(args), null, 2)
  } catch {
    return args
  }
}

// A tool_call message is either a live chip (content = tool name) or a
// DB-persisted row (JSON-encoded array of call objects). Label both.
function toolCallLabels(content: string): string[] {
  try {
    const v = JSON.parse(content)
    if (Array.isArray(v)) {
      return v.map((c) => toolLabel(typeof c?.fn_name === 'string' ? c.fn_name : ''))
    }
  } catch { /* live chip: plain tool name */ }
  return [toolLabel(content)]
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="flex-1 overflow-y-auto p-4 flex flex-col gap-3" ref="messagesEl">
      <template v-for="msg in visibleMessages" :key="msg.id">
        <!-- Tool activity indicator -->
        <div v-if="msg.role === 'tool_call'" class="self-start flex items-center gap-2 text-[0.78rem] text-[var(--c-7a9ec0)] py-0.5">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2-2 2.6-2.6z"/>
          </svg>
          <span>{{ toolCallLabels(msg.content).join(', ') }}…</span>
        </div>
        <!-- Normal message -->
        <div v-else :class="['flex flex-col gap-1 max-w-3xl', msg.role === 'user' ? 'self-end' : 'self-start']">
          <span class="text-[0.7rem] uppercase text-[var(--c-606060)]">{{ msg.role }}</span>
          <!-- Assistant: rendered markdown. User: plain text. -->
          <div
            v-if="msg.role === 'assistant'"
            class="md-body bg-surface py-[0.6rem] px-[0.8rem] rounded-lg text-[0.9rem] leading-[1.5] break-words"
            v-html="renderMarkdown(displayContent(msg.content))"
          />
          <div v-else :class="['flex flex-col gap-1.5', msg.role === 'user' ? 'bg-[var(--c-1e2a3a)] py-[0.6rem] px-[0.8rem] rounded-lg' : '']">
            <div v-if="displayImages(msg.content).length" class="flex flex-wrap gap-1.5">
              <img
                v-for="(src, i) in displayImages(msg.content)"
                :key="i"
                :src="src"
                class="max-w-[14rem] max-h-[14rem] rounded border border-[var(--c-2a3a4e)] object-contain"
              />
            </div>
            <pre v-if="displayContent(msg.content)" class="whitespace-pre-wrap font-[inherit] text-[0.9rem]">{{ displayContent(msg.content) }}</pre>
          </div>
        </div>
      </template>

      <!-- Inline approval cards: the turn is paused until decided. -->
      <div
        v-for="a in sessionApprovals"
        :key="a.id"
        class="self-start flex flex-col gap-2 max-w-3xl bg-[var(--c-1a1610)] border border-[var(--c-4a3a1a)] rounded-lg py-2.5 px-3"
      >
        <div class="flex items-center gap-2 text-[0.8rem] text-[var(--c-e0b060)]">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
          </svg>
          <span class="font-medium">{{ toolLabel(a.tool_name) }} — approval required</span>
        </div>
        <pre class="text-[0.72rem] text-[var(--c-a09070)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-2 overflow-x-auto whitespace-pre-wrap max-h-40">{{ prettyArgs(a.tool_args) }}</pre>
        <div class="flex items-center gap-2">
          <button class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254a35)]" @click="approvalsStore.approve(a.id)">Approve</button>
          <button class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-4a2525)]" @click="approvalsStore.reject(a.id)">Deny</button>
        </div>
      </div>
    </div>
    <div v-if="error" class="bg-[var(--c-5a2a2a)] text-[var(--c-e0c0c0)] text-[0.8rem] py-[0.4rem] px-4">{{ error }}</div>
    <!-- Messages queued while the model responds -->
    <div v-if="queued.length" class="flex flex-wrap gap-1.5 px-3 pt-2 items-center">
      <span class="text-[0.7rem] text-[var(--c-585858)] uppercase tracking-[0.05em]">Queued</span>
      <span
        v-for="(q, i) in queued"
        :key="i"
        class="inline-flex items-center gap-1.5 bg-[var(--c-16202e)] border border-[var(--c-24344a)] text-[var(--c-9ab4d4)] rounded-full px-2.5 py-[0.2rem] text-[0.75rem] max-w-[18rem]"
      >
        <span class="truncate">{{ q.text || '(image)' }}{{ q.images.length ? ` 🖼×${q.images.length}` : '' }}</span>
        <button class="bg-none border-none cursor-pointer text-[var(--c-587098)] hover:text-[var(--c-9ab4d4)] p-0 leading-none" title="Remove from queue" @click="queued.splice(i, 1)">✕</button>
      </span>
    </div>
    <!-- Pending image attachments -->
    <div v-if="pendingImages.length" class="flex flex-wrap gap-2 px-3 pt-2 items-center">
      <div v-for="(img, i) in pendingImages" :key="i" class="relative group">
        <img :src="`data:${img.mime};base64,${img.b64}`" class="w-14 h-14 object-cover rounded border border-raised" />
        <button
          class="absolute -top-1.5 -right-1.5 w-4 h-4 flex items-center justify-center rounded-full bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] text-[0.6rem] leading-none cursor-pointer opacity-0 group-hover:opacity-100 transition-opacity"
          title="Remove image"
          @click="pendingImages.splice(i, 1)"
        >✕</button>
      </div>
      <span class="text-[0.7rem] text-[var(--c-585858)]">attached — sent with your next message</span>
    </div>
    <div class="flex gap-2 p-3 border-t border-raised items-end" @drop="onDrop" @dragover.prevent>
      <button class="bg-surface text-[var(--c-5a8adf)] border border-raised rounded-md py-2 px-[0.6rem] cursor-pointer flex items-center flex-shrink-0 transition-colors duration-100 hover:not-disabled:bg-[var(--c-222222)] hover:not-disabled:text-[var(--c-7ab0ff)] disabled:opacity-40" title="New chat" :disabled="sending" @click="newSession">
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
      <span v-else class="text-[0.75rem] text-[var(--c-585858)] whitespace-nowrap">No providers configured</span>
      <!-- Stays enabled while a reply streams so the next message can be
           drafted; send() ignores Enter until the turn finishes. -->
      <textarea
        v-model="input"
        @keydown="onKeydown"
        @paste="onPaste"
        :placeholder="sending ? 'Type away — Enter queues it for when the reply finishes…' : 'Message… (Enter to send, Shift+Enter for newline, paste images)'"
        rows="3"
        class="flex-1 bg-surface text-[inherit] border border-raised rounded-md p-2 font-[inherit] text-[0.9rem] resize-none"
      />
      <div class="flex flex-col gap-1">
        <button @click="send" :disabled="sending || (!input.trim() && pendingImages.length === 0)" class="bg-[var(--c-2a4a7a)] text-fg border-none rounded-md py-2 px-4 cursor-pointer whitespace-nowrap disabled:opacity-40 disabled:cursor-not-allowed">Send</button>
        <button v-if="sending" @click="cancel" class="bg-[var(--c-5a2a2a)] text-fg border-none rounded-md py-2 px-4 cursor-pointer whitespace-nowrap disabled:opacity-40 disabled:cursor-not-allowed">Stop</button>
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
