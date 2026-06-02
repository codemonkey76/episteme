<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import * as api from '../api'

// ── Connection state ──────────────────────────────────────────────────────────
const connected = ref(false)
const checkingConnection = ref(true)

onMounted(async () => {
  try {
    const cfg = await api.integrations.email.getConfig()
    connected.value = cfg.connected
  } finally {
    checkingConnection.value = false
  }
  if (connected.value) await loadFolders()
})

// ── Folders ───────────────────────────────────────────────────────────────────
const FOLDER_ORDER = ['Inbox', 'Drafts', 'Sent Items', 'Deleted Items', 'Junk Email']

const folders = ref<api.MailFolder[]>([])
const selectedFolder = ref<api.MailFolder | null>(null)
const loadingFolders = ref(false)
const folderError = ref('')

async function loadFolders() {
  loadingFolders.value = true
  folderError.value = ''
  try {
    const res = await api.email.listFolders()
    folders.value = [...res.value].sort((a, b) => {
      const ai = FOLDER_ORDER.indexOf(a.displayName)
      const bi = FOLDER_ORDER.indexOf(b.displayName)
      if (ai === -1 && bi === -1) return a.displayName.localeCompare(b.displayName)
      if (ai === -1) return 1
      if (bi === -1) return -1
      return ai - bi
    })
    const inbox = folders.value.find(f => f.displayName === 'Inbox') ?? folders.value[0] ?? null
    if (inbox) selectFolder(inbox)
  } catch (e: unknown) {
    folderError.value = e instanceof Error ? e.message : 'Failed to load folders'
  } finally {
    loadingFolders.value = false
  }
}

async function selectFolder(folder: api.MailFolder) {
  selectedFolder.value = folder
  selectedMessage.value = null
  view.value = 'none'
  searchQuery.value = ''
  searchResults.value = []
  searchNextLink.value = null
  await loadMessages(folder.id)
}

// ── Messages ──────────────────────────────────────────────────────────────────
const messages = ref<api.MessageSummary[]>([])
const loadingMessages = ref(false)
const messagesSkip = ref(0)
const messagesHasMore = ref(false)
const PAGE = 30

const messagesError = ref('')

async function loadMessages(folderId: string, skip = 0) {
  loadingMessages.value = true
  messagesError.value = ''
  try {
    const res = await api.email.listMessages(folderId, skip, PAGE)
    if (skip === 0) {
      messages.value = res.value
    } else {
      messages.value.push(...res.value)
    }
    messagesSkip.value = skip + res.value.length
    messagesHasMore.value = res.value.length === PAGE
  } catch (e: unknown) {
    messagesError.value = e instanceof Error ? e.message : 'Failed to load messages'
  } finally {
    loadingMessages.value = false
  }
}

async function loadMore() {
  if (isSearching.value) {
    await runSearch(searchQuery.value.trim(), searchNextLink.value)
  } else if (selectedFolder.value) {
    await loadMessages(selectedFolder.value.id, messagesSkip.value)
  }
}

// ── Message detail ────────────────────────────────────────────────────────────
type View = 'none' | 'message' | 'compose'

const view = ref<View>('none')
const selectedMessage = ref<api.MessageDetail | null>(null)
const loadingMessage = ref(false)
const showReply = ref(false)
const iframeEl = ref<HTMLIFrameElement | null>(null)

async function selectMessage(summary: api.MessageSummary) {
  showReply.value = false
  view.value = 'message'
  loadingMessage.value = true
  try {
    const detail = await api.email.getMessage(summary.id)
    selectedMessage.value = detail
    if (!summary.isRead) {
      api.email.markRead(summary.id)
      const idx = messages.value.findIndex(m => m.id === summary.id)
      if (idx !== -1) {
        messages.value[idx] = { ...messages.value[idx], isRead: true }
        const folder = folders.value.find(f => f.id === selectedFolder.value?.id)
        if (folder && folder.unreadItemCount > 0) folder.unreadItemCount--
      }
    }
  } finally {
    loadingMessage.value = false
  }
}

function onIframeLoad() {
  if (!iframeEl.value) return
  try {
    const doc = iframeEl.value.contentDocument
    if (doc) iframeEl.value.style.height = doc.body.scrollHeight + 32 + 'px'
  } catch {}
}

watch(selectedMessage, () => {
  nextTick(() => {
    if (iframeEl.value) {
      iframeEl.value.style.height = '400px'
    }
  })
})

// ── Compose / reply ───────────────────────────────────────────────────────────
const composeForm = ref({ to: '', subject: '', body: '' })
const sending = ref(false)
const sendMsg = ref('')

function openCompose() {
  selectedMessage.value = null
  showReply.value = false
  view.value = 'compose'
  composeForm.value = { to: '', subject: '', body: '' }
  sendMsg.value = ''
}

function openReply() {
  if (!selectedMessage.value) return
  showReply.value = true
  const from = selectedMessage.value.from.emailAddress.address
  composeForm.value = {
    to: from,
    subject: selectedMessage.value.subject?.startsWith('Re:')
      ? (selectedMessage.value.subject ?? '')
      : `Re: ${selectedMessage.value.subject ?? ''}`,
    body: '',
  }
  sendMsg.value = ''
}

async function sendEmail() {
  sending.value = true
  sendMsg.value = ''
  try {
    const payload: api.SendEmailPayload = {
      to: composeForm.value.to.split(',').map(s => s.trim()).filter(Boolean),
      body: composeForm.value.body,
    }
    if (showReply.value && selectedMessage.value) {
      payload.reply_to_message_id = selectedMessage.value.id
    } else {
      payload.subject = composeForm.value.subject
    }
    const res = await api.email.send(payload)
    if (!res.ok) throw new Error(`${res.status}`)
    sendMsg.value = 'Sent.'
    if (!showReply.value) {
      view.value = 'none'
    } else {
      showReply.value = false
    }
    composeForm.value = { to: '', subject: '', body: '' }
  } catch (e: unknown) {
    sendMsg.value = e instanceof Error ? `Failed: ${e.message}` : 'Send failed.'
  } finally {
    sending.value = false
  }
}

// ── Pane resizing ─────────────────────────────────────────────────────────────
const folderPaneWidth = ref(168)
const listPaneWidth = ref(290)

type DividerTarget = 'folder' | 'list'
let activeDivider: DividerTarget | null = null
let dragStartX = 0
let dragStartWidth = 0

function onDividerPointerdown(target: DividerTarget, e: PointerEvent) {
  activeDivider = target
  dragStartX = e.clientX
  dragStartWidth = target === 'folder' ? folderPaneWidth.value : listPaneWidth.value
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
}

function onDividerPointermove(e: PointerEvent) {
  if (!activeDivider) return
  const delta = e.clientX - dragStartX
  if (activeDivider === 'folder') {
    folderPaneWidth.value = Math.max(120, Math.min(300, dragStartWidth + delta))
  } else {
    listPaneWidth.value = Math.max(200, Math.min(500, dragStartWidth + delta))
  }
}

function onDividerPointerup() {
  activeDivider = null
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
}

onUnmounted(() => {
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
})

// ── Search ────────────────────────────────────────────────────────────────────
const searchQuery = ref('')
const searchResults = ref<api.MessageSummary[]>([])
const searchNextLink = ref<string | null>(null)
const searchLoading = ref(false)
const searchError = ref('')

const isSearching = computed(() => searchQuery.value.trim().length > 0)
const displayedMessages = computed(() => isSearching.value ? searchResults.value : messages.value)
const displayedHasMore = computed(() => isSearching.value ? searchNextLink.value !== null : messagesHasMore.value)
const displayedLoading = computed(() => isSearching.value ? searchLoading.value : loadingMessages.value)

let searchTimer: ReturnType<typeof setTimeout> | null = null

watch(searchQuery, (q) => {
  if (searchTimer) clearTimeout(searchTimer)
  if (!q.trim()) {
    searchResults.value = []
    searchNextLink.value = null
    searchError.value = ''
    return
  }
  searchTimer = setTimeout(() => runSearch(q.trim()), 350)
})

async function runSearch(q: string, nextLink?: string | null) {
  searchLoading.value = true
  searchError.value = ''
  try {
    const res = await api.email.search(q, nextLink)
    if (nextLink) {
      searchResults.value.push(...res.value)
    } else {
      searchResults.value = res.value
    }
    searchNextLink.value = res.next_link
  } catch (e: unknown) {
    searchError.value = e instanceof Error ? e.message : 'Search failed'
  } finally {
    searchLoading.value = false
  }
}

// ── Utilities ─────────────────────────────────────────────────────────────────
function formatDate(iso: string): string {
  const d = new Date(iso)
  const now = new Date()
  const diffMs = now.getTime() - d.getTime()
  const diffDays = Math.floor(diffMs / 86400000)
  if (diffDays === 0) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  if (diffDays < 7) return d.toLocaleDateString([], { weekday: 'short' })
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' })
}

function displayName(ea: api.GraphEmailAddress): string {
  return ea.name || ea.address
}

const replyBody = computed(() => {
  const m = selectedMessage.value
  if (!m) return ''
  const date = new Date(m.receivedDateTime).toLocaleString()
  const from = `${m.from.emailAddress.name} <${m.from.emailAddress.address}>`
  return `\n\n— On ${date}, ${from} wrote:\n`
})
</script>

<template>
  <!-- Loading / not connected -->
  <div v-if="checkingConnection" class="center-state">
    <span class="spinner" />
  </div>

  <div v-else-if="!connected" class="center-state">
    <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="state-icon">
      <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/>
      <polyline points="22,6 12,13 2,6"/>
    </svg>
    <p class="state-title">No email account connected</p>
    <p class="state-sub">Connect your Microsoft 365 account in Settings → Integrations.</p>
  </div>

  <!-- 3-pane layout -->
  <div v-else class="email-layout">

    <!-- Left: folder list -->
    <aside class="folder-pane" :style="{ width: folderPaneWidth + 'px' }">
      <button class="btn-compose" @click="openCompose">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
        </svg>
        Compose
      </button>
      <div v-if="folderError" class="folder-error" :title="folderError">⚠ Load failed</div>

      <nav class="folder-list">
        <button
          v-for="f in folders"
          :key="f.id"
          :class="['folder-item', { active: selectedFolder?.id === f.id }]"
          @click="selectFolder(f)"
        >
          <span class="folder-name">{{ f.displayName }}</span>
          <span v-if="f.unreadItemCount > 0" class="unread-badge">{{ f.unreadItemCount }}</span>
        </button>
      </nav>
    </aside>

    <!-- Divider: folder / list -->
    <div
      class="pane-divider"
      @pointerdown="onDividerPointerdown('folder', $event)"
      @pointermove="onDividerPointermove"
      @pointerup="onDividerPointerup"
      @pointercancel="onDividerPointerup"
    />

    <!-- Middle: message list -->
    <div class="list-pane" :style="{ width: listPaneWidth + 'px' }">
      <div class="list-header">
        <span class="list-title">{{ selectedFolder?.displayName ?? '' }}</span>
        <div class="search-wrap">
          <svg class="search-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
          <input
            v-model="searchQuery"
            class="search-input"
            placeholder="Search"
            autocomplete="off"
          />
          <button v-if="searchQuery" class="search-clear" @click="searchQuery = ''">✕</button>
        </div>
      </div>
      <div v-if="searchError" class="list-error">{{ searchError }}</div>
      <div v-else-if="messagesError" class="list-error">{{ messagesError }}</div>
      <div v-else-if="displayedLoading && displayedMessages.length === 0" class="list-loading">
        <span class="spinner" />
      </div>
      <div v-else-if="displayedMessages.length === 0 && !displayedLoading" class="list-empty">
        {{ isSearching ? 'No results.' : 'No messages.' }}
      </div>
      <div v-else class="message-list">
        <button
          v-for="m in displayedMessages"
          :key="m.id"
          :class="['message-row', { unread: !m.isRead, selected: selectedMessage?.id === m.id }]"
          @click="selectMessage(m)"
        >
          <div class="msg-top">
            <span class="msg-from">{{ displayName(m.from.emailAddress) }}</span>
            <span class="msg-date">{{ formatDate(m.receivedDateTime) }}</span>
          </div>
          <div class="msg-subject">{{ m.subject || '(no subject)' }}</div>
          <div class="msg-preview">{{ m.bodyPreview }}</div>
        </button>
        <button v-if="displayedHasMore" class="load-more" :disabled="displayedLoading" @click="loadMore">
          {{ displayedLoading ? 'Loading…' : 'Load more' }}
        </button>
      </div>
    </div>

    <!-- Divider: list / reading -->
    <div
      class="pane-divider"
      @pointerdown="onDividerPointerdown('list', $event)"
      @pointermove="onDividerPointermove"
      @pointerup="onDividerPointerup"
      @pointercancel="onDividerPointerup"
    />

    <!-- Right: reading pane / compose -->
    <div class="reading-pane">

      <!-- Empty state -->
      <div v-if="view === 'none'" class="reading-empty">
        Select a message to read
      </div>

      <!-- Compose new -->
      <div v-else-if="view === 'compose'" class="compose-view">
        <div class="compose-header">New Message</div>
        <form class="compose-form" @submit.prevent="sendEmail">
          <label class="compose-field">
            <span>To</span>
            <input v-model="composeForm.to" placeholder="recipient@example.com" required />
          </label>
          <label class="compose-field">
            <span>Subject</span>
            <input v-model="composeForm.subject" placeholder="Subject" />
          </label>
          <textarea v-model="composeForm.body" class="compose-body" placeholder="Write your message…" required />
          <div class="compose-actions">
            <button type="submit" class="btn-send" :disabled="sending">{{ sending ? 'Sending…' : 'Send' }}</button>
            <button type="button" class="btn-cancel" @click="view = 'none'">Cancel</button>
            <span v-if="sendMsg" class="send-msg">{{ sendMsg }}</span>
          </div>
        </form>
      </div>

      <!-- Message detail -->
      <div v-else-if="view === 'message'" class="message-detail">
        <div v-if="loadingMessage" class="reading-loading"><span class="spinner" /></div>
        <template v-else-if="selectedMessage">
          <!-- Header -->
          <div class="detail-header">
            <h2 class="detail-subject">{{ selectedMessage.subject || '(no subject)' }}</h2>
            <div class="detail-meta">
              <div class="meta-row">
                <span class="meta-label">From</span>
                <span class="meta-value">
                  {{ selectedMessage.from.emailAddress.name }}
                  <span class="meta-address">&lt;{{ selectedMessage.from.emailAddress.address }}&gt;</span>
                </span>
              </div>
              <div class="meta-row">
                <span class="meta-label">To</span>
                <span class="meta-value">
                  {{ selectedMessage.toRecipients.map(r => r.emailAddress.address).join(', ') }}
                </span>
              </div>
              <div v-if="selectedMessage.ccRecipients.length" class="meta-row">
                <span class="meta-label">CC</span>
                <span class="meta-value">
                  {{ selectedMessage.ccRecipients.map(r => r.emailAddress.address).join(', ') }}
                </span>
              </div>
              <div class="meta-row">
                <span class="meta-label">Date</span>
                <span class="meta-value">{{ new Date(selectedMessage.receivedDateTime).toLocaleString() }}</span>
              </div>
            </div>
          </div>

          <!-- Body -->
          <div class="detail-body">
            <iframe
              v-if="selectedMessage.body.contentType === 'HTML'"
              ref="iframeEl"
              :srcdoc="selectedMessage.body.content"
              sandbox="allow-popups allow-popups-to-escape-sandbox"
              class="email-frame"
              @load="onIframeLoad"
            />
            <pre v-else class="email-text">{{ selectedMessage.body.content }}</pre>
          </div>

          <!-- Reply area -->
          <div v-if="!showReply" class="reply-bar">
            <button class="btn-reply" @click="openReply">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="9 17 4 12 9 7"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>
              </svg>
              Reply
            </button>
          </div>

          <form v-else class="compose-form reply-form" @submit.prevent="sendEmail">
            <div class="compose-header">Reply</div>
            <label class="compose-field">
              <span>To</span>
              <input v-model="composeForm.to" required />
            </label>
            <textarea v-model="composeForm.body" class="compose-body reply-body" :placeholder="'Write your reply…' + replyBody" required />
            <div class="compose-actions">
              <button type="submit" class="btn-send" :disabled="sending">{{ sending ? 'Sending…' : 'Send Reply' }}</button>
              <button type="button" class="btn-cancel" @click="showReply = false">Cancel</button>
              <span v-if="sendMsg" class="send-msg">{{ sendMsg }}</span>
            </div>
          </form>
        </template>
      </div>

    </div>
  </div>
</template>

<style scoped>
/* ── Layout ── */
.email-layout {
  display: flex;
  height: 100%;
  overflow: hidden;
  background: #0f0f0f;
}

.center-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  gap: 0.75rem;
  color: #484848;
}
.state-icon { opacity: 0.35; margin-bottom: 0.25rem; }
.state-title { font-size: 0.9375rem; font-weight: 600; color: #585858; }
.state-sub { font-size: 0.8125rem; text-align: center; max-width: 24rem; line-height: 1.5; }

/* ── Folder pane ── */
.folder-pane {
  min-width: 120px;
  flex-shrink: 0;
  background: #141414;
  border-right: 1px solid #1e1e1e;
  display: flex;
  flex-direction: column;
  padding: 0.5rem;
  gap: 0.25rem;
  overflow-y: auto;
}

.btn-compose {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.45rem 0.75rem;
  margin-bottom: 0.25rem;
  background: #1e3a6e;
  color: #7ab0ff;
  border: 1px solid #2a4a8a;
  border-radius: 0.375rem;
  cursor: pointer;
  font-size: 0.8125rem;
  font-family: inherit;
  transition: background 0.12s;
  width: 100%;
  justify-content: center;
}
.btn-compose:hover { background: #254880; }

.folder-list { display: flex; flex-direction: column; gap: 0.125rem; }

.folder-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.4rem 0.625rem;
  border-radius: 0.3rem;
  background: none;
  border: none;
  color: #808080;
  font-size: 0.8125rem;
  cursor: pointer;
  text-align: left;
  font-family: inherit;
  transition: background 0.1s, color 0.1s;
  width: 100%;
}
.folder-item:hover { background: #1e1e1e; color: #c0c0c0; }
.folder-item.active { background: #1c2a3a; color: #7ab0ff; }

.folder-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.folder-error {
  font-size: 0.72rem;
  color: #c05050;
  padding: 0.25rem 0.5rem;
  cursor: default;
}

.list-error {
  padding: 1rem;
  color: #c05050;
  font-size: 0.775rem;
  line-height: 1.5;
  word-break: break-word;
}

.unread-badge {
  background: #1e3a6e;
  color: #7ab0ff;
  font-size: 0.65rem;
  font-weight: 600;
  padding: 0.1rem 0.35rem;
  border-radius: 999px;
  flex-shrink: 0;
  min-width: 1.2rem;
  text-align: center;
}

/* ── Message list pane ── */
.pane-divider {
  width: 4px;
  flex-shrink: 0;
  background: #1a1a1a;
  cursor: col-resize;
  transition: background 0.15s;
  position: relative;
}
.pane-divider:hover,
.pane-divider:active {
  background: #3a6adf;
}

.list-pane {
  min-width: 200px;
  flex-shrink: 0;
  border-right: none;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.list-header {
  padding: 0.5rem 0.875rem;
  border-bottom: 1px solid #1e1e1e;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}
.list-title { font-size: 0.8125rem; font-weight: 600; color: #c0c0c0; }

.search-wrap {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  background: #1a1a1a;
  border: 1px solid #252525;
  border-radius: 0.3rem;
  padding: 0.25rem 0.5rem;
}
.search-icon { color: #484848; flex-shrink: 0; }
.search-input {
  flex: 1;
  background: none;
  border: none;
  color: #c0c0c0;
  font-size: 0.775rem;
  font-family: inherit;
  outline: none;
  min-width: 0;
}
.search-input::placeholder { color: #404040; }
.search-clear {
  background: none;
  border: none;
  color: #484848;
  cursor: pointer;
  font-size: 0.65rem;
  padding: 0;
  line-height: 1;
  flex-shrink: 0;
  transition: color 0.1s;
}
.search-clear:hover { color: #909090; }

.list-loading, .list-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #484848;
  font-size: 0.8125rem;
}

.message-list {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

.message-row {
  padding: 0.625rem 0.875rem;
  border-bottom: 1px solid #181818;
  background: none;
  border-left: 3px solid transparent;
  cursor: pointer;
  text-align: left;
  font-family: inherit;
  width: 100%;
  transition: background 0.1s;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}
.message-row:hover { background: #161616; }
.message-row.selected { background: #141e2a; border-left-color: #3a6adf; }
.message-row.unread .msg-from { color: #e0e0e0; font-weight: 600; }
.message-row.unread .msg-subject { color: #d0d0d0; font-weight: 500; }

.msg-top { display: flex; justify-content: space-between; align-items: baseline; gap: 0.5rem; }
.msg-from { font-size: 0.8rem; color: #a0a0a0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.msg-date { font-size: 0.7rem; color: #505050; flex-shrink: 0; }
.msg-subject { font-size: 0.8rem; color: #909090; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.msg-preview { font-size: 0.75rem; color: #505050; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.load-more {
  padding: 0.625rem;
  background: none;
  border: none;
  color: #505050;
  font-size: 0.775rem;
  cursor: pointer;
  font-family: inherit;
  text-align: center;
  transition: color 0.1s;
}
.load-more:hover:not(:disabled) { color: #909090; }
.load-more:disabled { opacity: 0.5; }

/* ── Reading pane ── */
.reading-pane {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.reading-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #383838;
  font-size: 0.8125rem;
}

.reading-loading {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* ── Message detail ── */
.message-detail {
  display: flex;
  flex-direction: column;
  flex: 1;
}

.detail-header {
  padding: 1rem 1.25rem 0.75rem;
  border-bottom: 1px solid #1e1e1e;
  flex-shrink: 0;
}

.detail-subject {
  font-size: 0.9375rem;
  font-weight: 600;
  color: #e0e0e0;
  margin-bottom: 0.625rem;
  line-height: 1.35;
}

.detail-meta {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.meta-row {
  display: flex;
  gap: 0.625rem;
  font-size: 0.775rem;
}
.meta-label {
  color: #505050;
  min-width: 3rem;
  flex-shrink: 0;
}
.meta-value { color: #a0a0a0; }
.meta-address { color: #585858; margin-left: 0.25rem; }

.detail-body {
  padding: 0.875rem 1.25rem;
  flex-shrink: 0;
}

.email-frame {
  width: 100%;
  min-height: 200px;
  height: 400px;
  border: none;
  background: #fff;
  border-radius: 0.375rem;
  display: block;
}

.email-text {
  font-size: 0.8125rem;
  color: #c0c0c0;
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.6;
  font-family: ui-monospace, monospace;
}

/* ── Reply bar ── */
.reply-bar {
  padding: 0.75rem 1.25rem;
  border-top: 1px solid #1e1e1e;
  flex-shrink: 0;
}

.btn-reply {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.35rem 0.75rem;
  background: #1a1a1a;
  color: #909090;
  border: 1px solid #2a2a2a;
  border-radius: 0.375rem;
  cursor: pointer;
  font-size: 0.8rem;
  font-family: inherit;
  transition: background 0.1s, color 0.1s;
}
.btn-reply:hover { background: #222; color: #c0c0c0; }

/* ── Compose / reply form ── */
.compose-view {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 1rem 1.25rem;
}

.reply-form {
  border-top: 1px solid #1e1e1e;
  padding: 0.875rem 1.25rem;
  flex-shrink: 0;
}

.compose-header {
  font-size: 0.8125rem;
  font-weight: 600;
  color: #808080;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  margin-bottom: 0.75rem;
}

.compose-form {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.compose-field {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  border-bottom: 1px solid #1e1e1e;
  padding-bottom: 0.4rem;
}
.compose-field span {
  font-size: 0.775rem;
  color: #585858;
  min-width: 3.5rem;
  flex-shrink: 0;
}
.compose-field input {
  flex: 1;
  background: none;
  border: none;
  color: #d0d0d0;
  font-size: 0.8125rem;
  font-family: inherit;
  outline: none;
}
.compose-field input::placeholder { color: #404040; }

.compose-body {
  flex: 1;
  min-height: 180px;
  resize: none;
  background: none;
  border: none;
  color: #d0d0d0;
  font-size: 0.8125rem;
  font-family: inherit;
  outline: none;
  line-height: 1.6;
  padding: 0.5rem 0;
}
.compose-body::placeholder { color: #404040; }
.reply-body { min-height: 120px; }

.compose-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding-top: 0.25rem;
}

.btn-send {
  background: #1e3a6e;
  color: #7ab0ff;
  border: 1px solid #2a4a8a;
  border-radius: 0.375rem;
  padding: 0.375rem 0.875rem;
  cursor: pointer;
  font-size: 0.8rem;
  font-family: inherit;
  transition: background 0.12s;
}
.btn-send:hover:not(:disabled) { background: #254880; }
.btn-send:disabled { opacity: 0.5; }

.btn-cancel {
  background: none;
  color: #585858;
  border: none;
  padding: 0.375rem 0.5rem;
  cursor: pointer;
  font-size: 0.8rem;
  font-family: inherit;
  transition: color 0.1s;
}
.btn-cancel:hover { color: #909090; }

.send-msg { font-size: 0.775rem; color: #707070; }

/* ── Spinner ── */
.spinner {
  width: 18px; height: 18px;
  border: 2px solid #2a2a2a;
  border-top-color: #505050;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
  display: inline-block;
}
@keyframes spin { to { transform: rotate(360deg); } }
</style>
