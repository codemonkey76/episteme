<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'
import { useWindowsStore } from '../stores/windows'
import { useSessionsStore } from '../stores/sessions'
import { useTasksStore } from '../stores/tasks'
import { useCalendarStore } from '../stores/calendar'
import AttachmentViewer from '../components/AttachmentViewer.vue'

const logs = useLogsStore()
const windows = useWindowsStore()
const sessions = useSessionsStore()
const tasksStore = useTasksStore()
const calStore = useCalendarStore()

// ── Connection state ──────────────────────────────────────────────────────────
const connected = ref(false)
const checkingConnection = ref(true)

// AI provider for drafting replies and "Ask AI" (defaults to first configured).
const aiProvider = ref('')
const providersList = ref<api.ProviderConfig[]>([])
const providerMenuOpen = ref(false)

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
    providersList.value = providers
    if (providers.length) aiProvider.value = providers[0].name
  } catch { /* no providers configured; AI reply will surface the error */ }
  // Pick up suggestions left over from earlier sends.
  if (connected.value) await loadSuggestions()
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

// Rewrite inline `cid:` image references to the attachment proxy URL so embedded
// images render (browsers can't resolve cid: URLs). Matches each `cid:NAME@host`
// to the attachment named NAME (Outlook's classic convention), with a contentId
// check when present. OWA and other clients often use opaque content-IDs
// (`cid:part1.abc@…`) that match neither — those fall back to pairing unmatched
// cid references with unused inline attachments in document order (Graph can't
// give us contentId: selecting it 400s on the base attachment collection).
// Updates reactively once the attachment metadata loads.
const renderedBody = computed(() => {
  const m = selectedMessage.value
  if (!m) return ''
  const html = m.body.content
  if (!isHtmlBody.value || attachments.value.length === 0) return html

  const cidRe = /cid:([^"'\s)>]+)/gi
  const cids = [...new Set([...html.matchAll(cidRe)].map(x => x[1]))]
  const used = new Set<string>()
  const assigned = new Map<string, api.Attachment>()

  // Pass 1: exact contentId/name matches claim their attachments first.
  for (const cid of cids) {
    const prefix = cid.split('@')[0]
    const att = attachments.value.find(a =>
      (a.contentId && a.contentId.replace(/^<|>$/g, '') === cid) ||
      a.name === cid ||
      a.name === prefix,
    )
    if (att) {
      used.add(att.id)
      assigned.set(cid, att)
    }
  }
  // Pass 2: remaining cid refs pair with unclaimed inline attachments in order.
  for (const cid of cids) {
    if (assigned.has(cid)) continue
    const att = attachments.value.find(a => a.isInline && !used.has(a.id))
    if (att) {
      used.add(att.id)
      assigned.set(cid, att)
    }
  }

  return html.replace(cidRe, (full, cid: string) => {
    const att = assigned.get(cid)
    return att ? api.email.attachmentUrl(m.id, att.id) : full
  })
})

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
    // NOTE: Graph's `hasAttachments` is false when a message has ONLY inline
    // images, so also fetch when the HTML body references `cid:` content.
    const hasInlineCid =
      detail.body?.contentType?.toLowerCase() === 'html' && detail.body.content.includes('cid:')
    if (summary.hasAttachments || hasInlineCid) {
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
const composeForm = ref({ to: '', cc: '', bcc: '', subject: '', body: '' })
const showCcBcc = ref(false)
const sending = ref(false)
const sendMsg = ref('')

function openCompose() {
  selectedMessage.value = null
  showReply.value = false
  view.value = 'compose'
  composeForm.value = { to: '', cc: '', bcc: '', subject: '', body: '' }
  showCcBcc.value = false
  sendMsg.value = ''
}

type ReplyMode = 'reply' | 'replyAll' | 'forward'
const replyMode = ref<ReplyMode>('reply')

// Comma-join unique addresses (case-insensitive); fields stay editable.
function dedupAddrs(addrs: string[]): string {
  const seen = new Set<string>()
  return addrs
    .filter(a => {
      const key = a?.toLowerCase()
      if (!key || seen.has(key)) return false
      seen.add(key)
      return true
    })
    .join(', ')
}
// Reply-all: sender + original To go to To; original Cc goes to Cc.
function replyAllTo(m: api.MessageDetail): string {
  return dedupAddrs([m.from.emailAddress.address, ...m.toRecipients.map(r => r.emailAddress.address)])
}
function replyAllCc(m: api.MessageDetail): string {
  return dedupAddrs(m.ccRecipients.map(r => r.emailAddress.address))
}

const aiDrafting = ref(false)
const aiWarming = ref(false)

// Extract just the newest message from an email, dropping quoted thread history
// so the AI replies to the latest message rather than to quoted prior replies.
function latestMessageText(m: api.MessageDetail): string {
  const isHtml = m.body.contentType?.toLowerCase() === 'html'
  const text = isHtml ? htmlToText(m.body.content) : m.body.content
  return truncateAtQuote(text)
}

// HTML → plain text PRESERVING line breaks (block tags / <br> become newlines),
// so line-anchored quote markers can match. Quoted blockquotes are dropped.
function htmlToText(html: string): string {
  let s = html
    .replace(/<(script|style)[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<blockquote[\s\S]*?<\/blockquote>/gi, '\n__QUOTE__\n')
    .replace(/<\/(p|div|tr|li|h[1-6])>/gi, '\n')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<[^>]+>/g, '')
  // Decode HTML entities safely (no script execution).
  const ta = document.createElement('textarea')
  ta.innerHTML = s
  s = ta.value
  return s
    .split('\n')
    .map(l => l.trim())
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

// Cut text at the first quoted-reply marker (Outlook "From:/Sent:" header,
// Gmail "On … wrote:", "Original Message", dividers, plain-text ">", or a
// dropped blockquote). Falls back to the full text if that would remove everything.
function truncateAtQuote(text: string): string {
  const markers = [
    /^From:\s.+\r?\n\s*(Sent|Date):\s/im,
    /^-{2,}\s*Original Message\s*-{2,}/im,
    /^On\s[\s\S]{1,200}?\bwrote:\s*$/im,
    /^_{5,}\s*$/m,
    /^>{1,}\s/m,
    /\n__QUOTE__/,
  ]
  let cut = text.length
  for (const re of markers) {
    const match = re.exec(text)
    if (match && match.index > 0 && match.index < cut) cut = match.index
  }
  const head = text.slice(0, cut).replace(/__QUOTE__/g, '').trim()
  return head.length >= 2 ? head : text.replace(/__QUOTE__/g, '').trim()
}

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
  aiWarming.value = false
  composeForm.value.body = ''
  // If no token arrives within a few seconds, the model is likely cold-loading.
  let firstToken = false
  const t0 = Date.now()
  const warmTimer = setTimeout(() => { if (!firstToken) aiWarming.value = true }, 4000)
  logs.info('Email', `AI draft requested via "${aiProvider.value}"`)
  try {
    const bodyText = latestMessageText(m)
    await api.streamAiDraft(
      {
        provider: aiProvider.value,
        from: `${m.from.emailAddress.name} <${m.from.emailAddress.address}>`,
        subject: m.subject ?? '',
        body: bodyText,
      },
      (text) => {
        if (!firstToken) {
          firstToken = true
          aiWarming.value = false
          logs.info('Email', `AI draft started after ${((Date.now() - t0) / 1000).toFixed(1)}s`)
        }
        composeForm.value.body += text
      },
    )
    aiDraftOriginal.value = composeForm.value.body
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Draft failed'
    sendMsg.value = `AI draft failed: ${msg}`
    logs.error('Email', `AI draft failed: ${msg}`)
  } finally {
    clearTimeout(warmTimer)
    aiWarming.value = false
    aiDrafting.value = false
  }
}

// Hand the email (text + inline images) to a new chat session and stream advice
// on what to do. Continues as a normal conversation for follow-ups.
const asking = ref(false)
async function askAi() {
  const m = selectedMessage.value
  if (!m || asking.value) return
  if (!aiProvider.value) {
    logs.error('Email', 'No AI provider configured — add one in Settings.')
    return
  }
  asking.value = true
  const subject = m.subject ?? '(no subject)'
  try {
    const s = await sessions.createSession(`Email: ${subject}`)
    await sessions.loadSession(s.id)
    // Display-only stand-in for the (multimodal) message seeded server-side.
    sessions.appendMessage({
      id: crypto.randomUUID(),
      session_id: s.id,
      role: 'user',
      content: `📧 Advise on this email: ${subject}`,
      created_at: new Date().toISOString(),
    })
    windows.openKey('chat', undefined, 'fill')
    await api.streamAdvise(
      m.id,
      { sessionId: s.id, provider: aiProvider.value },
      (tok) => sessions.appendToken(tok),
      () => {},
      () => {},
    )
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Failed'
    logs.error('Email', `Ask AI failed: ${msg}`)
  } finally {
    asking.value = false
  }
}

// Snapshot of the AI-generated draft, taken when streaming completes — sent
// alongside the (possibly edited) body so the backend can learn style
// preferences from the diff. Cleared whenever a compose form is (re)opened.
const aiDraftOriginal = ref('')

// ── Commitment suggestions ────────────────────────────────────────────────────
// Detected asynchronously after a send; shown as accept/dismiss toasts.
const suggestionsList = ref<api.Suggestion[]>([])
const acceptedIds = ref<Set<string>>(new Set())
let suggestionPollTimer: number | null = null

async function loadSuggestions() {
  try {
    const res = await api.suggestions.listPending()
    // Merge: keep cards already shown (incl. just-accepted ones mid-fade).
    const known = new Set(suggestionsList.value.map(s => s.id))
    for (const s of res.suggestions) {
      if (!known.has(s.id)) suggestionsList.value.push(s)
    }
  } catch { /* cosmetic — never disturb the mail flow */ }
}

// Detection takes the model a few seconds; poll briefly after each send.
function pollSuggestionsAfterSend() {
  if (suggestionPollTimer !== null) window.clearInterval(suggestionPollTimer)
  let polls = 0
  suggestionPollTimer = window.setInterval(async () => {
    polls += 1
    await loadSuggestions()
    if (polls >= 15 && suggestionPollTimer !== null) {
      window.clearInterval(suggestionPollTimer)
      suggestionPollTimer = null
    }
  }, 3000)
}

async function acceptSuggestion(s: api.Suggestion) {
  try {
    await api.suggestions.accept(s.id)
    acceptedIds.value.add(s.id)
    if (s.kind === 'event') calStore.notifyChanged()
    else tasksStore.notifyChanged()
    window.setTimeout(() => {
      suggestionsList.value = suggestionsList.value.filter(x => x.id !== s.id)
    }, 1500)
  } catch (e: unknown) {
    logs.error('Email', `Failed to accept suggestion: ${e instanceof Error ? e.message : e}`)
    suggestionsList.value = suggestionsList.value.filter(x => x.id !== s.id)
  }
}

async function dismissSuggestion(s: api.Suggestion) {
  suggestionsList.value = suggestionsList.value.filter(x => x.id !== s.id)
  try {
    await api.suggestions.dismiss(s.id)
  } catch { /* row stays pending; it'll reappear next mount */ }
}

function fmtWhen(iso: string): string {
  return new Date(iso).toLocaleString([], {
    weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
  })
}

// "No response needed" — file straight to Processed and close the reader.
const markingDone = ref(false)
async function markDone() {
  const m = selectedMessage.value
  if (!m || markingDone.value) return
  markingDone.value = true
  try {
    const res = await api.email.markDone(m.id)
    if (!res.ok) throw new Error(`${res.status}`)
    logs.info('Email', `Marked done: ${m.subject ?? '(no subject)'}`)
    view.value = 'none'
    selectedMessage.value = null
    await loadFolders()
  } catch (e: unknown) {
    logs.error('Email', `Mark done failed: ${e instanceof Error ? e.message : e}`)
  } finally {
    markingDone.value = false
  }
}

function startReply(mode: ReplyMode) {
  const m = selectedMessage.value
  if (!m) return
  replyMode.value = mode
  showReply.value = true
  sendMsg.value = ''
  showCcBcc.value = false
  aiDraftOriginal.value = ''
  const subject = m.subject ?? ''
  if (mode === 'forward') {
    composeForm.value = {
      to: '', cc: '', bcc: '',
      subject: subject.startsWith('Fwd:') ? subject : `Fwd: ${subject}`,
      body: '',
    }
  } else if (mode === 'replyAll') {
    const cc = replyAllCc(m)
    composeForm.value = {
      to: replyAllTo(m), cc, bcc: '',
      subject: subject.startsWith('Re:') ? subject : `Re: ${subject}`,
      body: '',
    }
    if (cc) showCcBcc.value = true // reveal Cc so the prefilled recipients are visible
  } else {
    composeForm.value = {
      to: m.from.emailAddress.address, cc: '', bcc: '',
      subject: subject.startsWith('Re:') ? subject : `Re: ${subject}`,
      body: '',
    }
  }
}

function parseAddrs(s: string): string[] {
  return s.split(',').map(a => a.trim()).filter(Boolean)
}

async function sendEmail() {
  sending.value = true
  sendMsg.value = ''
  const toList = parseAddrs(composeForm.value.to)
  logs.info('Email', `Sending email to ${toList.join(', ')}`)
  try {
    const payload: api.SendEmailPayload = {
      to: toList,
      cc: parseAddrs(composeForm.value.cc),
      bcc: parseAddrs(composeForm.value.bcc),
      body: composeForm.value.body,
    }
    if (showReply.value && selectedMessage.value) {
      payload.reply_to_message_id = selectedMessage.value.id
      payload.action = replyMode.value
      // Subject + latest message text give commitment detection enough
      // context to resolve terse replies into specific task titles.
      payload.subject = composeForm.value.subject
      payload.reply_context = latestMessageText(selectedMessage.value).slice(0, 2000)
    } else {
      payload.subject = composeForm.value.subject
    }
    // Provider powers post-send analysis (style learning + commitment
    // detection) — sent with every send, AI-drafted or not.
    if (aiProvider.value) payload.ai_provider = aiProvider.value
    // Started as an AI draft → let the backend learn from the edits.
    if (aiDraftOriginal.value) payload.ai_draft = aiDraftOriginal.value
    const res = await api.email.send(payload)
    if (!res.ok) throw new Error(`${res.status}`)
    logs.info('Email', 'Email sent successfully')
    sendMsg.value = 'Sent.'
    if (aiProvider.value) pollSuggestionsAfterSend()
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
      // Replies get filed to "Processed" server-side a moment later — refresh
      // so the original drops out of the Inbox list and counts update.
      if (replyMode.value !== 'forward') {
        window.setTimeout(() => {
          if (selectedFolder.value) loadMessages(selectedFolder.value.id)
          loadFolders()
        }, 3000)
      }
    }
    composeForm.value = { to: '', cc: '', bcc: '', subject: '', body: '' }
    showCcBcc.value = false
    aiDraftOriginal.value = ''
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
  if (suggestionPollTimer !== null) window.clearInterval(suggestionPollTimer)
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
  <div v-else class="relative flex h-full overflow-hidden bg-bg">

    <!-- Commitment suggestion toasts -->
    <div v-if="suggestionsList.length" class="absolute bottom-3 right-3 z-50 flex flex-col gap-2 max-w-[26rem]">
      <div v-for="s in suggestionsList" :key="s.id" class="flex flex-col gap-1.5 bg-[#10161a] border border-[#1e3a4a] rounded-lg py-2.5 px-3 shadow-lg">
        <div class="flex items-center gap-2 text-[0.78rem] text-[#6ab8df]">
          <svg v-if="s.kind === 'event'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>
          </svg>
          <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 11 12 14 22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>
          </svg>
          <span class="font-medium">You committed to something — add {{ s.kind === 'event' ? 'to calendar' : 'a task' }}?</span>
        </div>
        <p class="text-[0.8125rem] text-[#d0d0d0] leading-snug">{{ s.title }}</p>
        <p class="text-[0.72rem] text-[#587078]">
          <template v-if="s.start_at">{{ fmtWhen(s.start_at) }}<template v-if="s.end_at"> – {{ fmtWhen(s.end_at) }}</template></template>
          <template v-else>No due time</template>
          <template v-if="s.context"> · {{ s.context }}</template>
        </p>
        <div class="flex items-center gap-2 mt-0.5">
          <template v-if="acceptedIds.has(s.id)">
            <span class="text-[0.75rem] text-[#6ecf8e]">Added ✓</span>
          </template>
          <template v-else>
            <button class="bg-[#1e3a2a] text-[#6ecf8e] border border-[#2a5a3a] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[#254a35]" @click="acceptSuggestion(s)">
              {{ s.kind === 'event' ? 'Add event' : 'Add task' }}
            </button>
            <button class="bg-transparent text-[#585858] border border-[#303030] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="dismissSuggestion(s)">Dismiss</button>
          </template>
        </div>
      </div>
    </div>

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
            <button v-if="!showCcBcc" type="button" class="text-[0.7rem] text-[#5a7da0] hover:text-[#7ab0ff] flex-shrink-0" @click.prevent.stop="showCcBcc = true">Cc/Bcc</button>
          </label>
          <template v-if="showCcBcc">
            <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
              <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">Cc</span>
              <input v-model="composeForm.cc" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="cc@example.com" />
            </label>
            <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
              <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">Bcc</span>
              <input v-model="composeForm.bcc" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="bcc@example.com" />
            </label>
          </template>
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
              :srcdoc="renderedBody"
              sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
              class="w-full min-h-[200px] border-none bg-white rounded-md block transition-opacity duration-150"
              :class="bodyReady ? 'opacity-100' : 'opacity-0'"
              @load="onIframeLoad"
            />
            <pre v-else class="text-[0.8125rem] text-[#c0c0c0] whitespace-pre-wrap break-words leading-[1.6] font-mono">{{ selectedMessage.body.content }}</pre>
          </div>

          <!-- Reply area -->
          <div v-if="!showReply" class="py-3 px-5 border-t border-[#1e1e1e] flex-shrink-0 flex gap-2 items-center flex-wrap">
            <button class="inline-flex items-center gap-[0.35rem] py-[0.35rem] px-3 bg-[#23304a] text-[#a0c8ff] border border-[#2a4a8a] rounded-md cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:bg-[#2a3c5c] disabled:opacity-50" :disabled="asking" title="Send this email (and its images) to the AI for advice" @click="askAi">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
              </svg>
              {{ asking ? 'Asking…' : 'Ask AI' }}
            </button>
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
            <button class="inline-flex items-center gap-[0.35rem] py-[0.35rem] px-3 bg-[#1e3a2a] text-[#6ecf8e] border border-[#2a5a3a] rounded-md cursor-pointer text-[0.8rem] font-[inherit] transition-colors duration-100 hover:bg-[#254a35] disabled:opacity-50" :disabled="markingDone" title="No response needed — complete the flag and file to Processed" @click="markDone">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12"/>
              </svg>
              {{ markingDone ? 'Filing…' : 'Done' }}
            </button>

            <!-- AI model picker — compact icon dropdown, right-aligned -->
            <div v-if="providersList.length > 1" class="relative ml-auto">
              <button
                type="button"
                class="inline-flex items-center gap-[0.2rem] py-[0.35rem] px-2 bg-surface text-[#909090] border border-raised rounded-md cursor-pointer transition-colors duration-100 hover:bg-[#222] hover:text-[#c0c0c0]"
                :title="`AI model: ${aiProvider}`"
                @click="providerMenuOpen = !providerMenuOpen"
              >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
                  <polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/>
                </svg>
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
              </button>
              <template v-if="providerMenuOpen">
                <div class="fixed inset-0 z-[40]" @click="providerMenuOpen = false" />
                <div class="absolute right-0 bottom-full mb-1.5 z-[41] min-w-[12rem] bg-[#1c1c1c] border border-[#303030] rounded-md shadow-[0_8px_24px_rgba(0,0,0,0.5)] py-1">
                  <div class="px-3 py-1 text-[0.62rem] uppercase tracking-[0.06em] text-[#585858]">AI model</div>
                  <button
                    v-for="p in providersList"
                    :key="p.name"
                    type="button"
                    class="flex items-center gap-2 w-full px-3 py-1.5 text-left text-[0.78rem] font-[inherit] cursor-pointer hover:bg-[#262626]"
                    @click="aiProvider = p.name; providerMenuOpen = false"
                  >
                    <svg class="flex-shrink-0" :class="p.name === aiProvider ? 'text-[#7ab0ff]' : 'text-transparent'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                    <span :class="p.name === aiProvider ? 'text-[#7ab0ff]' : 'text-[#c0c0c0]'">{{ p.name }}</span>
                  </button>
                </div>
              </template>
            </div>
          </div>

          <form v-else class="flex flex-col gap-2 border-t border-[#1e1e1e] py-[0.875rem] px-5 flex-shrink-0" @submit.prevent="sendEmail">
            <div class="flex items-center gap-2 mb-3">
              <span class="text-[0.8125rem] font-semibold text-[#808080] uppercase tracking-[0.06em]">{{ replyMode === 'forward' ? 'Forward' : replyMode === 'replyAll' ? 'Reply all' : 'Reply' }}</span>
              <span v-if="aiDrafting" class="inline-flex items-center gap-[0.35rem] text-[0.75rem] text-[#7ab0ff]">
                <span class="inline-block w-[11px] h-[11px] border-2 border-[#2a4a8a] border-t-[#7ab0ff] rounded-full animate-[spin_0.7s_linear_infinite]" />
                {{ aiWarming ? 'Loading model (first use can take a while)…' : 'Drafting…' }}
              </span>
            </div>
            <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
              <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">To</span>
              <input v-model="composeForm.to" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="recipient@example.com" required />
              <button v-if="!showCcBcc" type="button" class="text-[0.7rem] text-[#5a7da0] hover:text-[#7ab0ff] flex-shrink-0" @click.prevent.stop="showCcBcc = true">Cc/Bcc</button>
            </label>
            <template v-if="showCcBcc">
              <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
                <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">Cc</span>
                <input v-model="composeForm.cc" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="cc@example.com" />
              </label>
              <label class="flex items-center gap-[0.625rem] border-b border-[#1e1e1e] pb-[0.4rem]">
                <span class="text-[0.775rem] text-[#585858] min-w-[3.5rem] flex-shrink-0">Bcc</span>
                <input v-model="composeForm.bcc" class="flex-1 bg-transparent border-none text-[#d0d0d0] text-[0.8125rem] font-[inherit] outline-none placeholder:text-[#404040]" placeholder="bcc@example.com" />
              </label>
            </template>
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
