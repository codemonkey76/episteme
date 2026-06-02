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
  role: 'user' | 'assistant' | 'tool'
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
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
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

      let data: { type: string; text?: string; action_id?: string; tool_name?: string; tool_args?: unknown }
      try { data = JSON.parse(raw) } catch { continue }

      if (data.type === 'token' && data.text != null) {
        onToken(data.text)
      } else if (data.type === 'done') {
        onDone()
        return
      } else if (data.type === 'awaiting_approval' && data.action_id && data.tool_name) {
        onApproval(data.action_id, data.tool_name, data.tool_args)
      }
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
}

export interface MessageDetail extends MessageSummary {
  ccRecipients: { emailAddress: GraphEmailAddress }[]
  body: { contentType: 'Text' | 'HTML'; content: string }
}

export interface SendEmailPayload {
  to: string[]
  subject?: string
  body: string
  reply_to_message_id?: string
}

export const email = {
  listFolders: () =>
    json<{ value: MailFolder[] }>('/email/folders'),
  listMessages: (folderId: string, skip = 0, top = 30) =>
    json<{ value: MessageSummary[] }>(`/email/folders/${folderId}/messages?skip=${skip}&top=${top}`),
  getMessage: (messageId: string) =>
    json<MessageDetail>(`/email/messages/${messageId}`),
  markRead: (messageId: string) =>
    fetch(BASE + `/email/messages/${messageId}/read`, { method: 'PATCH' }),
  send: (payload: SendEmailPayload) =>
    fetch(BASE + '/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
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

// Settings
export const settings = {
  listProviders: () => json<{ providers: ProviderConfig[] }>('/settings/providers'),
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
