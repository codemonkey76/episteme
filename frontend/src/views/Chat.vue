<script setup lang="ts">
import { ref, onMounted, nextTick } from 'vue'
import { useSessionsStore } from '../stores/sessions'
import { useApprovalsStore } from '../stores/approvals'
import { useLogsStore } from '../stores/logs'
import * as api from '../api'

const logs = useLogsStore()

const store = useSessionsStore()
const approvalsStore = useApprovalsStore()

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

  if (store.sessions.length === 0) {
    const s = await store.createSession()
    await store.loadSession(s.id)
  } else {
    await store.loadSession(store.sessions[0].id)
  }
})

async function send() {
  if (!input.value.trim() || sending.value || !store.activeSession) return
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
      () => { sending.value = false; scrollToBottom(); logs.info('Chat', 'Response complete') },
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
</script>

<template>
  <div class="chat">
    <div class="messages" ref="messagesEl">
      <div
        v-for="msg in store.messages"
        :key="msg.id"
        :class="['message', msg.role]"
      >
        <span class="role">{{ msg.role }}</span>
        <pre class="content">{{ msg.content }}</pre>
      </div>
    </div>
    <div v-if="error" class="error-bar">{{ error }}</div>
    <div class="input-bar">
      <button class="new-btn" title="New chat" :disabled="sending" @click="newSession">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
        </svg>
      </button>
      <select
        v-if="providers.length"
        v-model="provider"
        class="provider-select"
        :disabled="sending"
      >
        <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
      </select>
      <span v-else class="no-provider">No providers configured</span>
      <textarea
        v-model="input"
        @keydown="onKeydown"
        placeholder="Message… (Enter to send, Shift+Enter for newline)"
        rows="3"
        :disabled="sending"
      />
      <div class="btn-group">
        <button @click="send" :disabled="sending || !input.trim()">Send</button>
        <button v-if="sending" @click="cancel" class="cancel">Stop</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat { display: flex; flex-direction: column; height: 100%; }
.messages { flex: 1; overflow-y: auto; padding: 1rem; display: flex; flex-direction: column; gap: 0.75rem; }
.message { display: flex; flex-direction: column; gap: 0.25rem; max-width: 48rem; }
.message.user { align-self: flex-end; }
.message.assistant, .message.tool { align-self: flex-start; }
.role { font-size: 0.7rem; text-transform: uppercase; color: #606060; }
.content { white-space: pre-wrap; font-family: inherit; font-size: 0.9rem; }
.message.user .content { background: #1e2a3a; padding: 0.6rem 0.8rem; border-radius: 0.5rem; }
.message.assistant .content { background: #1a1a1a; padding: 0.6rem 0.8rem; border-radius: 0.5rem; }
.error-bar { background: #5a2a2a; color: #e0c0c0; font-size: 0.8rem; padding: 0.4rem 1rem; }
.input-bar { display: flex; gap: 0.5rem; padding: 0.75rem; border-top: 1px solid #2a2a2a; align-items: flex-end; }
.new-btn { background: #1a1a1a; color: #5a8adf; border: 1px solid #2a2a2a; border-radius: 0.375rem; padding: 0.5rem 0.6rem; cursor: pointer; display: flex; align-items: center; flex-shrink: 0; transition: background 0.1s; }
.new-btn:hover:not(:disabled) { background: #222; color: #7ab0ff; }
.new-btn:disabled { opacity: 0.4; }
.provider-select { background: #1a1a1a; color: inherit; border: 1px solid #2a2a2a; border-radius: 0.375rem; padding: 0.5rem; font-size: 0.8rem; cursor: pointer; }
.no-provider { font-size: 0.75rem; color: #585858; white-space: nowrap; }
textarea { flex: 1; background: #1a1a1a; color: inherit; border: 1px solid #2a2a2a; border-radius: 0.375rem; padding: 0.5rem; font-family: inherit; font-size: 0.9rem; resize: none; }
.btn-group { display: flex; flex-direction: column; gap: 0.25rem; }
button { background: #2a4a7a; color: #e0e0e0; border: none; border-radius: 0.375rem; padding: 0.5rem 1rem; cursor: pointer; white-space: nowrap; }
button:disabled { opacity: 0.4; cursor: not-allowed; }
button.cancel { background: #5a2a2a; }
</style>
