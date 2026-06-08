const BASE = '/api'

export interface Session {
  id: string
  title: string
  created_at: string
  updated_at: string
}

export interface Message {
  id: string
  session_id: string
  // 'tool_call' is a client-only, display-only indicator (not persisted).
  role: 'user' | 'assistant' | 'tool' | 'tool_call'
  content: string
  tool_calls?: string
  tool_call_id?: string
  created_at: string
}

export interface PendingAction {
  id: string
  session_id: string
  tool_name: string
  tool_args: string
  status: string
  created_at: string
  resolved_at?: string
}

export interface ProviderConfig {
  name: string
  provider: string
  base_url?: string
  api_key?: string
  model_id: string
}

export interface McpServerConfig {
  name: string
  transport:
    | { type: 'stdio'; command: string; args: string[] }
    | { type: 'http'; url: string }
}

export interface McpServerStatus {
  name: string
  connected: boolean
  tool_count: number
  error?: string
}

export interface ToolInfo {
  name: string
  description: string
  group: string
  source: 'native' | 'mcp'
  policy: 'auto' | 'ask'
  suggest_ask: boolean
}

// Invoked whenever an API call comes back 401, so the app can drop to the login
// screen no matter which request tripped it. Registered by the auth store.
let onUnauthorized: (() => void) | null = null
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn
}

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, {
    headers: { 'Content-Type': 'application/json', ...init?.headers },
    ...init,
  })
  if (!res.ok) {
    // Don't fire the handler for the status probe itself — it's expected to
    // report "not authenticated" without bouncing the UI.
    if (res.status === 401 && path !== '/auth/status') onUnauthorized?.()
    let msg = `${res.status} ${res.statusText}`
    try {
      const body = await res.json()
      if (body?.error) msg = body.error
    } catch {}
    throw new Error(msg)
  }
  return res.json() as Promise<T>
}

// Sessions
export const sessions = {
  list: () => json<{ sessions: Session[] }>('/sessions'),
  create: (title?: string) =>
    json<{ session: Session }>('/sessions', {
      method: 'POST',
      body: JSON.stringify({ title }),
    }),
  get: (id: string) => json<{ session: Session }>(`/sessions/${id}`),
  update: (id: string, title: string) =>
    json<{ ok: boolean }>(`/sessions/${id}`, {
      method: 'PUT',
      body: JSON.stringify({ title }),
    }),
  delete: (id: string) =>
    fetch(BASE + `/sessions/${id}`, { method: 'DELETE' }),
  messages: (id: string) =>
    json<{ messages: Message[] }>(`/sessions/${id}/messages`),
}

// Chat — POST returns an SSE stream; use fetch + ReadableStream (EventSource only supports GET).
export interface ChatImage {
  mime: string
  /** Raw image bytes, base64-encoded (no `data:` prefix). */
  b64: string
}

export async function streamChat(
  sessionId: string,
  message: string,
  provider: string,
  onToken: (text: string) => void,
  onDone: () => void,
  onApproval: (actionId: string, toolName: string, toolArgs: unknown) => void,
  onTool: (name: string) => void,
  signal?: AbortSignal,
  images?: ChatImage[],
): Promise<void> {
  const res = await fetch(`${BASE}/sessions/${sessionId}/chat`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message, provider, images: images ?? [] }),
    signal,
  })

  if (!res.ok || !res.body) throw new Error(`${res.status} ${res.statusText}`)

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { value, done } = await reader.read()
    if (done) break

    buffer += decoder.decode(value, { stream: true })
    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''

    for (const line of lines) {
      if (!line.startsWith('data: ')) continue
      const raw = line.slice(6).trim()
      if (!raw || raw === '[DONE]') continue

      let data: { type: string; text?: string; name?: string; action_id?: string; tool_name?: string; tool_args?: unknown }
      try { data = JSON.parse(raw) } catch { continue }

      if (data.type === 'token' && data.text != null) {
        onToken(data.text)
      } else if (data.type === 'tool' && data.name) {
        onTool(data.name)
      } else if (data.type === 'done') {
        onDone()
        return
      } else if (data.type === 'awaiting_approval' && data.action_id && data.tool_name) {
        onApproval(data.action_id, data.tool_name, data.tool_args)
      }
    }
  }
}

// Ask AI about an email — seeds a chat session server-side, then streams advice.
export async function streamAdvise(
  messageId: string,
  opts: { sessionId: string; provider: string; instruction?: string; mailbox?: string },
  onToken: (text: string) => void,
  onDone: () => void,
  onTool: (name: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`${BASE}/email/messages/${encodeURIComponent(messageId)}/advise`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ session_id: opts.sessionId, provider: opts.provider, instruction: opts.instruction, mailbox: opts.mailbox }),
    signal,
  })
  if (!res.ok || !res.body) throw new Error(`${res.status} ${res.statusText}`)

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { value, done } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })
    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''
    for (const line of lines) {
      if (!line.startsWith('data: ')) continue
      const raw = line.slice(6).trim()
      if (!raw) continue
      let data: { type: string; text?: string; name?: string }
      try { data = JSON.parse(raw) } catch { continue }
      if (data.type === 'token' && data.text != null) onToken(data.text)
      else if (data.type === 'tool' && data.name) onTool(data.name)
      else if (data.type === 'done') { onDone(); return }
    }
  }
}

// Web push (browser notifications)
export const push = {
  vapidKey: () => json<{ public_key: string }>('/push/vapid'),
  /** Register this browser's PushSubscription JSON as a web push token. */
  register: (subscription: string) =>
    fetch(BASE + '/push/register', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token: subscription, platform: 'web' }),
    }),
}

// Approvals
export const approvals = {
  listPending: (sessionId: string) =>
    json<{ pending_actions: PendingAction[] }>(`/sessions/${sessionId}/approvals`),
  // Pass editedArgs to run the operator's edits instead of the model's draft;
  // omit it to approve the draft verbatim.
  approve: (actionId: string, editedArgs?: Record<string, unknown>) =>
    fetch(BASE + `/approvals/${actionId}/approve`, {
      method: 'POST',
      ...(editedArgs
        ? {
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ edited_args: editedArgs }),
          }
        : {}),
    }),
  reject: (actionId: string) =>
    fetch(BASE + `/approvals/${actionId}/reject`, { method: 'POST' }),
}

// Jobs (background/scheduled agent runs) + the global approval queue
export interface Job {
  id: string
  user_id: string
  session_id: string
  kind: 'background' | 'scheduled' | 'research'
  name: string
  provider: string
  status: 'running' | 'needs_approval' | 'done' | 'failed'
  summary: string | null
  error: string | null
  created_at: string
  updated_at: string
}

export interface PendingActionGlobal {
  id: string
  session_id: string
  session_title: string
  tool_name: string
  tool_args: string
  created_at: string
}

export const jobs = {
  list: () => json<{ jobs: Job[] }>('/jobs'),
  pendingAll: () =>
    json<{ pending_actions: PendingActionGlobal[] }>('/approvals/pending'),
  /** Fail an in-flight job (e.g. one stuck on an unresponsive provider). */
  cancel: (id: string) =>
    json<{ cancelled: boolean }>(`/jobs/${id}/cancel`, { method: 'POST' }),
  /** Re-run a failed research job as a fresh run (new session + report). */
  retry: (id: string) =>
    json<{ job_id: string; session_id: string }>(`/jobs/${id}/retry`, { method: 'POST' }),
}

// Email (Microsoft Graph proxy)
export interface MailFolder {
  id: string
  displayName: string
  unreadItemCount: number
  totalItemCount: number
}

export interface GraphEmailAddress {
  name: string
  address: string
}

export interface MessageSummary {
  id: string
  subject: string | null
  // Null for drafts / items with no sender (common in Deleted Items).
  from: { emailAddress: GraphEmailAddress } | null
  toRecipients: { emailAddress: GraphEmailAddress }[]
  bodyPreview: string
  receivedDateTime: string
  isRead: boolean
  hasAttachments: boolean
  // True for unsent drafts — opens in the composer for editing, not the reader.
  isDraft?: boolean
  // "notFlagged" | "flagged" | "complete" (present when `flag` was selected)
  flag?: { flagStatus?: string }
  // PidTagLastVerbExecuted (0x1081): "102"=reply, "103"=reply-all, "104"=forward
  singleValueExtendedProperties?: { id: string; value: string }[]
}

export interface MessageDetail extends MessageSummary {
  ccRecipients: { emailAddress: GraphEmailAddress }[]
  bccRecipients?: { emailAddress: GraphEmailAddress }[]
  // Microsoft Graph returns this lowercase ("html"/"text"); compare case-insensitively.
  body: { contentType: string; content: string }
}

export interface SearchResult {
  value: MessageSummary[]
  next_link: string | null
}

/// AI-extracted helpdesk ticket, reviewed/edited before creation. The
/// client/requester ids come from the preview's sender match.
export interface TicketDraft {
  client_id: number
  requester_id: number
  client: string
  subject: string
  description: string
  priority: 'low' | 'medium' | 'high' | 'critical'
  /** How many image attachments the source email has (preview only). */
  attachment_count?: number
}

export interface AttachmentUpload {
  name: string
  content_type: string
  /// Raw file bytes, base64-encoded (no data: prefix).
  content_bytes: string
  is_inline?: boolean
  /// Content-ID for inline images, referenced in the body as `cid:<contentId>`.
  content_id?: string
}

export interface SendEmailPayload {
  to: string[]
  cc?: string[]
  bcc?: string[]
  subject?: string
  /// HTML body of the user's message. Quoted history is NOT included on
  /// replies/forwards — the backend's createReply/createForward draft keeps it.
  body: string
  reply_to_message_id?: string
  /// An existing draft being edited — sent in place, keeping its attachments.
  draft_id?: string
  action?: 'reply' | 'replyAll' | 'forward'
  attachments?: AttachmentUpload[]
  /// Original AI draft, when the send started from "AI reply" — the backend
  /// diffs it against `body` to learn writing-style preferences.
  ai_draft?: string
  ai_provider?: string
  /// Plain text of the message being replied to — context so commitment
  /// detection can resolve terse replies ("I'll get this done tonight").
  reply_context?: string
  /// Shared mailbox address to send as (omit for the user's own mailbox).
  mailbox?: string
}

export interface Attachment {
  id: string
  name: string
  contentType: string
  size: number
  isInline: boolean
  // Content-ID for inline images, referenced in the HTML body as `cid:<contentId>`.
  contentId?: string
}

// `mailbox` is the address of a shared mailbox to act on; omit for the user's
// own mailbox. Appended as `?mailbox=` (or `&mailbox=`) on each request.
function mbq(mailbox?: string, sep: '?' | '&' = '&'): string {
  return mailbox ? `${sep}mailbox=${encodeURIComponent(mailbox)}` : ''
}

export const email = {
  listFolders: (mailbox?: string) =>
    json<{ value: MailFolder[] }>(`/email/folders${mbq(mailbox, '?')}`),
  listAttachments: (messageId: string, mailbox?: string) =>
    json<{ value: Attachment[] }>(`/email/messages/${encodeURIComponent(messageId)}/attachments${mbq(mailbox, '?')}`),
  // Direct URL for an attachment's bytes — usable as an <img>/<iframe> src or a download link.
  attachmentUrl: (messageId: string, attId: string, mailbox?: string) =>
    `${BASE}/email/messages/${encodeURIComponent(messageId)}/attachments/${encodeURIComponent(attId)}/raw${mbq(mailbox, '?')}`,
  listMessages: (folderId: string, skip = 0, top = 30, mailbox?: string) =>
    json<{ value: MessageSummary[] }>(`/email/folders/${folderId}/messages?skip=${skip}&top=${top}${mbq(mailbox)}`),
  getMessage: (messageId: string, mailbox?: string) =>
    json<MessageDetail>(`/email/messages/${messageId}${mbq(mailbox, '?')}`),
  markAllRead: (folderId: string, mailbox?: string) =>
    json<{ marked: number }>(`/email/folders/${encodeURIComponent(folderId)}/read-all${mbq(mailbox, '?')}`, {
      method: 'POST',
    }),
  markRead: (messageId: string, mailbox?: string) =>
    fetch(BASE + `/email/messages/${messageId}/read${mbq(mailbox, '?')}`, { method: 'PATCH' }),
  // "No response needed" — complete the flag and file to Processed.
  markDone: (messageId: string, mailbox?: string) =>
    fetch(BASE + `/email/messages/${encodeURIComponent(messageId)}/done${mbq(mailbox, '?')}`, { method: 'POST' }),
  // Soft delete — move messages to Deleted Items.
  deleteMessages: (ids: string[], mailbox?: string) =>
    json<{ deleted: number }>('/email/messages/delete', {
      method: 'POST',
      body: JSON.stringify({ ids, mailbox }),
    }),
  // Move messages into a folder (by id, or a well-known name) — drag-and-drop.
  moveMessages: (ids: string[], destination: string, mailbox?: string) =>
    json<{ moved: number }>('/email/messages/move', {
      method: 'POST',
      body: JSON.stringify({ ids, destination, mailbox }),
    }),
  // Step 1: match the sender to a helpdesk contact + AI-extract the ticket
  // fields, for the user to review/edit before creating.
  ticketPreview: (messageId: string, payload: { provider: string; mailbox?: string }) =>
    json<TicketDraft>(
      `/email/messages/${encodeURIComponent(messageId)}/ticket/preview`,
      { method: 'POST', body: JSON.stringify(payload) },
    ),
  // Step 2: create the (possibly edited) ticket. message_id/mailbox let the
  // backend forward the email's image attachments onto the ticket.
  createTicket: (draft: TicketDraft, ctx?: { message_id: string; mailbox?: string }) =>
    json<{ reference: string; id: number; subject: string; priority: string; client: string; attached: number }>(
      '/email/tickets',
      { method: 'POST', body: JSON.stringify({ ...draft, ...ctx }) },
    ),
  search: (q: string, nextLink?: string | null, mailbox?: string) => {
    const params = new URLSearchParams({ q })
    if (nextLink) params.set('next_link', nextLink)
    if (mailbox) params.set('mailbox', mailbox)
    return json<SearchResult>(`/email/search?${params}`)
  },
  send: (payload: SendEmailPayload) =>
    fetch(BASE + '/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
  // Per-mailbox HTML signatures ("" key = the user's own mailbox).
  getSignatures: () => json<Record<string, string>>('/email/signatures'),
  saveSignatures: (map: Record<string, string>) =>
    fetch(BASE + '/email/signatures', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(map),
    }),
}

// AI draft — POST returns an SSE stream of reply tokens (model can be slow, so stream live).
export async function streamAiDraft(
  payload: { provider: string; from: string; subject: string; body: string },
  onToken: (text: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`${BASE}/email/ai-draft`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    signal,
  })

  if (!res.ok || !res.body) throw new Error(`${res.status} ${res.statusText}`)

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { value, done } = await reader.read()
    if (done) break

    buffer += decoder.decode(value, { stream: true })
    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''

    for (const line of lines) {
      if (!line.startsWith('data: ')) continue
      const raw = line.slice(6).trim()
      if (!raw || raw === '[DONE]') continue

      let data: { type: string; text?: string; message?: string }
      try { data = JSON.parse(raw) } catch { continue }

      if (data.type === 'token' && data.text != null) onToken(data.text)
      else if (data.type === 'error') throw new Error(data.message || 'draft failed')
    }
  }
}

// AI summary — POST returns an SSE stream of summary tokens (inline, no chat session).
export async function streamSummary(
  messageId: string,
  opts: { provider: string; mailbox?: string },
  onToken: (text: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`${BASE}/email/messages/${encodeURIComponent(messageId)}/summarize`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ provider: opts.provider, mailbox: opts.mailbox }),
    signal,
  })

  if (!res.ok || !res.body) throw new Error(`${res.status} ${res.statusText}`)

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { value, done } = await reader.read()
    if (done) break

    buffer += decoder.decode(value, { stream: true })
    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''

    for (const line of lines) {
      if (!line.startsWith('data: ')) continue
      const raw = line.slice(6).trim()
      if (!raw || raw === '[DONE]') continue

      let data: { type: string; text?: string; message?: string }
      try { data = JSON.parse(raw) } catch { continue }

      if (data.type === 'token' && data.text != null) onToken(data.text)
      else if (data.type === 'error') throw new Error(data.message || 'summary failed')
    }
  }
}

// Calendar (Microsoft Graph)
export interface CalendarEvent {
  id: string
  subject: string
  start: string // RFC3339 UTC
  end: string
  location: string
  is_all_day: boolean
  web_link: string
}

export interface NewCalendarEvent {
  subject: string
  start: string // RFC3339 (with offset)
  end?: string
  is_all_day?: boolean
  location?: string
  body?: string
  reminder_minutes_before?: number
}

export const calendar = {
  list: (params?: { start?: string; end?: string }) => {
    const p = new URLSearchParams()
    if (params?.start) p.set('start', params.start)
    if (params?.end) p.set('end', params.end)
    return json<{ events: CalendarEvent[] }>(`/calendar/events?${p}`)
  },
  create: (payload: NewCalendarEvent) =>
    json<{ event: CalendarEvent }>('/calendar/events', {
      method: 'POST',
      body: JSON.stringify(payload),
    }),
  remove: (id: string) =>
    fetch(BASE + `/calendar/events/${encodeURIComponent(id)}`, { method: 'DELETE' }),
}

// Memories
export interface Memory {
  id: string
  content: string
  category: string
  source: string
  session_id: string | null
  created_at: string
  updated_at: string
}

export const memories = {
  list: (params?: { category?: string; q?: string; limit?: number }) => {
    const p = new URLSearchParams()
    if (params?.category && params.category !== 'All') p.set('category', params.category)
    if (params?.q) p.set('q', params.q)
    if (params?.limit !== undefined) p.set('limit', String(params.limit))
    return json<{ memories: Memory[] }>(`/memories?${p}`)
  },
  create: (content: string, category: string) =>
    json<{ memory: Memory }>('/memories', {
      method: 'POST',
      body: JSON.stringify({ content, category }),
    }),
  update: (id: string, content: string, category: string) =>
    fetch(BASE + `/memories/${id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content, category }),
    }),
  remove: (id: string) => fetch(BASE + `/memories/${id}`, { method: 'DELETE' }),
}

// Usage (admin)
export interface UsageRow {
  username: string
  provider: string
  model_id: string
  purpose: string
  requests: number
  prompt_tokens: number
  completion_tokens: number
  /** Dollar cost under the admin price table; null when the model is unpriced. */
  cost: number | null
}

export const usageSummary = (days = 30) =>
  json<{ days: number; usage: UsageRow[] }>(`/usage/summary?days=${days}`)

// Model prices (admin): $ per million tokens, substring-matched on model id.
export interface ModelPrice {
  model: string
  prompt_per_mtok: number
  completion_per_mtok: number
}

export const modelPrices = {
  get: () => json<{ prices: ModelPrice[] }>('/settings/model-prices'),
  set: (prices: ModelPrice[]) =>
    json<{ prices: ModelPrice[] }>('/settings/model-prices', {
      method: 'PUT',
      body: JSON.stringify({ prices }),
    }),
}

// Scheduled agents
export interface ScheduledAgent {
  id: string
  name: string
  time: string
  days: string[]
  provider: string
  instructions: string
  enabled: boolean
  last_run: string
}

export const scheduledAgents = {
  list: () => json<{ agents: ScheduledAgent[] }>('/scheduled-agents'),
  save: (agents: ScheduledAgent[]) =>
    json<{ agents: ScheduledAgent[] }>('/scheduled-agents', {
      method: 'PUT',
      body: JSON.stringify(agents),
    }),
  run: (id: string) =>
    json<{ session_id: string }>(`/scheduled-agents/${id}/run`, { method: 'POST' }),
}

// Research reports
export interface Report {
  id: string
  job_id: string | null
  title: string
  created_at: string
  /** Public share token; non-null = anyone with the link can view it. */
  share_token: string | null
}

export const reports = {
  list: () => json<{ reports: Report[] }>('/reports'),
  htmlPath: (id: string) => `${BASE}/reports/${id}/html`,
  remove: (id: string) => fetch(BASE + `/reports/${id}`, { method: 'DELETE' }),
  startResearch: (topic: string, depth: 'quick' | 'standard' | 'deep') =>
    json<{ job_id: string; session_id: string }>('/research', {
      method: 'POST',
      body: JSON.stringify({ topic, depth }),
    }),
  /** Mint (or return the existing) public link; `path` is origin-relative. */
  share: (id: string) =>
    json<{ token: string; path: string }>(`/reports/${id}/share`, { method: 'POST' }),
  unshare: (id: string) => fetch(BASE + `/reports/${id}/share`, { method: 'DELETE' }),
}

// Conversation search
export interface SearchHit {
  session_id: string
  session_title: string
  message_id: string
  role: string
  snippet: string
  created_at: string
}

export const searchSessions = (q: string, limit = 30) => {
  const p = new URLSearchParams({ q, limit: String(limit) })
  return json<{ hits: SearchHit[] }>(`/sessions/search?${p}`)
}

// Documents
export interface Document {
  id: string
  filename: string
  mime: string
  size: number
  status: 'indexing' | 'ready' | 'error'
  error_message: string | null
  created_at: string
  chunk_count: number
}

export const documents = {
  list: () => json<{ documents: Document[] }>('/documents'),
  upload: (filename: string, contentType: string, contentBytes: string) =>
    json<{ document: Document }>('/documents', {
      method: 'POST',
      body: JSON.stringify({
        filename,
        content_type: contentType,
        content_bytes: contentBytes,
      }),
    }),
  remove: (id: string) => fetch(BASE + `/documents/${id}`, { method: 'DELETE' }),
}

// Tasks
export interface Task {
  id: string
  title: string
  notes: string | null
  due_at: string | null
  priority: 'low' | 'normal' | 'high'
  status: 'open' | 'done'
  // null = the implicit "General" list.
  list_id: string | null
  created_at: string
  updated_at: string
}

export interface TaskListInfo {
  id: string
  name: string
  created_at: string
}

export const tasks = {
  // `list` filters to one to-do list: 'general' for the default, or a list id.
  list: (params?: { status?: string; q?: string; limit?: number; list?: string }) => {
    const p = new URLSearchParams()
    if (params?.status && params.status !== 'all') p.set('status', params.status)
    if (params?.q) p.set('q', params.q)
    if (params?.limit !== undefined) p.set('limit', String(params.limit))
    if (params?.list) p.set('list', params.list)
    return json<{ tasks: Task[] }>(`/tasks?${p}`)
  },
  create: (task: { title: string; notes?: string; due_at?: string; priority?: string; list_id?: string }) =>
    json<{ task: Task }>('/tasks', {
      method: 'POST',
      body: JSON.stringify(task),
    }),
  // Partial update; pass null for notes/due_at to clear, null list_id → General.
  update: (
    id: string,
    patch: Partial<{ title: string; notes: string | null; due_at: string | null; priority: string; status: string; list_id: string | null }>,
  ) =>
    json<{ task: Task }>(`/tasks/${id}`, {
      method: 'PUT',
      body: JSON.stringify(patch),
    }),
  remove: (id: string) => fetch(BASE + `/tasks/${id}`, { method: 'DELETE' }),
  // ── To-do lists (the implicit "General" list is not included) ──
  lists: () => json<{ lists: TaskListInfo[] }>('/tasks/lists'),
  createList: (name: string) =>
    json<{ list: TaskListInfo }>('/tasks/lists', { method: 'POST', body: JSON.stringify({ name }) }),
  renameList: (id: string, name: string) =>
    fetch(BASE + `/tasks/lists/${encodeURIComponent(id)}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name }),
    }),
  removeList: (id: string) =>
    fetch(BASE + `/tasks/lists/${encodeURIComponent(id)}`, { method: 'DELETE' }),
}

// Notes
export interface Note {
  id: string
  title: string
  content: string
  created_at: string
  updated_at: string
}

export const notes = {
  list: (params?: { q?: string; limit?: number }) => {
    const p = new URLSearchParams()
    if (params?.q) p.set('q', params.q)
    if (params?.limit !== undefined) p.set('limit', String(params.limit))
    return json<{ notes: Note[] }>(`/notes?${p}`)
  },
  create: (title: string, content: string) =>
    json<{ note: Note }>('/notes', {
      method: 'POST',
      body: JSON.stringify({ title, content }),
    }),
  update: (id: string, patch: Partial<{ title: string; content: string }>) =>
    json<{ note: Note }>(`/notes/${id}`, {
      method: 'PUT',
      body: JSON.stringify(patch),
    }),
  remove: (id: string) => fetch(BASE + `/notes/${id}`, { method: 'DELETE' }),
}

// Suggestions (commitments detected in sent emails)
export interface Suggestion {
  id: string
  kind: 'task' | 'event'
  title: string
  start_at: string | null
  end_at: string | null
  context: string | null
  status: string
  created_at: string
}

export const suggestions = {
  listPending: () => json<{ suggestions: Suggestion[] }>('/suggestions'),
  accept: (id: string) =>
    json<{ kind: string }>(`/suggestions/${id}/accept`, { method: 'POST' }),
  dismiss: (id: string) => fetch(BASE + `/suggestions/${id}/dismiss`, { method: 'POST' }),
}

// Email auto-categorizer
export interface CategorizerTask {
  mailbox: string // '' = own mailbox
  enabled: boolean
  provider: string
  instructions: string // extra sorting rules, appended to the base prompt
}

export interface CategorizerConfig {
  interval_secs: number
  batch_limit: number
  tasks: CategorizerTask[]
}

export interface CategorizerRunSummary {
  scanned: number
  moved: number
  flagged: number
  skipped: number
  message: string
}

export const emailCategorizer = {
  getConfig: () => json<CategorizerConfig>('/email/categorizer'),
  saveConfig: (cfg: CategorizerConfig) =>
    json<CategorizerConfig>('/email/categorizer', {
      method: 'PUT',
      body: JSON.stringify(cfg),
    }),
  runNow: (mailbox?: string) =>
    json<CategorizerRunSummary>(`/email/categorizer/run${mailbox ? `?mailbox=${encodeURIComponent(mailbox)}` : ''}`, { method: 'POST' }),
}

// Integrations
export interface EmailConfigStatus {
  configured: boolean
  connected: boolean
  tenant_id: string
  client_id: string
  connected_email: string | null
}

export interface SaveEmailConfig {
  tenant_id: string
  client_id: string
  client_secret?: string
}

export interface SharedMailbox {
  address: string
  name: string | null
}

export const integrations = {
  email: {
    getConfig: () => json<EmailConfigStatus>('/integrations/email/config'),
    saveConfig: (config: SaveEmailConfig) =>
      fetch(BASE + '/integrations/email/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
      }),
    disconnect: () =>
      fetch(BASE + '/integrations/email/config', { method: 'DELETE' }),
    listShared: () =>
      json<{ mailboxes: SharedMailbox[] }>('/integrations/email/shared'),
    // Verifies access server-side; rejects (throws) if the user can't open it.
    addShared: (address: string, name?: string) =>
      json<{ mailboxes: SharedMailbox[] }>('/integrations/email/shared', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ address, name }),
      }),
    removeShared: (address: string) =>
      fetch(BASE + `/integrations/email/shared/${encodeURIComponent(address)}`, { method: 'DELETE' }),
  },
  helpdesk: {
    getConfig: () => json<HelpdeskStatus>('/integrations/helpdesk/config'),
    // Logs in with the credentials once; only the resulting token is stored.
    connect: (base_url: string, email: string, password: string) =>
      json<HelpdeskStatus>('/integrations/helpdesk/config', {
        method: 'POST',
        body: JSON.stringify({ base_url, email, password }),
      }),
    disconnect: () =>
      fetch(BASE + '/integrations/helpdesk/config', { method: 'DELETE' }),
  },
  github: {
    getConfig: () => json<GithubStatus>('/integrations/github/config'),
    // Verifies the token (GET /user); only the token is stored.
    connect: (token: string, default_owner: string) =>
      json<GithubStatus>('/integrations/github/config', {
        method: 'POST',
        body: JSON.stringify({ token, default_owner }),
      }),
    disconnect: () => fetch(BASE + '/integrations/github/config', { method: 'DELETE' }),
  },
}

export interface HelpdeskStatus {
  connected: boolean
  base_url: string
  email: string
}

export interface GithubStatus {
  connected: boolean
  login: string
  default_owner: string
}

// Auth
export interface AuthStatus {
  setup_required: boolean
  authenticated: boolean
  username: string | null
  role: 'admin' | 'member' | null
  impersonator: string | null
}

export interface UserAccount {
  id: string
  username: string
  role: 'admin' | 'member'
  status: 'active' | 'disabled'
  created_at: string
}

export interface Invite {
  code: string
  label: string
  created_at: string
  expires_at: string
  used_by: string | null
  used_at: string | null
}

export const users = {
  list: () => json<{ users: UserAccount[] }>('/users'),
  impersonate: (id: string) =>
    json<{ ok: true; username: string }>(`/users/${id}/impersonate`, { method: 'POST' }),
  disable: (id: string) => fetch(BASE + `/users/${id}/disable`, { method: 'POST' }),
  enable: (id: string) => fetch(BASE + `/users/${id}/enable`, { method: 'POST' }),
  remove: (id: string) => fetch(BASE + `/users/${id}`, { method: 'DELETE' }),
}

export interface PromptInfo {
  key: string
  name: string
  description: string
  variables: string[]
  default: string
  content: string
  customized: boolean
}

export const prompts = {
  list: () => json<{ prompts: PromptInfo[] }>('/settings/prompts'),
  save: (key: string, content: string) =>
    fetch(BASE + `/settings/prompts/${key}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content }),
    }),
  reset: (key: string) => fetch(BASE + `/settings/prompts/${key}`, { method: 'DELETE' }),
}

export const invites = {
  create: (label: string) =>
    json<{ invite: Invite }>('/admin/invites', {
      method: 'POST',
      body: JSON.stringify({ label }),
    }),
  list: () => json<{ invites: Invite[] }>('/admin/invites'),
  revoke: (code: string) =>
    fetch(BASE + `/admin/invites/${code}`, { method: 'DELETE' }),
}

export const auth = {
  status: () => json<AuthStatus>('/auth/status'),
  stopImpersonating: () =>
    json<{ ok: true; username: string }>('/auth/stop-impersonating', { method: 'POST' }),
  checkInvite: (code: string) =>
    json<{ valid: boolean; label: string | null }>(`/auth/invite/${encodeURIComponent(code)}`),
  register: (code: string, username: string, password: string) =>
    json<{ ok: true; username: string }>('/auth/register', {
      method: 'POST',
      body: JSON.stringify({ code, username, password }),
    }),
  setup: (username: string, password: string) =>
    json<{ ok: true; username: string }>('/auth/setup', {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    }),
  login: (username: string, password: string, code?: string) =>
    json<{ ok: boolean; username?: string; totp_required?: boolean }>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ username, password, code }),
    }),
  logout: () =>
    fetch(BASE + '/auth/logout', { method: 'POST' }).then(() => undefined),
  changePassword: (current: string, next: string) =>
    json<{ ok: true }>('/auth/change-password', {
      method: 'POST',
      body: JSON.stringify({ current, next }),
    }),
  // Two-factor (TOTP) enrollment.
  twoFactor: {
    status: () => json<{ enabled: boolean; recovery_codes_left: number }>('/auth/2fa'),
    /** Begin enrollment: pending secret + otpauth URL + QR (inline SVG). */
    setup: () =>
      json<{ secret: string; otpauth_url: string; qr_svg: string }>('/auth/2fa/setup', {
        method: 'POST',
      }),
    /** Verify the first code; returns the recovery codes — shown exactly once. */
    enable: (code: string) =>
      json<{ ok: true; recovery_codes: string[] }>('/auth/2fa/enable', {
        method: 'POST',
        body: JSON.stringify({ code }),
      }),
    disable: (password: string) =>
      json<{ ok: true }>('/auth/2fa/disable', {
        method: 'POST',
        body: JSON.stringify({ password }),
      }),
  },
}

// Logs
export interface LogEntry {
  id: string
  ts: number
  category: string
  level: string
  message: string
}

export const logs = {
  create: (entry: LogEntry) =>
    fetch(BASE + '/logs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(entry),
    }),
  list: (params?: { limit?: number; offset?: number; category?: string; level?: string; q?: string }) => {
    const p = new URLSearchParams()
    if (params?.limit !== undefined) p.set('limit', String(params.limit))
    if (params?.offset !== undefined) p.set('offset', String(params.offset))
    if (params?.category) p.set('category', params.category)
    if (params?.level) p.set('level', params.level)
    if (params?.q) p.set('q', params.q)
    return json<{ entries: LogEntry[] }>(`/logs?${p}`)
  },
  clear: () => fetch(BASE + '/logs', { method: 'DELETE' }),
  streamUrl: `${BASE}/logs/stream`,
}

// Settings
export const settings = {
  listProviders: () => json<{ providers: ProviderConfig[] }>('/settings/providers'),
  listOllamaModels: (baseUrl: string) =>
    json<{ models: string[] }>(`/settings/ollama/models?base_url=${encodeURIComponent(baseUrl)}`),
  upsertProvider: (p: ProviderConfig) =>
    fetch(BASE + '/settings/providers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(p),
    }),
  deleteProvider: (name: string) =>
    fetch(BASE + `/settings/providers/${name}`, { method: 'DELETE' }),
  listMcpServers: () =>
    json<{ mcp_servers: McpServerConfig[] }>('/settings/mcp-servers'),
  upsertMcpServer: (s: McpServerConfig) =>
    json<{ status: McpServerStatus }>('/settings/mcp-servers', {
      method: 'POST',
      body: JSON.stringify(s),
    }),
  deleteMcpServer: (name: string) =>
    fetch(BASE + `/settings/mcp-servers/${name}`, { method: 'DELETE' }),
  mcpServerStatus: () =>
    json<{ statuses: McpServerStatus[] }>('/settings/mcp-servers/status'),
  listTools: () => json<{ tools: ToolInfo[] }>('/settings/tools'),
  setToolPolicy: (name: string, policy: 'auto' | 'ask') =>
    fetch(BASE + '/settings/tools', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, policy }),
    }),
  getTheme: () => json<{ theme: string | null }>('/settings/theme'),
  setTheme: (theme: string) =>
    fetch(BASE + '/settings/theme', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ theme }),
    }),
  getTimezone: () => json<{ timezone: string; configured: boolean }>('/settings/timezone'),
  setTimezone: async (timezone: string) => {
    const res = await fetch(BASE + '/settings/timezone', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ timezone }),
    })
    if (!res.ok) {
      let msg = `${res.status} ${res.statusText}`
      try {
        const body = await res.json()
        if (body?.error) msg = body.error
      } catch {}
      throw new Error(msg)
    }
  },
}
