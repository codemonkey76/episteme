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

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, {
    headers: { 'Content-Type': 'application/json', ...init?.headers },
    ...init,
  })
  if (!res.ok) {
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
export async function streamChat(
  sessionId: string,
  message: string,
  provider: string,
  onToken: (text: string) => void,
  onDone: () => void,
  onApproval: (actionId: string, toolName: string, toolArgs: unknown) => void,
  onTool: (name: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`${BASE}/sessions/${sessionId}/chat`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message, provider }),
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
  opts: { sessionId: string; provider: string; instruction?: string },
  onToken: (text: string) => void,
  onDone: () => void,
  onTool: (name: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`${BASE}/email/messages/${encodeURIComponent(messageId)}/advise`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ session_id: opts.sessionId, provider: opts.provider, instruction: opts.instruction }),
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

// Approvals
export const approvals = {
  listPending: (sessionId: string) =>
    json<{ pending_actions: PendingAction[] }>(`/sessions/${sessionId}/approvals`),
  approve: (actionId: string) =>
    fetch(BASE + `/approvals/${actionId}/approve`, { method: 'POST' }),
  reject: (actionId: string) =>
    fetch(BASE + `/approvals/${actionId}/reject`, { method: 'POST' }),
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
  from: { emailAddress: GraphEmailAddress }
  toRecipients: { emailAddress: GraphEmailAddress }[]
  bodyPreview: string
  receivedDateTime: string
  isRead: boolean
  hasAttachments: boolean
  // "notFlagged" | "flagged" | "complete" (present when `flag` was selected)
  flag?: { flagStatus?: string }
  // PidTagLastVerbExecuted (0x1081): "102"=reply, "103"=reply-all, "104"=forward
  singleValueExtendedProperties?: { id: string; value: string }[]
}

export interface MessageDetail extends MessageSummary {
  ccRecipients: { emailAddress: GraphEmailAddress }[]
  // Microsoft Graph returns this lowercase ("html"/"text"); compare case-insensitively.
  body: { contentType: string; content: string }
}

export interface SearchResult {
  value: MessageSummary[]
  next_link: string | null
}

export interface SendEmailPayload {
  to: string[]
  cc?: string[]
  bcc?: string[]
  subject?: string
  body: string
  reply_to_message_id?: string
  action?: 'reply' | 'replyAll' | 'forward'
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

export const email = {
  listFolders: () =>
    json<{ value: MailFolder[] }>('/email/folders'),
  listAttachments: (messageId: string) =>
    json<{ value: Attachment[] }>(`/email/messages/${encodeURIComponent(messageId)}/attachments`),
  // Direct URL for an attachment's bytes — usable as an <img>/<iframe> src or a download link.
  attachmentUrl: (messageId: string, attId: string) =>
    `${BASE}/email/messages/${encodeURIComponent(messageId)}/attachments/${encodeURIComponent(attId)}/raw`,
  listMessages: (folderId: string, skip = 0, top = 30) =>
    json<{ value: MessageSummary[] }>(`/email/folders/${folderId}/messages?skip=${skip}&top=${top}`),
  getMessage: (messageId: string) =>
    json<MessageDetail>(`/email/messages/${messageId}`),
  markRead: (messageId: string) =>
    fetch(BASE + `/email/messages/${messageId}/read`, { method: 'PATCH' }),
  search: (q: string, nextLink?: string | null) => {
    const params = new URLSearchParams({ q })
    if (nextLink) params.set('next_link', nextLink)
    return json<SearchResult>(`/email/search?${params}`)
  },
  send: (payload: SendEmailPayload) =>
    fetch(BASE + '/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
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

// Email auto-categorizer
export interface CategorizerConfig {
  enabled: boolean
  provider: string
  interval_secs: number
  batch_limit: number
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
  runNow: () =>
    json<CategorizerRunSummary>('/email/categorizer/run', { method: 'POST' }),
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
  },
}

// Auth (stubs — wired once backend auth is implemented)
export const auth = {
  changePassword: (_current: string, _next: string) =>
    Promise.reject(new Error('Not implemented yet.')),
  toggleTwoFactor: (_enable: boolean) =>
    Promise.reject(new Error('Not implemented yet.')),
  logout: () => Promise.resolve(),
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
    fetch(BASE + '/settings/mcp-servers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(s),
    }),
  deleteMcpServer: (name: string) =>
    fetch(BASE + `/settings/mcp-servers/${name}`, { method: 'DELETE' }),
}
