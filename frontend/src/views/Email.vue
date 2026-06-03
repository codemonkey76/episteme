<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'
import { useWindowsStore } from '../stores/windows'
import AttachmentViewer from '../components/AttachmentViewer.vue'

const logs = useLogsStore()
const windows = useWindowsStore()

// ── Connection state ──────────────────────────────────────────────────────────
const connected = ref(false)
const checkingConnection = ref(true)

// AI provider used for drafting replies (first configured provider).
const aiProvider = ref('')

onMounted(async () => {
  try {
    const cfg = await api.integrations.email.getConfig()
    connected.value = cfg.connected
  } finally {
    checkingConnection.value = false
  }
  if (connected.value) await loadFolders()
  try {
    const { providers } = await api.settings.listProviders()
    if (providers.length) aiProvider.value = providers[0].name
  } catch { /* no providers configured; AI reply will surface the error */ }
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
  logs.debug('Email', 'Loading mail folders')
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
    logs.info('Email', `Loaded ${folders.value.length} folders`)
    // Preserve the current selection across a refresh; fall back to Inbox on first load.
    const current = selectedFolder.value && folders.value.find(f => f.id === selectedFolder.value!.id)
    const target = current ?? folders.value.find(f => f.displayName === 'Inbox') ?? folders.value[0] ?? null
    if (target) selectFolder(target)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Failed to load folders'
    folderError.value = msg
    logs.error('Email', `Failed to load folders: ${msg}`)
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
  const folder = folders.value.find(f => f.id === folderId)
  logs.debug('Email', `Loading messages for "${folder?.displayName ?? folderId}" (skip=${skip})`)
  try {
    const res = await api.email.listMessages(folderId, skip, PAGE)
    if (skip === 0) {
      messages.value = res.value
    } else {
      messages.value.push(...res.value)
    }
    messagesSkip.value = skip + res.value.length
    messagesHasMore.value = res.value.length === PAGE
    logs.info('Email', `Loaded ${res.value.length} messages (total ${messagesSkip.value})`)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Failed to load messages'
    messagesError.value = msg
    logs.error('Email', `Failed to load messages: ${msg}`)
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
// Hides the HTML body until its height has been measured, so the user doesn't
// see the iframe render at the placeholder height and then jump to full size.
const bodyReady = ref(false)

// Attachments for the open message. Inline ones are embedded in the HTML body,
// so only non-inline attachments are surfaced as chips.
const attachments = ref<api.Attachment[]>([])
const visibleAttachments = computed(() => attachments.value.filter(a => !a.isInline))

function openAttachment(att: api.Attachment) {
  const m = selectedMessage.value
  if (!m) return
  windows.open({
    key: `attachment:${att.id}`,
    title: att.name,
    component: AttachmentViewer,
    props: {
      url: api.email.attachmentUrl(m.id, att.id),
      name: att.name,
      contentType: att.contentType,
    },
    width: 760,
    height: 620,
  })
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

// Microsoft Graph returns body.contentType lowercase ("html"/"text"), so
// compare case-insensitively rather than against a fixed-case literal.
const isHtmlBody = computed(
  () => selectedMessage.value?.body.contentType?.toLowerCase() === 'html',
)

async function selectMessage(summary: api.MessageSummary) {
  showReply.value = false
  view.value = 'message'
  loadingMessage.value = true
  attachments.value = []
  try {
    const detail = await api.email.getMessage(summary.id)
    selectedMessage.value = detail
    // Fetch attachment metadata in the background so the body renders right away.
    if (summary.hasAttachments) {
      api.email
        .listAttachments(summary.id)
        .then(r => { if (selectedMessage.value?.id === summary.id) attachments.value = r.value })
        .catch(() => { /* attachments are non-critical */ })
    }
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
  const el = iframeEl.value
  if (!el) return
  try {
    const doc = el.contentDocument
    if (doc) {
      const h = Math.max(doc.documentElement.scrollHeight, doc.body.scrollHeight)
      el.style.height = h + 16 + 'px'
    }
  } catch {}
  bodyReady.value = true
}

watch(selectedMessage, () => {
  // Hide and reset to a small placeholder before the new body loads so the
  // previous (possibly tall) email's height doesn't linger; onIframeLoad
  // re-measures and reveals it at the correct height.
  bodyReady.value = false
  nextTick(() => {
    if (iframeEl.value) iframeEl.value.style.height = '200px'
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

type ReplyMode = 'reply' | 'replyAll' | 'forward'
const replyMode = ref<ReplyMode>('reply')

// Sender + all To + Cc recipients, de-duplicated (case-insensitive). The field
// stays editable so the user can drop their own address if it's included.
function replyAllRecipients(m: api.MessageDetail): string {
  const seen = new Set<string>()
  return [
    m.from.emailAddress.address,
    ...m.toRecipients.map(r => r.emailAddress.address),
    ...m.ccRecipients.map(r => r.emailAddress.address),
  ]
    .filter(a => {
      const key = a?.toLowerCase()
      if (!key || seen.has(key)) return false
      seen.add(key)
      return true
    })
    .join(', ')
}

const aiDrafting = ref(false)

// Open a reply and ask the configured AI provider to draft the body. The draft
// lands in the editable composer so the user can revise it before sending.
async function aiReply() {
  const m = selectedMessage.value
  if (!m) return
  startReply('reply')
  if (!aiProvider.value) {
    sendMsg.value = 'No AI provider configured — add one in Settings.'
    return
  }
  aiDrafting.value = true
  composeForm.value.body = ''
  try {
    const bodyText =
      m.body.contentType?.toLowerCase() === 'html'
        ? (new DOMParser().parseFromString(m.body.content, 'text/html').body.textContent ?? '').trim()
        : m.body.content
    await api.streamAiDraft(
      {
        provider: aiProvider.value,
        from: `${m.from.emailAddress.name} <${m.from.emailAddress.address}>`,
        subject: m.subject ?? '',
        body: bodyText,
      },
      (text) => { composeForm.value.body += text },
    )
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Draft failed'
    sendMsg.value = `AI draft failed: ${msg}`
    logs.error('Email', `AI draft failed: ${msg}`)
  } finally {
    aiDrafting.value = false
  }
}

function startReply(mode: ReplyMode) {
  const m = selectedMessage.value
  if (!m) return
  replyMode.value = mode
  showReply.value = true
  sendMsg.value = ''
  const subject = m.subject ?? ''
  if (mode === 'forward') {
    composeForm.value = {
      to: '',
      subject: subject.startsWith('Fwd:') ? subject : `Fwd: ${subject}`,
      body: '',
    }
  } else {
    composeForm.value = {
      to: mode === 'replyAll' ? replyAllRecipients(m) : m.from.emailAddress.address,
      subject: subject.startsWith('Re:') ? subject : `Re: ${subject}`,
      body: '',
    }
  }
}

async function sendEmail() {
  sending.value = true
  sendMsg.value = ''
  const toList = composeForm.value.to.split(',').map(s => s.trim()).filter(Boolean)
  logs.info('Email', `Sending email to ${toList.join(', ')}`)
  try {
    const payload: api.SendEmailPayload = {
      to: composeForm.value.to.split(',').map(s => s.trim()).filter(Boolean),
      body: composeForm.value.body,
    }
    if (showReply.value && selectedMessage.value) {
      payload.reply_to_message_id = selectedMessage.value.id
      payload.action = replyMode.value
    } else {
      payload.subject = composeForm.value.subject
    }
    const res = await api.email.send(payload)
    if (!res.ok) throw new Error(`${res.status}`)
    logs.info('Email', 'Email sent successfully')
    sendMsg.value = 'Sent.'
    if (!showReply.value) {
      view.value = 'none'
    } else {
      // Optimistically mark the original as replied/forwarded so the list icon
      // updates immediately (Graph sets the real value server-side).
      const verb = replyMode.value === 'forward' ? '104' : '103'
      const id = selectedMessage.value?.id
      for (const list of [messages.value, searchResults.value]) {
        const item = list.find(x => x.id === id)
        if (item) item.singleValueExtendedProperties = [{ id: 'Integer 0x1081', value: verb }]
      }
      showReply.value = false
    }
    composeForm.value = { to: '', subject: '', body: '' }
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Send failed'
    sendMsg.value = `Failed: ${msg}`
    logs.error('Email', `Failed to send email: ${msg}`)
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
  if (!nextLink) logs.info('Email', `Searching for "${q}"`)
  try {
    const res = await api.email.search(q, nextLink)
    if (nextLink) {
      searchResults.value.push(...res.value)
    } else {
      searchResults.value = res.value
    }
    searchNextLink.value = res.next_link
    logs.info('Email', `Search returned ${res.value.length} results${nextLink ? ' (page+)' : ''}`)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Search failed'
    searchError.value = msg
    logs.error('Email', `Search failed: ${msg}`)
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

function isFlagged(m: api.MessageSummary): boolean {
  return m.flag?.flagStatus === 'flagged'
}

// Last action taken on the message, from PidTagLastVerbExecuted (0x1081):
// 102 = reply, 103 = reply-all (both shown as "reply"), 104 = forward.
function replyState(m: api.MessageSummary): 'reply' | 'forward' | null {
  const p = m.singleValueExtendedProperties?.find(x => x.id?.includes('0x1081'))
  const v = p ? parseInt(p.value, 10) : NaN
  if (v === 102 || v === 103) return 'reply'
  if (v === 104) return 'forward'
  return null
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
  <div v-if="checkingConnection" class="flex flex-col items-center justify-center h-full gap-3 text-[#484848]">
    <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[#505050] rounded-full animate-[spin_0.7s_linear_infinite]" />
  </div>

  <div v-else-if="!connected" class="flex flex-col items-center justify-center h-full gap-3 text-[#484848]">
    <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="opacity-35 mb-1">
      <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/>
      <polyline points="22,6 12,13 2,6"/>
    </svg>
    <p class="text-[0.9375rem] font-semibold text-[#585858]">No email account connected</p>
    <p class="text-[0.8125rem] text-center max-w-[24rem] leading-normal">Connect your Microsoft 365 account in Settings → Integrations.</p>
  </div>

  <!-- 3-pane layout -->
  <div v-else class="flex h-full overflow-hidden bg-bg">

    <!-- Left: folder list -->
    <aside class="min-w-[120px] flex-shrink-0 bg-[#141414] border-r border-[#1e1e1e] flex flex-col p-2 gap-1 overflow-y-auto" :style="{ width: folderPaneWidth + 'px' }">
      <div class="flex gap-1 mb-1">
        <button class="flex flex-1 items-center gap-[0.4rem] py-[0.45rem] px-3 bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded-md cursor-pointer text-[0.8125rem] font-[inherit] transition-colors duration-[0.12s] justify-center hover:bg-[#254880]" @click="openCompose">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
          </svg>
          Compose
        </button>
        <button
          class="flex items-center justify-center px-2 bg-surface text-[#808080] border border-raised rounded-md cursor-pointer transition-colors duration-[0.12s] hover:bg-[#222] hover:text-[#c0c0c0] disabled:opacity-50"
          title="Refresh folders"
          :disabled="loadingFolders"
          @click="loadFolders"
        >
          <svg :class="loadingFolders ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        </button>
      </div>
      <div v-if="folderError" class="text-[0.72rem] text-danger py-1 px-2 cursor-default" :title="folderError">⚠ Load failed</div>

      <nav class="flex flex-col gap-[0.125rem]">
        <button
          v-for="f in folders"
          :key="f.id"
          :class="['flex items-center justify-between py-[0.4rem] px-[0.625rem] rounded-[0.3rem] bg-transparent border-none text-[0.8125rem] cursor-pointer text-left font-[inherit] transition-colors duration-100 w-full', selectedFolder?.id === f.id ? 'bg-[#1c2a3a] text-[#7ab0ff]' : 'text-[#808080] hover:bg-[#1e1e1e] hover:text-[#c0c0c0]']"
          @click="selectFolder(f)"
        >
          <span class="overflow-hidden text-ellipsis whitespace-nowrap">{{ f.displayName }}</span>
          <span v-if="f.unreadItemCount > 0" class="bg-[#1e3a6e] text-[#7ab0ff] text-[0.65rem] font-semibold py-[0.1rem] px-[0.35rem] rounded-full flex-shrink-0 min-w-[1.2rem] text-center">{{ f.unreadItemCount }}</span>
        </button>
      </nav>
    </aside>

    <!-- Divider: folder / list -->
    <div
      class="w-[4px] flex-shrink-0 bg-surface cursor-col-resize transition-colors duration-150 relative hover:bg-[#3a6adf] active:bg-[#3a6adf]"
      @pointerdown="onDividerPointerdown('folder', $event)"
      @pointermove="onDividerPointermove"
      @pointerup="onDividerPointerup"
      @pointercancel="onDividerPointerup"
    />

    <!-- Middle: message list -->
    <div class="min-w-[200px] flex-shrink-0 flex flex-col overflow-hidden" :style="{ width: listPaneWidth + 'px' }">
      <div class="py-2 px-[0.875rem] border-b border-[#1e1e1e] flex-shrink-0 flex flex-col gap-[0.4rem]">
        <span class="text-[0.8125rem] font-semibold text-[#c0c0c0]">{{ selectedFolder?.displayName ?? '' }}</span>
        <div class="flex items-center gap-[0.375rem] bg-surface border border-[#252525] rounded-[0.3rem] py-1 px-2">
          <svg class="text-[#484848] flex-shrink-0" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
          <input
            v-model="searchQuery"
            class="flex-1 bg-transparent border-none text-[#c0c0c0] text-[0.775rem] font-[inherit] outline-none min-w-0 placeholder:text-[#404040]"
            placeholder="Search"
            autocomplete="off"
          />
          <button v-if="searchQuery" class="bg-transparent border-none text-[#484848] cursor-pointer text-[0.65rem] p-0 leading-none flex-shrink-0 transition-colors duration-100 hover:text-muted" @click="searchQuery = ''">✕</button>
        </div>
      </div>
      <div v-if="searchError" class="p-4 text-danger text-[0.775rem] leading-normal break-words">{{ searchError }}</div>
      <div v-else-if="messagesError" class="p-4 text-danger text-[0.775rem] leading-normal break-words">{{ messagesError }}</div>
      <div v-else-if="displayedLoading && displayedMessages.length === 0" class="flex-1 flex items-center justify-center text-[#484848] text-[0.8125rem]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[#505050] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="displayedMessages.length === 0 && !displayedLoading" class="flex-1 flex items-center justify-center text-[#484848] text-[0.8125rem]">
        {{ isSearching ? 'No results.' : 'No messages.' }}
      </div>
      <div v-else class="flex-1 overflow-y-auto flex flex-col">
        <button
          v-for="m in displayedMessages"
          :key="m.id"
          :class="['py-[0.625rem] px-[0.875rem] border-b border-[#181818] bg-transparent border-l-[3px] cursor-pointer text-left font-[inherit] w-full transition-colors duration-100 flex flex-col gap-[0.2rem]', selectedMessage?.id === m.id ? 'bg-[#141e2a] border-l-[#3a6adf]' : 'border-l-transparent hover:bg-[#161616]']"
          @click="selectMessage(m)"
        >
          <div class="flex justify-between items-baseline gap-2">
            <span :class="['text-[0.8rem] overflow-hidden text-ellipsis whitespace-nowrap', !m.isRead ? 'text-fg font-semibold' : 'text-[#a0a0a0]']">{{ displayName(m.from.emailAddress) }}</span>
            <span class="flex items-center gap-1 flex-shrink-0">
              <!-- replied / forwarded -->
              <svg v-if="replyState(m) === 'reply'" class="text-[#5a9ad0]" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" title="Replied">
                <polyline points="9 17 4 12 9 7"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>
              </svg>
              <svg v-else-if="replyState(m) === 'forward'" class="text-[#5a9ad0]" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" title="Forwarded">
                <polyline points="15 17 20 12 15 7"/><path d="M4 18v-2a4 4 0 0 1 4-4h12"/>
              </svg>
              <!-- flagged -->
              <svg v-if="isFlagged(m)" width="11" height="11" viewBox="0 0 24 24" fill="#d0823a" stroke="#d0823a" stroke-width="1.5" stroke-linejoin="round" title="Flagged">
                <path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15" stroke-linecap="round"/>
              </svg>
              <span class="text-[0.7rem] text-[#505050]">{{ formatDate(m.receivedDateTime) }}</span>
            </span>
          </div>
          <div :class="['text-[0.8rem] overflow-hidden text-ellipsis whitespace-nowrap', !m.isRead ? 'text-[#d0d0d0] font-medium' : 'text-muted']">{{ m.subject || '(no subject)' }}</div>
          <div class="text-[0.75rem] text-[#505050] overflow-hidden text-ellipsis whitespace-nowrap">{{ m.bodyPreview }}</div>
        </button>
        <button v-if="displayedHasMore" class="p-[0.625rem] bg-transparent border-none text-[#505050] text-[0.775rem] cursor-pointer font-[inherit] text-center transition-colors duration-100 hover:not-disabled:text-muted disabled:opacity-50" :disabled="displayedLoading" @click="loadMore">
          {{ displayedLoading ? 'Loading…' : 'Load more' }}
        </button>
      </div>
    </div>

    <!-- Divider: list / reading -->
    <div
      class="w-[4px] flex-shrink-0 bg-surface cursor-col-resize transition-colors duration-150 relative hover:bg-[#3a6adf] active:bg-[#3a6adf]"
      @pointerdown="onDividerPointerdown('list', $event)"
      @pointermove="onDividerPointermove"
      @pointerup="onDividerPointerup"
      @pointercancel="onDividerPointerup"
    />

    <!-- Right: reading pane / compose -->
    <div class="flex-1 overflow-y-auto flex flex-col min-w-0">

      <!-- Empty state -->
      <div v-if="view === 'none'" class="flex-1 flex items-center justify-center text-[#383838] text-[0.8125rem]">
        Select a message to read
      </div>

      <!-- Compose new -->
      <div v-else-if="view === 'compose'" class="flex-1 flex flex-col py-4 px-5">
        <div class="text-[0.8125rem] font-semibold text-[#808080] uppercase tracking-[0.06em] mb-3">New Message</div>
        <form class="flex flex-col gap-2" @submit.prevent="sendEmail">
          <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
            <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">To</span>
            <input v-model="composeForm.to" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="recipient@example.com" required />
          </label>
          <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
            <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">Subject</span>
            <input v-model="composeForm.subject" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="Subject" />
          </label>
          <textarea v-model="composeForm.body" class="flex-1 min-h-[180px] resize-none bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none leading-[1.6] py-2 px-0 placeholder:text-[#404040]" placeholder="Write your message…" required />
          <div class="flex items-center gap-2 pt-1">
            <button type="submit" class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded-md py-[0.375rem] px-[0.875rem] cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-[0.12s] hover:not-disabled:bg-[#254880] disabled:opacity-50" :disabled="sending">{{ sending ? 'Sending…' : 'Send' }}</button>
            <button type="button" class="bg-transparent text-[#585858] border-none py-[0.375rem] px-2 cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:text-muted" @click="view = 'none'">Cancel</button>
            <span v-if="sendMsg" class="text-[0.775rem] text-[#707070]">{{ sendMsg }}</span>
          </div>
        </form>
      </div>

      <!-- Message detail -->
      <div v-else-if="view === 'message'" class="flex flex-col flex-1">
        <div v-if="loadingMessage" class="flex-1 flex items-center justify-center"><span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[#505050] rounded-full animate-[spin_0.7s_linear_infinite]" /></div>
        <template v-else-if="selectedMessage">
          <!-- Header -->
          <div class="pt-4 px-5 pb-3 border-b border-[#1e1e1e] flex-shrink-0">
            <h2 class="text-[0.9375rem] font-semibold text-fg mb-[0.625rem] leading-[1.35]">{{ selectedMessage.subject || '(no subject)' }}</h2>
            <div class="flex flex-col gap-1">
              <div class="flex gap-[0.625rem] text-[0.775rem]">
                <span class="text-[#505050] min-w-[3rem] flex-shrink-0">From</span>
                <span class="text-[#a0a0a0]">
                  {{ selectedMessage.from.emailAddress.name }}
                  <span class="text-[#585858] ml-1">&lt;{{ selectedMessage.from.emailAddress.address }}&gt;</span>
                </span>
              </div>
              <div class="flex gap-[0.625rem] text-[0.775rem]">
                <span class="text-[#505050] min-w-[3rem] flex-shrink-0">To</span>
                <span class="text-[#a0a0a0]">
                  {{ selectedMessage.toRecipients.map(r => r.emailAddress.address).join(', ') }}
                </span>
              </div>
              <div v-if="selectedMessage.ccRecipients.length" class="flex gap-[0.625rem] text-[0.775rem]">
                <span class="text-[#505050] min-w-[3rem] flex-shrink-0">CC</span>
                <span class="text-[#a0a0a0]">
                  {{ selectedMessage.ccRecipients.map(r => r.emailAddress.address).join(', ') }}
                </span>
              </div>
              <div class="flex gap-[0.625rem] text-[0.775rem]">
                <span class="text-[#505050] min-w-[3rem] flex-shrink-0">Date</span>
                <span class="text-[#a0a0a0]">{{ new Date(selectedMessage.receivedDateTime).toLocaleString() }}</span>
              </div>
            </div>
          </div>

          <!-- Attachments -->
          <div v-if="visibleAttachments.length" class="flex flex-wrap gap-2 px-5 py-3 border-b border-[#1e1e1e] flex-shrink-0">
            <button
              v-for="att in visibleAttachments"
              :key="att.id"
              class="flex items-center gap-2 max-w-[16rem] bg-surface border border-raised rounded-md py-1.5 px-2.5 cursor-pointer text-left font-[inherit] transition-colors duration-100 hover:bg-[#222] hover:border-[#3a3a3a]"
              :title="`Open ${att.name}`"
              @click="openAttachment(att)"
            >
              <svg class="text-[#7ab0ff] flex-shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/>
              </svg>
              <span class="flex flex-col min-w-0">
                <span class="text-[0.78rem] text-[#d0d0d0] overflow-hidden text-ellipsis whitespace-nowrap">{{ att.name }}</span>
                <span class="text-[0.68rem] text-[#585858]">{{ formatSize(att.size) }}</span>
              </span>
            </button>
          </div>

          <!-- Body -->
          <div class="py-[0.875rem] px-5 flex-shrink-0">
            <!-- allow-same-origin (without allow-scripts) lets us measure the content
                 height so the iframe grows to fit; email scripts still can't run. -->
            <iframe
              v-if="isHtmlBody"
              ref="iframeEl"
              :srcdoc="selectedMessage.body.content"
              sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
              class="w-full min-h-[200px] border-none bg-white rounded-md block transition-opacity duration-150"
              :class="bodyReady ? 'opacity-100' : 'opacity-0'"
              @load="onIframeLoad"
            />
            <pre v-else class="text-[0.8125rem] text-[#c0c0c0] whitespace-pre-wrap break-words leading-[1.6] font-mono">{{ selectedMessage.body.content }}</pre>
          </div>

          <!-- Reply area -->
          <div v-if="!showReply" class="py-3 px-5 border-t border-[#1e1e1e] flex-shrink-0 flex gap-2">
            <button class="inline-flex items-center gap-[0.35rem] py-[0.35rem] px-3 bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded-md cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:bg-[#254880]" @click="aiReply">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z"/>
              </svg>
              AI reply
            </button>
            <button class="inline-flex items-center gap-[0.35rem] py-[0.35rem] px-3 bg-surface text-muted border border-raised rounded-md cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:bg-[#222] hover:text-[#c0c0c0]" @click="startReply('reply')">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="9 17 4 12 9 7"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>
              </svg>
              Reply
            </button>
            <button class="inline-flex items-center gap-[0.35rem] py-[0.35rem] px-3 bg-surface text-muted border border-raised rounded-md cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:bg-[#222] hover:text-[#c0c0c0]" @click="startReply('replyAll')">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="7 17 2 12 7 7"/><polyline points="12 17 7 12 12 7"/><path d="M22 18v-1a4 4 0 0 0-4-4H7"/>
              </svg>
              Reply all
            </button>
            <button class="inline-flex items-center gap-[0.35rem] py-[0.35rem] px-3 bg-surface text-muted border border-raised rounded-md cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:bg-[#222] hover:text-[#c0c0c0]" @click="startReply('forward')">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="15 17 20 12 15 7"/><path d="M4 18v-2a4 4 0 0 1 4-4h12"/>
              </svg>
              Forward
            </button>
          </div>

          <form v-else class="flex flex-col gap-2 border-t border-[#1e1e1e] py-[0.875rem] px-5 flex-shrink-0" @submit.prevent="sendEmail">
            <div class="flex items-center gap-2 mb-3">
              <span class="text-[0.8125rem] font-semibold text-[#808080] uppercase tracking-[0.06em]">{{ replyMode === 'forward' ? 'Forward' : replyMode === 'replyAll' ? 'Reply all' : 'Reply' }}</span>
              <span v-if="aiDrafting" class="inline-flex items-center gap-[0.35rem] text-[0.75rem] text-[#7ab0ff]">
                <span class="inline-block w-[11px] h-[11px] border-2 border-[#2a4a8a] border-t-[#7ab0ff] rounded-full animate-[spin_0.7s_linear_infinite]" />
                Drafting…
              </span>
            </div>
            <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
              <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">To</span>
              <input v-model="composeForm.to" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="recipient@example.com" required />
            </label>
            <textarea v-model="composeForm.body" class="flex-1 min-h-[120px] resize-none bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none leading-[1.6] py-2 px-0 placeholder:text-[#404040]" :placeholder="(replyMode === 'forward' ? 'Add a message…' : 'Write your reply…') + replyBody" :required="replyMode !== 'forward'" />
            <div class="flex items-center gap-2 pt-1">
              <button type="submit" class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded-md py-[0.375rem] px-[0.875rem] cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-[0.12s] hover:not-disabled:bg-[#254880] disabled:opacity-50" :disabled="sending || aiDrafting">{{ sending ? 'Sending…' : replyMode === 'forward' ? 'Send Forward' : 'Send Reply' }}</button>
              <button type="button" class="bg-transparent text-[#585858] border-none py-[0.375rem] px-2 cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:text-muted" @click="showReply = false">Cancel</button>
              <span v-if="sendMsg" class="text-[0.775rem] text-[#707070]">{{ sendMsg }}</span>
            </div>
          </form>
        </template>
      </div>

    </div>
  </div>
</template>
