<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'
import { useAuthStore } from '../stores/auth'
import { THEMES, saveTheme, currentTheme } from '../theme'
import RichTextEditor from './RichTextEditor.vue'

const logs = useLogsStore()
const activeTheme = ref(currentTheme())
function selectTheme(key: string) {
  activeTheme.value = key
  saveTheme(key) // applies locally and persists to the account
}
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.role === 'admin')

const props = defineProps<{ initialTab?: string }>()

type Tab = 'account' | 'providers' | 'mcp' | 'tools' | 'integrations' | 'agents' | 'appearance' | 'users' | 'system'
const ADMIN_TABS: Tab[] = ['providers', 'mcp', 'tools', 'users', 'system']

// A persisted window may restore with an admin tab (e.g. after the admin
// impersonates a member and the page reloads) — fall back to Account.
function clampTab(tab: Tab): Tab {
  return !isAdmin.value && ADMIN_TABS.includes(tab) ? 'account' : tab
}
const activeTab = ref<Tab>(clampTab((props.initialTab as Tab) ?? 'account'))

// ── Providers ─────────────────────────────────────────────────────────────────
const providers = ref<api.ProviderConfig[]>([])
const mcpServers = ref<api.McpServerConfig[]>([])
const newProvider = ref<api.ProviderConfig>({
  name: '', provider: 'anthropic', model_id: '', base_url: '', api_key: '',
})
const ollamaModels = ref<string[]>([])
const ollamaFetching = ref(false)
const ollamaFetchError = ref('')

async function fetchOllamaModels() {
  if (!newProvider.value.base_url) return
  ollamaFetching.value = true
  ollamaFetchError.value = ''
  ollamaModels.value = []
  logs.info('Ollama', `Fetching models from ${newProvider.value.base_url}`)
  try {
    const res = await api.settings.listOllamaModels(newProvider.value.base_url)
    ollamaModels.value = res.models
    logs.info('Ollama', `Found ${res.models.length} models: ${res.models.join(', ')}`)
    if (res.models.length > 0 && !newProvider.value.model_id) {
      newProvider.value.model_id = res.models[0]
    }
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Could not reach Ollama'
    ollamaFetchError.value = msg
    logs.error('Ollama', `Failed to fetch models: ${msg}`)
  } finally {
    ollamaFetching.value = false
  }
}

function onProviderTypeChange() {
  ollamaModels.value = []
  ollamaFetchError.value = ''
  newProvider.value.model_id = ''
  newProvider.value.base_url = ''
  newProvider.value.api_key = ''
}

onMounted(async () => {
  try {
    const [pRes, mRes] = await Promise.all([
      api.settings.listProviders(),
      api.settings.listMcpServers(),
    ])
    providers.value = pRes.providers
    mcpServers.value = mRes.mcp_servers
  } catch { /* members may lack some of these; tabs are role-gated anyway */ }
  await refreshMcpStatus()
  if (isAdmin.value) await loadTools()
  await loadTimezone()
  await loadIntegrations()
  await loadAllMailboxes()
  await loadSignatures()
})

async function saveProvider() {
  await api.settings.upsertProvider(newProvider.value)
  logs.info('Settings', `Saved provider "${newProvider.value.name}" (${newProvider.value.provider} / ${newProvider.value.model_id})`)
  providers.value = (await api.settings.listProviders()).providers
  newProvider.value = { name: '', provider: 'anthropic', model_id: '', base_url: '', api_key: '' }
}

async function deleteProvider(name: string) {
  await api.settings.deleteProvider(name)
  logs.info('Settings', `Deleted provider "${name}"`)
  providers.value = providers.value.filter((p) => p.name !== name)
}

// ── MCP servers ───────────────────────────────────────────────────────────────
const mcpStatuses = ref<api.McpServerStatus[]>([])
const newMcp = ref({ name: '', type: 'stdio' as 'stdio' | 'http', command: '', args: '', url: '' })
const mcpSaving = ref(false)
const mcpMsg = ref('')

function mcpStatusFor(name: string) {
  return mcpStatuses.value.find((s) => s.name === name)
}

async function refreshMcpStatus() {
  try {
    mcpStatuses.value = (await api.settings.mcpServerStatus()).statuses
  } catch {
    // Status is cosmetic — never break the settings panel over it.
  }
}

async function saveMcpServer() {
  mcpSaving.value = true
  mcpMsg.value = ''
  const cfg: api.McpServerConfig = {
    name: newMcp.value.name.trim(),
    transport:
      newMcp.value.type === 'stdio'
        ? { type: 'stdio', command: newMcp.value.command.trim(), args: newMcp.value.args.split(/\s+/).filter(Boolean) }
        : { type: 'http', url: newMcp.value.url.trim() },
  }
  try {
    // The backend connects before responding, so this reports live status.
    const res = await api.settings.upsertMcpServer(cfg)
    if (res.status.connected) {
      mcpMsg.value = `Connected — ${res.status.tool_count} tool${res.status.tool_count === 1 ? '' : 's'} available.`
      newMcp.value = { name: '', type: 'stdio', command: '', args: '', url: '' }
    } else {
      mcpMsg.value = res.status.error ?? 'Connection failed.'
    }
    mcpServers.value = (await api.settings.listMcpServers()).mcp_servers
    await refreshMcpStatus()
  } catch (e: unknown) {
    mcpMsg.value = e instanceof Error ? e.message : 'Save failed.'
  } finally {
    mcpSaving.value = false
  }
}

async function deleteMcpServer(name: string) {
  await api.settings.deleteMcpServer(name)
  mcpServers.value = mcpServers.value.filter((s) => s.name !== name)
  await refreshMcpStatus()
}

// ── Tool approval policies ────────────────────────────────────────────────────
const tools = ref<api.ToolInfo[]>([])
const toolsLoading = ref(false)

const toolGroups = computed(() => {
  const groups = new Map<string, api.ToolInfo[]>()
  for (const t of tools.value) {
    const list = groups.get(t.group) ?? []
    list.push(t)
    groups.set(t.group, list)
  }
  return [...groups.entries()]
})

async function loadTools() {
  toolsLoading.value = true
  try {
    tools.value = (await api.settings.listTools()).tools
  } catch {
    // Panel stays usable without the list.
  } finally {
    toolsLoading.value = false
  }
}

async function toggleToolPolicy(t: api.ToolInfo) {
  const policy = t.policy === 'ask' ? 'auto' : 'ask'
  t.policy = policy // optimistic
  await api.settings.setToolPolicy(t.name, policy)
}

// ── Users & invites (admin) ──────────────────────────────────────────────────
const userList = ref<api.UserAccount[]>([])
const inviteList = ref<api.Invite[]>([])
const inviteLabelInput = ref('')
const copiedCode = ref('')

// Redeemed invites vanish (the new account in the user list is the record);
// only pending, unexpired codes stay actionable.
const pendingInvites = computed(() =>
  inviteList.value.filter(i => !i.used_by && new Date(i.expires_at) > new Date()),
)

let usersPollTimer: number | null = null
watch(activeTab, (tab) => {
  if (usersPollTimer !== null) {
    window.clearInterval(usersPollTimer)
    usersPollTimer = null
  }
  if (tab === 'users' && isAdmin.value) {
    usersPollTimer = window.setInterval(loadUsers, 8000)
  }
})
onUnmounted(() => {
  if (usersPollTimer !== null) window.clearInterval(usersPollTimer)
})

function inviteLink(inv: api.Invite): string {
  return `${window.location.origin}/?invite=${inv.code}`
}

async function loadUsers() {
  if (!isAdmin.value) return
  try {
    userList.value = (await api.users.list()).users
    inviteList.value = (await api.invites.list()).invites
  } catch { /* surfaced by individual actions */ }
}

async function createInvite() {
  const res = await api.invites.create(inviteLabelInput.value.trim())
  inviteLabelInput.value = ''
  inviteList.value.unshift(res.invite)
  await copyInvite(res.invite)
}

async function copyInvite(inv: api.Invite) {
  try {
    await navigator.clipboard.writeText(inviteLink(inv))
    copiedCode.value = inv.code
    setTimeout(() => { if (copiedCode.value === inv.code) copiedCode.value = '' }, 2000)
  } catch { /* clipboard unavailable; the link is shown inline anyway */ }
}

async function revokeInvite(inv: api.Invite) {
  await api.invites.revoke(inv.code)
  inviteList.value = inviteList.value.filter(i => i.code !== inv.code)
}

async function setUserStatus(u: api.UserAccount, action: 'disable' | 'enable') {
  await (action === 'disable' ? api.users.disable(u.id) : api.users.enable(u.id))
  u.status = action === 'disable' ? 'disabled' : 'active'
}

async function deleteUser(u: api.UserAccount) {
  if (!confirm(`Delete ${u.username} and ALL their data? This can't be undone.`)) return
  await api.users.remove(u.id)
  userList.value = userList.value.filter(x => x.id !== u.id)
}

// ── Usage (admin) ─────────────────────────────────────────────────────────────
const usageDays = ref(30)
const usageRows = ref<api.UsageRow[]>([])

async function loadUsage() {
  try {
    const res = await api.usageSummary(usageDays.value)
    usageRows.value = res.usage
  } catch {
    usageRows.value = []
  }
  loadPrices()
}

const usageTotalCost = computed(() =>
  usageRows.value.reduce((sum, r) => sum + (r.cost ?? 0), 0),
)

/** "$0.0312" for small sums, "$12.46" once they grow — never "$0.00" for a priced row. */
function fmtCost(cost: number | null): string {
  if (cost == null) return '—'
  return '$' + (cost >= 0.1 ? cost.toFixed(2) : cost.toFixed(4))
}

// ── Model prices (admin) ──────────────────────────────────────────────────────
const priceRows = ref<api.ModelPrice[]>([])
const pricesMsg = ref('')
let pricesLoaded = false

async function loadPrices() {
  if (pricesLoaded) return
  try {
    priceRows.value = (await api.modelPrices.get()).prices
    pricesLoaded = true
  } catch {
    /* non-admin or transient — the section just stays empty */
  }
}

function addPriceRow() {
  priceRows.value.push({ model: '', prompt_per_mtok: 0, completion_per_mtok: 0 })
}

async function savePrices() {
  pricesMsg.value = ''
  try {
    priceRows.value = (await api.modelPrices.set(priceRows.value)).prices
    pricesMsg.value = 'Saved.'
    pricesLoaded = true
    // Re-price the visible usage table with the new rates.
    const res = await api.usageSummary(usageDays.value)
    usageRows.value = res.usage
  } catch (e: unknown) {
    pricesMsg.value = e instanceof Error ? e.message : 'Failed.'
  }
}

// ── Scheduled agents ──────────────────────────────────────────────────────────
const DAY_TOKENS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const
const agentsList = ref<api.ScheduledAgent[]>([])
const agentsError = ref('')
const agentsSaving = ref(false)
const agentRunning = ref<string | null>(null)

async function loadAgents() {
  try {
    const res = await api.scheduledAgents.list()
    agentsList.value = res.agents
  } catch (e: unknown) {
    agentsError.value = e instanceof Error ? e.message : 'Failed to load agents'
  }
}

function addAgent() {
  agentsList.value.push({
    id: '',
    name: '',
    time: '07:00',
    days: ['mon', 'tue', 'wed', 'thu', 'fri'],
    provider: '',
    instructions: '',
    enabled: true,
    quiet: false,
    last_run: '',
  })
}

function toggleAgentDay(a: api.ScheduledAgent, day: string) {
  const i = a.days.indexOf(day)
  if (i === -1) a.days.push(day)
  else a.days.splice(i, 1)
}

async function saveAgents() {
  agentsSaving.value = true
  agentsError.value = ''
  try {
    const res = await api.scheduledAgents.save(agentsList.value)
    agentsList.value = res.agents
    logs.info('Scheduler', `Saved ${res.agents.length} scheduled agent(s)`)
  } catch (e: unknown) {
    agentsError.value = e instanceof Error ? e.message : 'Failed to save agents'
  } finally {
    agentsSaving.value = false
  }
}

function removeAgent(i: number) {
  agentsList.value.splice(i, 1)
}

async function runAgentNow(a: api.ScheduledAgent) {
  if (!a.id) {
    agentsError.value = 'Save first, then run'
    return
  }
  agentRunning.value = a.id
  agentsError.value = ''
  try {
    await api.scheduledAgents.run(a.id)
    logs.info('Scheduler', `Ran '${a.name}' — output is in a new chat session`)
  } catch (e: unknown) {
    agentsError.value = e instanceof Error ? e.message : 'Run failed'
  } finally {
    agentRunning.value = null
  }
}

// ── Timezone ──────────────────────────────────────────────────────────────────
const timezone = ref('')
const timezones = ref<string[]>([])
const tzSaving = ref(false)
const tzMsg = ref('')

async function loadTimezone() {
  timezones.value = Intl.supportedValuesOf('timeZone')
  try {
    const res = await api.settings.getTimezone()
    timezone.value = res.timezone
    // First run: nothing saved yet. Adopt the browser's zone and persist it
    // immediately — a preselected-but-unsaved dropdown looks configured
    // while the backend silently stays on UTC.
    if (!res.configured) {
      timezone.value = Intl.DateTimeFormat().resolvedOptions().timeZone
      await saveTimezone()
    }
  } catch {
    timezone.value = Intl.DateTimeFormat().resolvedOptions().timeZone
  }
}

async function saveTimezone() {
  tzSaving.value = true
  tzMsg.value = ''
  try {
    await api.settings.setTimezone(timezone.value)
    tzMsg.value = 'Saved.'
  } catch (e: unknown) {
    tzMsg.value = e instanceof Error ? e.message : 'Save failed.'
  } finally {
    tzSaving.value = false
  }
}

// ── Integrations (helpdesk / phoneus / github / Microsoft 365) ────────────────
// Microsoft 365 is now a multi-instance integration like the others: each
// connected O365 account is a row, and its shared mailboxes + AI auto-sort are
// managed inside that instance's edit modal.
const callbackUri = computed(() => window.location.origin + '/api/integrations/email/callback')

const INTEGRATION_KINDS = [
  { kind: 'helpdesk' as const, label: 'Helpdesk', auth: 'login' as const, blurb: 'Tickets, replies, and time logging from chat' },
  { kind: 'phoneus' as const, label: 'PhoneUs', auth: 'login' as const, blurb: 'Customer balances, services, contacts, and SMS' },
  { kind: 'github' as const, label: 'GitHub', auth: 'token' as const, blurb: 'Read commits, files, and pull requests' },
  { kind: 'recipes' as const, label: 'Recipe Box', auth: 'login' as const, blurb: 'Find, create, and share recipes from chat' },
  { kind: 'microsoft' as const, label: 'Microsoft 365', auth: 'oauth' as const, blurb: 'Email, calendar, shared mailboxes, and AI auto-sort' },
]
function kindMeta(kind: string) {
  return INTEGRATION_KINDS.find((k) => k.kind === kind) ?? INTEGRATION_KINDS[0]
}

const integrations = ref<api.IntegrationView[]>([])
// The modal's working copy; `id` set = editing, else adding.
type IntegrationDraft = api.IntegrationInput & { id?: string; connected?: boolean }
const intDraft = ref<IntegrationDraft | null>(null)
// Which tab of the Microsoft 365 modal is showing.
const modalTab = ref<'account' | 'shared' | 'autosort'>('account')
const intMsg = ref('')
const intSaving = ref(false)

async function loadIntegrations() {
  try { integrations.value = (await api.integrations.list()).integrations } catch { /* none */ }
}

function openAddIntegration() {
  intMsg.value = ''
  modalTab.value = 'account'
  intDraft.value = {
    kind: 'helpdesk', name: '', base_url: '', email: '', password: '', token: '',
    default_owner: '', tenant_id: '', client_id: '', client_secret: '', is_default: false,
  }
  sharedMailboxes.value = []
  catConfig.value = { interval_secs: 300, batch_limit: 25, tasks: [] }
}

async function openEditIntegration(row: api.IntegrationView) {
  intMsg.value = ''
  modalTab.value = 'account'
  intDraft.value = {
    id: row.id, kind: row.kind, name: row.name, is_default: row.is_default,
    base_url: row.base_url, email: row.account, password: '', token: '', default_owner: row.default_owner,
    tenant_id: row.tenant_id, client_id: row.client_id, client_secret: '', connected: row.connected,
  }
  if (row.kind === 'microsoft') await loadMicrosoftSubResources(row.id)
}

// Load a Microsoft instance's shared mailboxes + AI-sort config into the modal.
async function loadMicrosoftSubResources(id: string) {
  sharedMailboxes.value = []
  catConfig.value = { interval_secs: 300, batch_limit: 25, tasks: [] }
  catMailbox.value = ''
  try { sharedMailboxes.value = (await api.integrations.listShared(id)).mailboxes } catch { /* not connected */ }
  try { catConfig.value = await api.emailCategorizer.getConfig(id) } catch { /* keep defaults */ }
  ensureCatTasks()
}

async function saveIntegration() {
  const d = intDraft.value
  if (!d || !d.name.trim()) { intMsg.value = 'Name is required.'; return }
  intSaving.value = true
  intMsg.value = ''
  try {
    if (d.id) {
      await api.integrations.update(d.id, d)
      await loadIntegrations()
      // Microsoft stays open so the user can connect / manage mailboxes + AI sort.
      if (d.kind === 'microsoft') intMsg.value = 'Saved.'
      else intDraft.value = null
    } else {
      const created = await api.integrations.create(d)
      await loadIntegrations()
      if (d.kind === 'microsoft') {
        // Switch to edit mode on the new instance so connect/shared/AI-sort appear.
        await openEditIntegration(created)
        intMsg.value = 'Saved — now connect the mailbox below.'
      } else {
        intDraft.value = null
      }
    }
  } catch (e: unknown) {
    intMsg.value = e instanceof Error ? e.message : 'Save failed.'
  } finally {
    intSaving.value = false
  }
}

async function removeIntegration(row: api.IntegrationView) {
  if (!confirm(`Remove ${kindMeta(row.kind).label} "${row.name}"?`)) return
  await api.integrations.remove(row.id)
  await loadIntegrations()
}

async function makeDefaultIntegration(row: api.IntegrationView) {
  await api.integrations.setDefault(row.id)
  await loadIntegrations()
}

// Microsoft 365 connection actions (operate on the instance open in the modal).
function connectMicrosoft() {
  if (intDraft.value?.id) window.location.href = api.integrations.connectUrl(intDraft.value.id)
}
async function disconnectMicrosoft() {
  if (!intDraft.value?.id) return
  await api.integrations.disconnect(intDraft.value.id)
  intDraft.value.connected = false
  sharedMailboxes.value = []
  await loadIntegrations()
}

// ── Shared mailboxes (scoped to the Microsoft instance open in the modal) ─────
const sharedMailboxes = ref<api.SharedMailbox[]>([])
const sharedForm = ref({ address: '', name: '' })
const sharedMsg = ref('')
const sharedSaving = ref(false)

async function addShared() {
  const id = intDraft.value?.id
  const address = sharedForm.value.address.trim()
  if (!id || !address) return
  sharedSaving.value = true
  sharedMsg.value = ''
  try {
    const res = await api.integrations.addShared(id, address, sharedForm.value.name.trim() || undefined)
    sharedMailboxes.value = res.mailboxes
    ensureCatTasks() // give the new mailbox an auto-sort row
    sharedForm.value = { address: '', name: '' }
    sharedMsg.value = 'Mailbox added.'
  } catch (e: unknown) {
    sharedMsg.value = e instanceof Error ? e.message : 'Could not add mailbox.'
  } finally {
    sharedSaving.value = false
  }
}

async function removeShared(address: string) {
  const id = intDraft.value?.id
  if (!id) return
  await api.integrations.removeShared(id, address)
  sharedMailboxes.value = sharedMailboxes.value.filter(m => m.address !== address)
  // Drop its auto-sort task so we don't persist a row for a gone mailbox.
  catConfig.value.tasks = catConfig.value.tasks.filter(t => t.mailbox !== address)
  // The AI-sort tab can't be left pointing at a removed mailbox.
  if (catMailbox.value === address) catMailbox.value = ''
}

// ── Email signatures ────────────────────────────────────────────────────────────
// Per-mailbox HTML signatures ('' = own mailbox); the email composer inserts
// the active mailbox's signature into new messages and replies. The picker spans
// every connected account's own + shared mailboxes (signatures are keyed by
// address, applied whenever that mailbox is selected in the Email window).
const signatures = ref<Record<string, string>>({})
const sigMailbox = ref('')
const sigHtml = ref('')
const sigMsg = ref('')
const sigSaving = ref(false)

const anyMicrosoftConnected = computed(() =>
  integrations.value.some(i => i.kind === 'microsoft' && i.connected),
)

// Aggregate of every connected account's own + shared mailboxes, for the
// signature picker (which is per-address, not per-account).
const allMailboxes = ref<{ address: string; label: string }[]>([{ address: '', label: 'My mailbox' }])
async function loadAllMailboxes() {
  const out = [{ address: '', label: 'My mailbox' }]
  for (const i of integrations.value.filter(x => x.kind === 'microsoft' && x.connected)) {
    try {
      const { mailboxes } = await api.integrations.listShared(i.id)
      for (const m of mailboxes) {
        if (!out.some(o => o.address === m.address)) out.push({ address: m.address, label: m.name || m.address })
      }
    } catch { /* skip this account */ }
  }
  allMailboxes.value = out
}

async function loadSignatures() {
  try {
    signatures.value = await api.email.getSignatures()
    sigHtml.value = signatures.value[sigMailbox.value] ?? ''
  } catch { /* none saved yet */ }
}

function switchSigMailbox(address: string) {
  // Keep unsaved edits for the mailbox we're leaving so flipping back doesn't lose them.
  signatures.value[sigMailbox.value] = sigHtml.value
  sigMailbox.value = address
  sigHtml.value = signatures.value[address] ?? ''
}

async function saveSignatures() {
  sigSaving.value = true
  sigMsg.value = ''
  signatures.value[sigMailbox.value] = sigHtml.value
  try {
    const res = await api.email.saveSignatures(signatures.value)
    if (!res.ok) throw new Error(`${res.status}`)
    sigMsg.value = 'Saved.'
    logs.info('Settings', 'Email signatures saved')
  } catch (e: unknown) {
    sigMsg.value = e instanceof Error ? `Save failed: ${e.message}` : 'Save failed.'
  } finally {
    sigSaving.value = false
  }
}

// ── Email auto-sort (categorizer) ───────────────────────────────────────────────
const catConfig = ref<api.CategorizerConfig>({
  interval_secs: 300, batch_limit: 25, tasks: [],
})
const catMsg = ref('')
const catSaving = ref(false)
// The mailbox address currently running a manual sort, or null.
const catRunning = ref<string | null>(null)
// Which mailbox the AI-sort tab is configuring ('' = own mailbox).
const catMailbox = ref('')

// One auto-sort task per mailbox: the own mailbox ('') plus each shared one.
const mailboxRows = computed(() => [
  { address: '', label: 'My mailbox' },
  ...sharedMailboxes.value.map(m => ({ address: m.address, label: m.name || m.address })),
])
const anyCatEnabled = computed(() => catConfig.value.tasks.some(t => t.enabled))

// Make sure every current mailbox has a task row (existing settings preserved).
function ensureCatTasks() {
  for (const row of mailboxRows.value) {
    if (!catConfig.value.tasks.some(t => t.mailbox === row.address)) {
      catConfig.value.tasks.push({ mailbox: row.address, enabled: false, provider: '', instructions: '' })
    }
  }
}
function catTaskFor(address: string): api.CategorizerTask {
  return catConfig.value.tasks.find(t => t.mailbox === address)!
}
// Guarantee a task exists for every row before it renders (own mailbox now,
// shared ones as they load), so catTaskFor is always safe.
watch(mailboxRows, ensureCatTasks, { immediate: true })

const CATEGORY_FOLDERS = [
  { label: 'Promotions', desc: 'marketing, newsletters, offers' },
  { label: 'Invoices', desc: 'bills, receipts, statements' },
  { label: 'Notifications', desc: 'alerts, Sentry errors, CI, monitoring' },
  { label: 'Deliveries', desc: 'shipping & order tracking' },
]

async function saveCategorizer() {
  const id = intDraft.value?.id
  if (!id) return
  catSaving.value = true
  catMsg.value = ''
  try {
    catConfig.value = await api.emailCategorizer.saveConfig(catConfig.value, id)
    ensureCatTasks()
    catMsg.value = 'Saved.'
    const on = catConfig.value.tasks.filter(t => t.enabled).length
    logs.info('Categorizer', `Auto-sort saved — ${on} mailbox${on === 1 ? '' : 'es'} on (every ${catConfig.value.interval_secs}s)`)
  } catch (e: unknown) {
    catMsg.value = e instanceof Error ? e.message : 'Save failed.'
  } finally {
    catSaving.value = false
  }
}

async function runCategorizer(mailbox: string) {
  const id = intDraft.value?.id
  if (!id) return
  catRunning.value = mailbox
  catMsg.value = ''
  logs.info('Categorizer', `Manual run started (${mailbox || 'my mailbox'})`)
  try {
    const s = await api.emailCategorizer.runNow(id, mailbox || undefined)
    catMsg.value = s.message
    logs.info('Categorizer', s.message)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : 'Run failed.'
    catMsg.value = msg
    logs.error('Categorizer', `Run failed: ${msg}`)
  } finally {
    catRunning.value = null
  }
}

// ── Account ───────────────────────────────────────────────────────────────────
const passwordForm = ref({ current: '', next: '', confirm: '' })
const passwordMsg = ref('')
const twoFactorEnabled = ref(false)
const accountMsg = ref('')

async function changePassword() {
  if (passwordForm.value.next !== passwordForm.value.confirm) {
    passwordMsg.value = 'New passwords do not match.'
    return
  }
  try {
    await api.auth.changePassword(passwordForm.value.current, passwordForm.value.next)
    passwordMsg.value = 'Password updated.'
    passwordForm.value = { current: '', next: '', confirm: '' }
  } catch (e: unknown) {
    passwordMsg.value = e instanceof Error ? e.message : 'Failed.'
  }
}

// ── Two-factor (TOTP) ──
const twoFactorRecoveryLeft = ref(0)
// Enrollment in progress: QR + secret shown, waiting for the first code.
const totpSetup = ref<{ secret: string; qr_svg: string } | null>(null)
const totpEnrollCode = ref('')
// Shown exactly once after enabling; dismissed by the user.
const recoveryCodes = ref<string[] | null>(null)
const totpDisablePassword = ref('')
const totpDisabling = ref(false)

onMounted(async () => {
  try {
    const s = await api.auth.twoFactor.status()
    twoFactorEnabled.value = s.enabled
    twoFactorRecoveryLeft.value = s.recovery_codes_left
  } catch {
    /* transient — section just shows "Not configured" */
  }
})

async function startTwoFactorSetup() {
  accountMsg.value = ''
  try {
    const s = await api.auth.twoFactor.setup()
    totpSetup.value = { secret: s.secret, qr_svg: s.qr_svg }
    totpEnrollCode.value = ''
  } catch (e: unknown) {
    accountMsg.value = e instanceof Error ? e.message : 'Failed.'
  }
}

async function confirmTwoFactor() {
  accountMsg.value = ''
  try {
    const r = await api.auth.twoFactor.enable(totpEnrollCode.value.trim())
    recoveryCodes.value = r.recovery_codes
    twoFactorEnabled.value = true
    twoFactorRecoveryLeft.value = r.recovery_codes.length
    totpSetup.value = null
    totpEnrollCode.value = ''
  } catch (e: unknown) {
    accountMsg.value = e instanceof Error ? e.message : 'Failed.'
  }
}

async function disableTwoFactor() {
  accountMsg.value = ''
  totpDisabling.value = true
  try {
    await api.auth.twoFactor.disable(totpDisablePassword.value)
    twoFactorEnabled.value = false
    twoFactorRecoveryLeft.value = 0
    totpDisablePassword.value = ''
    recoveryCodes.value = null
    accountMsg.value = 'Two-factor authentication disabled.'
  } catch (e: unknown) {
    accountMsg.value = e instanceof Error ? e.message : 'Failed.'
  } finally {
    totpDisabling.value = false
  }
}

function copyRecoveryCodes() {
  if (recoveryCodes.value) {
    void navigator.clipboard.writeText(recoveryCodes.value.join('\n'))
    accountMsg.value = 'Recovery codes copied.'
  }
}

// ── Browser notifications (web push) ──
const pushSupported =
  typeof window !== 'undefined' && 'serviceWorker' in navigator && 'PushManager' in window
const pushEnabled = ref(false)
const pushMsg = ref('')

async function currentPushSubscription(): Promise<PushSubscription | null> {
  if (!pushSupported) return null
  const reg = await navigator.serviceWorker.getRegistration()
  return (await reg?.pushManager.getSubscription()) ?? null
}

onMounted(async () => {
  pushEnabled.value = !!(await currentPushSubscription())
})

/** base64url → bytes, the shape pushManager.subscribe wants the VAPID key in. */
function b64urlToBytes(b64url: string): Uint8Array<ArrayBuffer> {
  const pad = '='.repeat((4 - (b64url.length % 4)) % 4)
  const raw = atob((b64url + pad).replace(/-/g, '+').replace(/_/g, '/'))
  // Explicit ArrayBuffer backing so TS accepts it as a BufferSource.
  const bytes = new Uint8Array(new ArrayBuffer(raw.length))
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i)
  return bytes
}

async function enablePush() {
  pushMsg.value = ''
  // Diagnose the common silent failures before asking — in these states the
  // permission prompt never appears and requestPermission resolves "denied".
  if (!window.isSecureContext) {
    pushMsg.value = 'Notifications need HTTPS — open the app via its https:// address.'
    return
  }
  if (Notification.permission === 'denied') {
    pushMsg.value =
      'Notifications are blocked for this site in the browser. Allow them via the padlock/site-settings menu, then try again. (Brave also needs "Use Google services for push messaging" enabled in brave://settings/privacy.)'
    return
  }
  try {
    const perm = await Notification.requestPermission()
    if (perm !== 'granted') {
      pushMsg.value =
        perm === 'denied'
          ? 'The browser blocked the permission prompt — check the site-settings (padlock) menu. On Brave, also enable "Use Google services for push messaging" in brave://settings/privacy.'
          : 'Permission prompt dismissed — click Enable again to retry.'
      return
    }
    const reg = await navigator.serviceWorker.register('/sw.js')
    const { public_key } = await api.push.vapidKey()
    const sub = await reg.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: b64urlToBytes(public_key),
    })
    await api.push.register(JSON.stringify(sub))
    pushEnabled.value = true
    pushMsg.value = 'This browser will now receive notifications.'
  } catch (e: unknown) {
    pushMsg.value = e instanceof Error ? e.message : 'Failed to enable notifications.'
  }
}

async function disablePush() {
  try {
    // Unsubscribing kills the endpoint; the server prunes its row on the
    // next send, so no explicit deregister call is needed.
    const sub = await currentPushSubscription()
    if (sub) await sub.unsubscribe()
    pushEnabled.value = false
    pushMsg.value = 'Notifications disabled in this browser.'
  } catch (e: unknown) {
    pushMsg.value = e instanceof Error ? e.message : 'Failed.'
  }
}

async function logout() {
  await authStore.logout()
  // No /login route — clearing `authenticated` drops the app back to AuthGate.
}
</script>

<template>
  <div class="flex flex-row h-full min-h-[420px]">
    <!-- Vertical nav -->
    <nav class="w-[156px] min-w-[156px] bg-[var(--c-141414)] border-r border-[var(--c-252525)] flex flex-col p-2 gap-0.5 overflow-y-auto">
      <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'account' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'account'">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>
        </svg>
        Account
      </button>
      <button v-if="isAdmin" :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'providers' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'providers'">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
        </svg>
        Providers
      </button>
      <button v-if="isAdmin" :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'mcp' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'mcp'">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>
        </svg>
        MCP Servers
      </button>
      <button v-if="isAdmin" :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'tools' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'tools'; loadTools()">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2-2 2.6-2.6z"/>
        </svg>
        Tools
      </button>
      <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'integrations' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'integrations'">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>
        </svg>
        Integrations
      </button>
      <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'agents' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'agents'; loadAgents()">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>
        </svg>
        Agents
      </button>
      <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'appearance' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'appearance'">
        <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
        Appearance
      </button>

      <div v-if="isAdmin" class="flex flex-col gap-0.5 mt-2 pt-2 border-t border-[var(--c-252525)]">
        <span class="text-[0.65rem] font-semibold uppercase tracking-[0.08em] text-[var(--c-484848)] px-3 pt-1 pb-1.5 pointer-events-none">Admin</span>
        <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'users' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'users'; loadUsers()">
          <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>
          </svg>
          Users
        </button>
        <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'system' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'system'; loadUsage()">
          <svg class="shrink-0" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/>
          </svg>
          System
        </button>
      </div>
    </nav>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto min-w-0">

      <!-- Account -->
      <div v-if="activeTab === 'account'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">Account</h2>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Change password</h3>
          <form class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]" @submit.prevent="changePassword">
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Current password <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="passwordForm.current" type="password" autocomplete="current-password" /></label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">New password <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="passwordForm.next" type="password" autocomplete="new-password" /></label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Confirm new password <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="passwordForm.confirm" type="password" autocomplete="new-password" /></label>
            <p v-if="passwordMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ passwordMsg }}</p>
            <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)]">Update password</button>
          </form>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Two-factor authentication</h3>
          <div class="flex flex-col gap-3 bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg px-3.5 py-3">
            <div class="flex items-center justify-between gap-4">
              <div>
                <div class="text-[0.8125rem] text-[var(--c-d0d0d0)]">Authenticator app (TOTP)</div>
                <div class="text-[0.75rem] text-[var(--c-585858)] mt-[0.1rem]">
                  {{ twoFactorEnabled ? `Active — ${twoFactorRecoveryLeft} recovery code${twoFactorRecoveryLeft === 1 ? '' : 's'} left` : 'Not configured' }}
                </div>
              </div>
              <button v-if="!twoFactorEnabled && !totpSetup" class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-282828)]" @click="startTwoFactorSetup">
                Enable
              </button>
            </div>

            <!-- Enrollment: scan, then confirm with the first code. -->
            <div v-if="totpSetup" class="flex flex-col gap-2 border-t border-[var(--c-1e1e1e)] pt-3">
              <p class="text-[0.775rem] text-[var(--c-888888)]">Scan with your authenticator app (Google Authenticator, Aegis, 1Password…), then enter the 6-digit code it shows.</p>
              <!-- Backend-generated QR SVG of the otpauth URL — our own markup, not email content. -->
              <div class="self-start rounded bg-white p-2" v-html="totpSetup.qr_svg"></div>
              <p class="text-[0.7rem] text-[var(--c-585858)]">Can't scan? Enter this key manually: <code class="text-[var(--c-c0c0c0)] select-all">{{ totpSetup.secret }}</code></p>
              <div class="flex items-center gap-2">
                <input v-model="totpEnrollCode" type="text" inputmode="numeric" autocomplete="one-time-code" placeholder="123456" maxlength="6"
                  class="w-28 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.85rem] font-[inherit] tracking-[0.2em] focus:outline-none focus:border-[var(--c-3a6adf)]"
                  @keyup.enter="confirmTwoFactor" />
                <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:bg-[var(--c-254880)]" @click="confirmTwoFactor">Turn on</button>
                <button class="bg-none border-none text-[var(--c-585858)] cursor-pointer text-[0.8rem] font-[inherit] hover:text-[var(--c-c0c0c0)]" @click="totpSetup = null">Cancel</button>
              </div>
            </div>

            <!-- Recovery codes: shown exactly once after enabling. -->
            <div v-if="recoveryCodes" class="flex flex-col gap-2 border-t border-[var(--c-1e1e1e)] pt-3">
              <p class="text-[0.775rem] text-[var(--c-e0b060)]">Save these recovery codes somewhere safe — they are shown only once. Each one signs you in a single time if you lose your authenticator.</p>
              <div class="grid grid-cols-2 gap-x-6 gap-y-1 self-start font-mono text-[0.8rem] text-[var(--c-c0c0c0)] bg-surface border border-raised rounded px-3 py-2 select-all">
                <span v-for="c in recoveryCodes" :key="c">{{ c }}</span>
              </div>
              <div class="flex items-center gap-2">
                <button class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:bg-[var(--c-282828)]" @click="copyRecoveryCodes">Copy all</button>
                <button class="bg-none border-none text-[var(--c-585858)] cursor-pointer text-[0.8rem] font-[inherit] hover:text-[var(--c-c0c0c0)]" @click="recoveryCodes = null">I've saved them</button>
              </div>
            </div>

            <!-- Disable: password-gated (also the lost-authenticator path,
                 after signing in with a recovery code). -->
            <div v-if="twoFactorEnabled" class="flex items-center gap-2 border-t border-[var(--c-1e1e1e)] pt-3">
              <input v-model="totpDisablePassword" type="password" autocomplete="current-password" placeholder="Password"
                class="w-44 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
              <button :disabled="totpDisabling || !totpDisablePassword" class="bg-[var(--c-2a1010)] text-[var(--c-ff7070)] border border-[var(--c-4a1a1a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:bg-[var(--c-3a1515)] disabled:opacity-50 disabled:cursor-default" @click="disableTwoFactor">
                {{ totpDisabling ? 'Disabling…' : 'Disable 2FA' }}
              </button>
            </div>
          </div>
          <p v-if="accountMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ accountMsg }}</p>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Browser notifications</h3>
          <div class="flex items-center justify-between bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg px-3.5 py-3 gap-4">
            <div>
              <div class="text-[0.8125rem] text-[var(--c-d0d0d0)]">Web push on this browser</div>
              <div class="text-[0.75rem] text-[var(--c-585858)] mt-[0.1rem]">{{ !pushSupported ? 'Not supported by this browser' : pushEnabled ? 'Active — job results, approvals, and flagged mail notify here' : 'Not enabled' }}</div>
            </div>
            <button v-if="pushSupported" class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-282828)]" @click="pushEnabled ? disablePush() : enablePush()">
              {{ pushEnabled ? 'Disable' : 'Enable' }}
            </button>
          </div>
          <p v-if="pushMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ pushMsg }}</p>
        </section>

        <section v-if="anyMicrosoftConnected" class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Email signature</h3>
          <div class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]">
            <label v-if="allMailboxes.length > 1" class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Mailbox
              <select
                class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] self-start"
                :value="sigMailbox"
                @change="switchSigMailbox(($event.target as HTMLSelectElement).value)"
              >
                <option v-for="row in allMailboxes" :key="row.address" :value="row.address">{{ row.label }}</option>
              </select>
            </label>
            <RichTextEditor v-model="sigHtml" min-height="100px" placeholder="Your signature — formatting and pasted images are kept." />
            <p class="text-[0.75rem] text-[var(--c-585858)]">Appended to new messages and replies sent from this mailbox. Paste from Outlook/Word to keep an existing signature's layout.</p>
            <div class="flex items-center gap-2">
              <button type="button" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)] disabled:opacity-50" :disabled="sigSaving" @click="saveSignatures">{{ sigSaving ? 'Saving…' : 'Save signature' }}</button>
              <span v-if="sigMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ sigMsg }}</span>
            </div>
          </div>
        </section>

        <section class="flex flex-col gap-3 border-t border-[var(--c-222222)] pt-5">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Danger zone</h3>
          <button class="bg-[var(--c-2a1010)] text-[var(--c-ff7070)] border border-[var(--c-4a1a1a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-3a1515)]" @click="logout">Sign out</button>
        </section>
      </div>

      <!-- Providers -->
      <div v-else-if="activeTab === 'providers'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">Model Providers</h2>

        <section class="flex flex-col gap-3">
          <ul v-if="providers.length" class="list-none flex flex-col gap-1.5">
            <li v-for="p in providers" :key="p.name" class="flex justify-between items-center bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-md px-3 py-2 gap-4">
              <div class="flex flex-col gap-[0.1rem] min-w-0">
                <span class="text-[0.8125rem] text-[var(--c-d0d0d0)]">{{ p.name }}</span>
                <span class="text-[0.75rem] text-[var(--c-585858)]">{{ p.provider }} · {{ p.model_id }}</span>
              </div>
              <button class="bg-[var(--c-1e1010)] text-[var(--c-a06060)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] shrink-0 hover:bg-[var(--c-2a1515)] hover:text-[var(--c-d08080)]" @click="deleteProvider(p.name)">Remove</button>
            </li>
          </ul>
          <p v-else class="text-[var(--c-484848)] text-[0.8125rem]">No providers configured.</p>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Add / update provider</h3>
          <form class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]" @submit.prevent="saveProvider">
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Name <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.name" placeholder="e.g. my-ollama" required /></label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Provider
              <select class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.provider" @change="onProviderTypeChange">
                <option>anthropic</option>
                <option>openai</option>
                <option>ollama</option>
                <option>openai_compatible</option>
                <option>gemini</option>
                <option>groq</option>
                <option>deepseek</option>
              </select>
            </label>

            <!-- Ollama: URL + discover -->
            <template v-if="newProvider.provider === 'ollama'">
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
                Ollama URL
                <div class="flex gap-1.5">
                  <input class="flex-1 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.base_url" placeholder="http://192.168.1.x:11434" />
                  <button type="button" class="bg-[var(--c-1a1a2a)] text-[var(--c-7090d0)] border border-[var(--c-2a2a4a)] rounded px-[0.6rem] py-1.5 cursor-pointer text-[0.775rem] font-[inherit] whitespace-nowrap transition-[background] duration-[120ms] shrink-0 hover:not-disabled:bg-[var(--c-222240)] disabled:opacity-40 disabled:cursor-default" :disabled="!newProvider.base_url || ollamaFetching" @click="fetchOllamaModels">
                    {{ ollamaFetching ? '…' : 'Fetch models' }}
                  </button>
                </div>
              </label>
              <p v-if="ollamaFetchError" class="text-[0.775rem] text-[var(--c-c06060)]">{{ ollamaFetchError }}</p>
              <label v-if="ollamaModels.length" class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
                Model
                <select class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.model_id">
                  <option v-for="m in ollamaModels" :key="m" :value="m">{{ m }}</option>
                </select>
              </label>
              <label v-else-if="!ollamaFetching" class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
                Model ID
                <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.model_id" placeholder="Fetch models above, or type manually" />
              </label>
            </template>

            <!-- All other providers -->
            <template v-else>
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Model ID <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.model_id" required /></label>
              <label v-if="newProvider.provider === 'openai_compatible'" class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
                Base URL <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.base_url" placeholder="https://…/v1" />
              </label>
              <label v-if="newProvider.provider !== 'ollama'" class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
                API Key <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newProvider.api_key" type="password" />
              </label>
            </template>

            <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)]" :disabled="!newProvider.name || !newProvider.model_id">Save</button>
          </form>
        </section>
      </div>

      <!-- MCP Servers -->
      <div v-else-if="activeTab === 'mcp'" class="px-6 py-5 flex flex-col gap-6">
        <div class="flex items-center justify-between">
          <h2 class="text-[0.9375rem] font-semibold text-fg">MCP Servers</h2>
          <button class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-282828)]" @click="refreshMcpStatus">Refresh</button>
        </div>

        <section class="flex flex-col gap-3">
          <ul v-if="mcpServers.length" class="list-none flex flex-col gap-1.5">
            <li v-for="s in mcpServers" :key="s.name" class="flex justify-between items-center bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-md px-3 py-2 gap-4">
              <div class="flex flex-col gap-[0.1rem] min-w-0">
                <span class="text-[0.8125rem] text-[var(--c-d0d0d0)] flex items-center gap-1.5">
                  <span
                    class="inline-block w-2 h-2 rounded-full shrink-0"
                    :class="mcpStatusFor(s.name)?.connected ? 'bg-[var(--c-4caf6e)]' : 'bg-[var(--c-c05050)]'"
                  ></span>
                  {{ s.name }}
                </span>
                <span class="text-[0.75rem] text-[var(--c-585858)]">
                  {{ s.transport.type === 'stdio' ? `stdio · ${s.transport.command} ${s.transport.args.join(' ')}` : `http · ${s.transport.url}` }}
                </span>
                <span v-if="mcpStatusFor(s.name)?.connected" class="text-[0.75rem] text-[var(--c-4caf6e)]">
                  {{ mcpStatusFor(s.name)?.tool_count }} tool{{ mcpStatusFor(s.name)?.tool_count === 1 ? '' : 's' }}
                </span>
                <span v-else-if="mcpStatusFor(s.name)?.error" class="text-[0.75rem] text-[var(--c-c06060)] break-all">
                  {{ mcpStatusFor(s.name)?.error }}
                </span>
              </div>
              <button class="bg-[var(--c-1e1010)] text-[var(--c-a06060)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] shrink-0 hover:bg-[var(--c-2a1515)] hover:text-[var(--c-d08080)]" @click="deleteMcpServer(s.name)">Remove</button>
            </li>
          </ul>
          <p v-else class="text-[var(--c-484848)] text-[0.8125rem]">No MCP servers configured.</p>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Add / update server</h3>
          <form class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]" @submit.prevent="saveMcpServer">
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Name <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newMcp.name" placeholder="e.g. filesystem" required /></label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Transport
              <select class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newMcp.type">
                <option value="stdio">stdio (local command)</option>
                <option value="http">http (remote server)</option>
              </select>
            </label>

            <template v-if="newMcp.type === 'stdio'">
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Command <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newMcp.command" placeholder="e.g. npx" required /></label>
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Arguments <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newMcp.args" placeholder="e.g. -y @modelcontextprotocol/server-everything" /></label>
            </template>
            <template v-else>
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">URL <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="newMcp.url" placeholder="https://example.com/mcp" required /></label>
            </template>

            <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)] disabled:opacity-40 disabled:cursor-default" :disabled="mcpSaving || !newMcp.name || (newMcp.type === 'stdio' ? !newMcp.command : !newMcp.url)">
              {{ mcpSaving ? 'Connecting…' : 'Save & connect' }}
            </button>
            <p v-if="mcpMsg" class="text-[0.775rem]" :class="mcpMsg.startsWith('Connected') ? 'text-[var(--c-4caf6e)]' : 'text-[var(--c-c06060)]'">{{ mcpMsg }}</p>
          </form>
          <p class="text-[0.75rem] text-[var(--c-585858)]">Tools from connected servers are offered to the model in every chat. By default they run without asking — flag individual tools as "ask first" under <b>Settings → Tools</b>.</p>
        </section>
      </div>

      <!-- Tools (approval policies) -->
      <div v-else-if="activeTab === 'tools'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">Tools</h2>
        <p class="text-[0.775rem] text-[var(--c-787878)] -mt-3">Tools marked <b>Ask first</b> pause the chat and wait for your approval before running. Everything else runs automatically.</p>

        <div v-if="toolsLoading && tools.length === 0" class="text-[var(--c-484848)] text-[0.8125rem]">Loading…</div>
        <p v-else-if="tools.length === 0" class="text-[var(--c-484848)] text-[0.8125rem]">No tools available.</p>

        <section v-for="[group, groupTools] in toolGroups" :key="group" class="flex flex-col gap-2">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">{{ group }}</h3>
          <ul class="list-none flex flex-col gap-1">
            <li v-for="t in groupTools" :key="t.name" class="flex items-center justify-between bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-md px-3 py-2 gap-4">
              <div class="flex flex-col gap-[0.1rem] min-w-0">
                <span class="text-[0.8125rem] text-[var(--c-d0d0d0)] flex items-center gap-2">
                  {{ t.name }}
                  <span v-if="t.suggest_ask && t.policy !== 'ask'" class="text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border text-[var(--c-e0b060)] border-[var(--c-e0b06055)]" title="This tool reports that it modifies external state">ask suggested</span>
                </span>
                <span class="text-[0.75rem] text-[var(--c-585858)] break-words">{{ t.description }}</span>
              </div>
              <label class="flex items-center gap-1.5 shrink-0 cursor-pointer text-[0.75rem] select-none" :class="t.policy === 'ask' ? 'text-[var(--c-e0b060)]' : 'text-[var(--c-585858)]'">
                <input type="checkbox" class="accent-[var(--c-e0b060)] cursor-pointer" :checked="t.policy === 'ask'" @change="toggleToolPolicy(t)" />
                Ask first
              </label>
            </li>
          </ul>
        </section>
      </div>

      <!-- Integrations -->
      <div v-else-if="activeTab === 'integrations'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">Integrations</h2>

        <!-- Integrations (helpdesk / phoneus / github) — multiple named instances -->
        <section class="bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg p-3.5 flex flex-col gap-3">
          <div class="flex items-center justify-between gap-4">
            <div>
              <div class="text-[0.8125rem] text-[var(--c-d0d0d0)] font-medium">Integrations</div>
              <div class="text-[0.72rem] text-[var(--c-585858)]">Connect one or more Microsoft 365, Helpdesk, PhoneUs, or GitHub accounts. The AI uses the default of each kind, or asks which when several exist. Open a Microsoft 365 entry to manage its shared mailboxes and AI auto-sort.</div>
            </div>
            <button class="shrink-0 flex items-center gap-[0.35rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-254880)]" @click="openAddIntegration">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
              Add integration
            </button>
          </div>

          <p v-if="integrations.length === 0" class="text-[0.775rem] text-[var(--c-585858)]">No integrations yet — add a Microsoft 365, Helpdesk, PhoneUs, or GitHub connection.</p>
          <div v-else class="flex flex-col gap-1.5">
            <div v-for="row in integrations" :key="row.id" class="flex items-center gap-3 bg-[var(--c-0d0d0d)] border border-[var(--c-1e1e1e)] rounded px-3 py-2">
              <span class="text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border text-[var(--c-7ab0ff)] border-[var(--c-2a4a8a)]">{{ kindMeta(row.kind).label }}</span>
              <div class="flex-1 min-w-0">
                <div class="text-[0.8125rem] text-fg truncate">
                  {{ row.name }}
                  <span v-if="row.is_default" class="ml-1 text-[0.58rem] uppercase tracking-[0.04em] text-[var(--c-4caf6e)] border border-[var(--c-1a4a2e)] rounded px-1 py-[0.05rem]">default</span>
                </div>
                <div class="text-[0.68rem] truncate flex items-center gap-1.5">
                  <template v-if="row.kind === 'microsoft'">
                    <span v-if="row.connected" class="inline-flex items-center gap-1 text-success"><span class="w-1.5 h-1.5 rounded-full bg-success shrink-0"></span>{{ row.account || 'Connected' }}</span>
                    <span v-else class="text-[var(--c-b0a030)]">Not connected — open to connect</span>
                  </template>
                  <span v-else class="text-[var(--c-585858)]">{{ row.account }}<template v-if="row.base_url"> · {{ row.base_url }}</template></span>
                </div>
              </div>
              <button v-if="!row.is_default" class="text-[0.7rem] text-[var(--c-808080)] hover:text-[var(--c-c0c0c0)] bg-none border-none cursor-pointer font-[inherit]" title="Use this one by default" @click="makeDefaultIntegration(row)">Make default</button>
              <button class="text-[var(--c-606060)] hover:text-[var(--c-a0c0ff)] p-1 cursor-pointer bg-none border-none" title="Edit" @click="openEditIntegration(row)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>
              </button>
              <button class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none" title="Remove" @click="removeIntegration(row)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          </div>
        </section>

        <!-- Add / edit integration modal -->
        <div v-if="intDraft" class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" @click.self="intDraft = null">
          <div :class="['w-full bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded-lg p-4 flex flex-col gap-3 max-h-[88vh] overflow-y-auto', intDraft.kind === 'microsoft' ? 'max-w-lg' : 'max-w-md']">
            <!-- Sticky header so the close button stays reachable while scrolling. -->
            <div class="sticky -top-4 z-10 bg-[var(--c-141414)] -mx-4 px-4 pt-4 pb-2 -mt-4 flex items-center justify-between gap-2 border-b border-[var(--c-1e1e1e)]">
              <span class="text-[0.85rem] text-fg font-medium">{{ intDraft.id ? `Edit ${kindMeta(intDraft.kind).label}` : `Add ${kindMeta(intDraft.kind).label}` }}</span>
              <button type="button" class="text-[var(--c-808080)] hover:text-fg bg-none border-none cursor-pointer p-1 -mr-1 leading-none" title="Close" @click="intDraft = null">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </button>
            </div>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Type
              <select v-model="intDraft.kind" :disabled="!!intDraft.id" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] disabled:opacity-60">
                <option v-for="k in INTEGRATION_KINDS" :key="k.kind" :value="k.kind">{{ k.label }} — {{ k.blurb }}</option>
              </select>
            </label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Name
              <input v-model="intDraft.name" placeholder="e.g. ASG Portal" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
            </label>

            <!-- Non-Microsoft kinds: simple credential form. -->
            <template v-if="intDraft.kind === 'helpdesk' || intDraft.kind === 'phoneus'">
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">URL
                <input v-model="intDraft.base_url" placeholder="https://portal.example.com" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
              </label>
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Email
                <input v-model="intDraft.email" type="email" autocomplete="off" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
              </label>
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Password
                <input v-model="intDraft.password" type="password" autocomplete="new-password" :placeholder="intDraft.id ? 'Enter to re-authenticate' : ''" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
              </label>
            </template>
            <template v-else-if="intDraft.kind === 'github'">
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Personal access token
                <input v-model="intDraft.token" type="password" autocomplete="off" :placeholder="intDraft.id ? 'Enter to replace' : 'github_pat_… or ghp_…'" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
              </label>
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Default owner <span class="text-[var(--c-585858)]">(optional)</span>
                <input v-model="intDraft.default_owner" placeholder="your-org-or-username" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
              </label>
            </template>

            <!-- Microsoft 365 tab bar (once the instance exists). -->
            <div v-if="intDraft.kind === 'microsoft' && intDraft.id" class="flex gap-1 border-b border-[var(--c-222222)]">
              <button
                v-for="t in ([['account','Account'],['shared','Shared mailboxes'],['autosort','AI sort']] as const)"
                :key="t[0]"
                type="button"
                :disabled="t[0] !== 'account' && !intDraft.connected"
                :title="t[0] !== 'account' && !intDraft.connected ? 'Connect the account first' : ''"
                :class="['px-3 py-1.5 text-[0.775rem] font-[inherit] border-b-2 -mb-px cursor-pointer bg-transparent disabled:opacity-40 disabled:cursor-default', modalTab === t[0] ? 'border-[var(--c-3a6adf)] text-[var(--c-7ab0ff)]' : 'border-transparent text-[var(--c-808080)] hover:not-disabled:text-[var(--c-c0c0c0)]']"
                @click="modalTab = t[0]"
              >{{ t[1] }}</button>
            </div>

            <!-- Account tab: also the whole body for non-Microsoft kinds and a not-yet-saved Microsoft instance. -->
            <template v-if="intDraft.kind !== 'microsoft' || !intDraft.id || modalTab === 'account'">
              <template v-if="intDraft.kind === 'microsoft'">
                <details class="instructions border border-[var(--c-222222)] rounded-md overflow-hidden">
                  <summary class="px-3 py-[0.45rem] text-[0.775rem] text-[var(--c-707070)] cursor-pointer select-none list-none hover:text-[var(--c-a0a0a0)]">Azure app setup instructions</summary>
                  <ol class="pt-3 pr-3.5 pb-3.5 pl-7 flex flex-col gap-2 text-[0.775rem] text-muted leading-[1.5] border-t border-[var(--c-1e1e1e)]">
                    <li class="pl-1">In <strong class="text-[var(--c-c0c0c0)]">portal.azure.com → Microsoft Entra ID → App registrations → New registration</strong>, choose "Accounts in any organizational directory".</li>
                    <li class="pl-1">Under <em class="not-italic text-[var(--c-a0a0a0)]">Redirect URI</em>, platform <strong class="text-[var(--c-c0c0c0)]">Web</strong>, enter:
                      <code class="block mt-[0.3rem] font-mono text-[0.75rem] bg-surface px-2 py-[0.3rem] rounded text-[var(--c-a0c8ff)] break-all">{{ callbackUri }}</code>
                    </li>
                    <li class="pl-1">Copy the <strong class="text-[var(--c-c0c0c0)]">Application (client) ID</strong> and <strong class="text-[var(--c-c0c0c0)]">Directory (tenant) ID</strong> into the fields below.</li>
                    <li class="pl-1">Under <strong class="text-[var(--c-c0c0c0)]">Certificates &amp; secrets</strong>, create a client secret and copy its <strong class="text-[var(--c-c0c0c0)]">Value</strong> below.</li>
                    <li class="pl-1">Under <strong class="text-[var(--c-c0c0c0)]">API permissions</strong>, add delegated Microsoft Graph permissions: Mail.Read, Mail.ReadWrite, Mail.Send, the three Mail.*.Shared, Calendars.ReadWrite, User.Read — then grant admin consent.</li>
                  </ol>
                </details>
                <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Tenant ID (Directory ID)
                  <input v-model="intDraft.tenant_id" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
                </label>
                <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Client ID (Application ID)
                  <input v-model="intDraft.client_id" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
                </label>
                <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Client Secret
                  <input v-model="intDraft.client_secret" type="password" autocomplete="new-password" :placeholder="intDraft.id ? 'Leave blank to keep existing secret' : 'Paste secret value here'" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" />
                </label>
              </template>

              <label class="flex items-center gap-2 text-[0.775rem] text-muted cursor-pointer select-none">
                <input type="checkbox" v-model="intDraft.is_default" class="cursor-pointer" /> Default for {{ kindMeta(intDraft.kind).label }}
              </label>
              <p v-if="intDraft.kind !== 'microsoft'" class="text-[0.7rem] text-[var(--c-585858)] leading-[1.5]">Credentials are exchanged for a token; the password/token is never stored.<template v-if="intDraft.kind === 'phoneus'"> Sending an SMS asks for approval in chat.</template></p>
              <div class="flex items-center gap-2">
                <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1.5 text-[0.8rem] font-[inherit] cursor-pointer hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="intSaving" @click="saveIntegration">{{ intSaving ? 'Saving…' : (intDraft.kind === 'microsoft' ? 'Save credentials' : 'Save') }}</button>
                <button v-if="intDraft.kind !== 'microsoft'" class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1.5 text-[0.8rem] font-[inherit] cursor-pointer hover:text-muted" @click="intDraft = null">Cancel</button>
                <span v-if="intMsg" class="text-[0.775rem]" :class="intMsg.startsWith('Saved') ? 'text-[var(--c-6ecf8e)]' : 'text-[var(--c-c06060)]'">{{ intMsg }}</span>
              </div>

              <!-- Connection (Microsoft, once the instance exists). -->
              <div v-if="intDraft.kind === 'microsoft' && intDraft.id" class="flex flex-col gap-2 border-t border-[var(--c-222222)] pt-3">
                <div class="flex items-center justify-between gap-2">
                  <span class="text-[0.78rem] text-[var(--c-c0c0c0)] font-medium">Connection</span>
                  <span v-if="intDraft.connected" class="text-[0.72rem] inline-flex items-center gap-1 text-success"><span class="w-1.5 h-1.5 rounded-full bg-success"></span>Connected</span>
                  <span v-else class="text-[0.72rem] text-[var(--c-b0a030)]">Not connected</span>
                </div>
                <div class="flex items-center gap-2">
                  <button v-if="!intDraft.connected" type="button" class="bg-[var(--c-0d2a1a)] text-success border border-[var(--c-1a4030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:bg-[var(--c-122e1e)]" @click="connectMicrosoft">Connect Microsoft 365 →</button>
                  <button v-else type="button" class="bg-[var(--c-2a1010)] text-[var(--c-ff7070)] border border-[var(--c-4a1a1a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:bg-[var(--c-3a1515)]" @click="disconnectMicrosoft">Disconnect</button>
                </div>
                <p v-if="!intDraft.connected" class="text-[0.72rem] text-[var(--c-585858)] leading-[1.5]">Save your credentials, then click Connect to authorise via Microsoft login. You'll be redirected back here.</p>
              </div>
            </template>

            <!-- Shared mailboxes tab. -->
            <template v-else-if="modalTab === 'shared'">
              <ul v-if="sharedMailboxes.length" class="list-none flex flex-col gap-1.5">
                <li v-for="m in sharedMailboxes" :key="m.address" class="flex items-center justify-between gap-3 bg-surface border border-[var(--c-1e1e1e)] rounded-md px-3 py-1.5">
                  <span class="flex flex-col min-w-0">
                    <span class="text-[0.8rem] text-[var(--c-d0d0d0)] truncate">{{ m.name || m.address }}</span>
                    <span v-if="m.name" class="text-[0.7rem] text-[var(--c-585858)] truncate">{{ m.address }}</span>
                  </span>
                  <button type="button" class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none shrink-0" title="Remove" @click="removeShared(m.address)">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                  </button>
                </li>
              </ul>
              <p v-else class="text-[0.75rem] text-[var(--c-585858)]">No shared mailboxes yet.</p>
              <form class="flex items-end gap-2 flex-wrap" @submit.prevent="addShared">
                <label class="flex flex-col gap-[0.2rem] text-[0.72rem] text-muted flex-1 min-w-[11rem]">Mailbox address
                  <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="sharedForm.address" type="email" placeholder="team@company.com" />
                </label>
                <label class="flex flex-col gap-[0.2rem] text-[0.72rem] text-muted flex-1 min-w-[7rem]">Label (optional)
                  <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="sharedForm.name" placeholder="Support" />
                </label>
                <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="sharedSaving || !sharedForm.address.trim()">{{ sharedSaving ? 'Checking…' : 'Add' }}</button>
              </form>
              <p v-if="sharedMsg" class="text-[0.75rem]" :class="sharedMsg === 'Mailbox added.' ? 'text-[var(--c-6ecf8e)]' : 'text-[var(--c-c06060)]'">{{ sharedMsg }}</p>
              <p class="text-[0.72rem] text-[var(--c-585858)] leading-[1.5]">Add mailboxes you've been granted access to. We verify you can open them before saving. They appear in the Email window's mailbox switcher under this account.</p>
            </template>

            <!-- AI sort tab. -->
            <template v-else-if="modalTab === 'autosort'">
              <div class="flex items-center justify-between gap-2">
                <span class="text-[0.78rem] text-[var(--c-c0c0c0)] font-medium">AI auto-sort</span>
                <span :class="['text-[0.72rem] px-[0.5rem] py-[0.1rem] rounded-full border', anyCatEnabled ? 'bg-[var(--c-0d2a1a)] text-success border-[var(--c-1a4030)]' : 'bg-surface text-[var(--c-484848)] border-[var(--c-282828)]']">{{ anyCatEnabled ? 'Active' : 'Off' }}</span>
              </div>
              <ul class="list-none flex flex-col gap-1">
                <li v-for="c in CATEGORY_FOLDERS" :key="c.label" class="flex items-baseline gap-2 text-[0.75rem]">
                  <span class="text-[var(--c-a0c8ff)] font-medium min-w-[5.5rem]">{{ c.label }}</span>
                  <span class="text-[var(--c-585858)]">{{ c.desc }}</span>
                </li>
                <li class="flex items-baseline gap-2 text-[0.75rem]">
                  <span class="text-[var(--c-d0a030)] font-medium min-w-[5.5rem]">⚑ Flagged</span>
                  <span class="text-[var(--c-585858)]">anything needing your attention (stays in Inbox)</span>
                </li>
              </ul>
              <!-- Pick a mailbox to configure (own + shared), one at a time. -->
              <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">Mailbox
                <select v-model="catMailbox" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]">
                  <option v-for="row in mailboxRows" :key="row.address" :value="row.address">{{ row.label }}{{ catTaskFor(row.address)?.enabled ? ' — on' : '' }}</option>
                </select>
              </label>
              <div class="flex flex-col gap-2 bg-[var(--c-0d0d0d)] border border-[var(--c-1e1e1e)] rounded-md p-3">
                <label class="flex items-center justify-between gap-4 text-[0.8125rem] text-[var(--c-d0d0d0)] cursor-pointer">
                  <span class="truncate">Auto-sort this mailbox</span>
                  <input type="checkbox" v-model="catTaskFor(catMailbox).enabled" class="w-4 h-4 accent-[var(--c-3a6adf)] cursor-pointer shrink-0" />
                </label>
                <div class="flex items-center justify-between gap-3 flex-wrap">
                  <select v-model="catTaskFor(catMailbox).provider" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.78rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] min-w-[9rem]">
                    <option value="">First configured</option>
                    <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
                  </select>
                  <button type="button" class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-2.5 py-1 cursor-pointer text-[0.75rem] font-[inherit] hover:not-disabled:bg-[var(--c-282828)] disabled:opacity-50" :disabled="catRunning !== null" @click="runCategorizer(catMailbox)">{{ catRunning === catMailbox ? 'Sorting…' : 'Run now' }}</button>
                </div>
                <textarea v-model="catTaskFor(catMailbox).instructions" rows="3" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.775rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Custom sorting instructions for this mailbox (optional)" />
              </div>
              <div class="flex flex-col gap-2.5 bg-[var(--c-0d0d0d)] border border-[var(--c-1e1e1e)] rounded-md p-3">
                <label class="flex items-center justify-between gap-4 text-[0.775rem] text-muted"><span>Check interval (seconds)</span>
                  <input type="number" min="60" v-model.number="catConfig.interval_secs" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] w-[6rem]" />
                </label>
                <label class="flex items-center justify-between gap-4 text-[0.775rem] text-muted"><span>Max emails per run</span>
                  <input type="number" min="1" max="50" v-model.number="catConfig.batch_limit" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] w-[6rem]" />
                </label>
              </div>
              <div class="flex items-center gap-2">
                <button type="button" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="catSaving" @click="saveCategorizer">{{ catSaving ? 'Saving…' : 'Save auto-sort' }}</button>
                <span v-if="catMsg" class="text-[0.75rem] text-[var(--c-888888)]">{{ catMsg }}</span>
              </div>
              <p class="text-[0.72rem] text-[var(--c-585858)] leading-[1.5]">Auto-sort moves and flags mail in your live mailbox. Every action is recorded in the Logs window.</p>
            </template>

          </div>
        </div>

      </div>

      <!-- Appearance -->
      <!-- Scheduled agents -->
      <div v-else-if="activeTab === 'agents'" class="px-6 py-5 flex flex-col gap-5">
        <div>
          <h2 class="text-[0.9375rem] font-semibold text-fg">Scheduled agents</h2>
          <p class="text-[var(--c-585858)] text-[0.75rem] mt-1">Recurring agent runs in your timezone — e.g. "summarize overnight email" each weekday at 7:00. Output lands in a new chat session (and a push notification if configured). Tools that need approval are skipped, never auto-approved.</p>
        </div>

        <div v-if="agentsError" class="text-danger text-[0.775rem]">{{ agentsError }}</div>

        <div v-for="(a, i) in agentsList" :key="a.id || i" class="flex flex-col gap-2.5 bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-md p-3">
          <div class="flex items-center gap-2">
            <input v-model="a.name" class="flex-1 bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.8125rem] font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Name (e.g. Morning briefing)" />
            <input v-model="a.time" class="w-[4.5rem] bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.8125rem] font-[inherit] outline-none text-center focus:border-[var(--c-3a6adf)]" placeholder="07:00" />
            <label class="flex items-center gap-1.5 text-[0.75rem] text-[var(--c-808080)] cursor-pointer select-none">
              <input type="checkbox" v-model="a.enabled" class="cursor-pointer" /> enabled
            </label>
            <label class="flex items-center gap-1.5 text-[0.75rem] text-[var(--c-808080)] cursor-pointer select-none" title="Don't send the usual ‘run finished’ notification — only notify if the agent itself flags something (via notify_user). Ideal for health/monitor checks.">
              <input type="checkbox" v-model="a.quiet" class="cursor-pointer" /> quiet
            </label>
          </div>
          <div class="flex items-center gap-1">
            <button v-for="d in DAY_TOKENS" :key="d" :class="['px-2 py-[0.2rem] rounded text-[0.68rem] uppercase border cursor-pointer font-[inherit] transition-colors duration-100', a.days.includes(d) ? 'bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border-[var(--c-2a4a8a)]' : 'bg-surface text-[var(--c-585858)] border-raised hover:text-[var(--c-909090)]']" @click="toggleAgentDay(a, d)">{{ d }}</button>
            <select v-model="a.provider" class="ml-auto bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
              <option value="">default provider</option>
              <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
            </select>
          </div>
          <textarea v-model="a.instructions" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Instructions — what should the agent do on each run?" />
          <div class="flex items-center gap-2">
            <button class="bg-surface text-[var(--c-7adfbb)] border border-raised rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-1a241e)] disabled:opacity-50" :disabled="agentRunning === a.id || !a.id" :title="a.id ? '' : 'Save first'" @click="runAgentNow(a)">{{ agentRunning === a.id ? 'Running…' : 'Run now' }}</button>
            <span v-if="a.last_run" class="text-[0.68rem] text-[var(--c-505050)]">last ran {{ a.last_run }}</span>
            <button class="ml-auto bg-none border-none text-[var(--c-606060)] hover:text-[var(--c-d08080)] cursor-pointer text-xs font-[inherit]" @click="removeAgent(i)">Remove</button>
          </div>
        </div>

        <div class="flex items-center gap-2">
          <button class="flex items-center gap-[0.35rem] bg-surface text-[var(--c-808080)] border border-raised rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" @click="addAgent">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
            Add agent
          </button>
          <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="agentsSaving" @click="saveAgents">{{ agentsSaving ? 'Saving…' : 'Save' }}</button>
        </div>
      </div>

      <div v-else-if="activeTab === 'appearance'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">Appearance</h2>
        <section class="flex flex-col gap-3">
          <h3 class="text-[0.8125rem] font-semibold text-[var(--c-c0c0c0)]">Theme</h3>
          <p class="text-[var(--c-585858)] text-[0.75rem] -mt-1">Applies to this browser. Each device remembers its own choice.</p>
          <div class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-2.5">
            <button
              v-for="t in THEMES"
              :key="t.key"
              :class="['flex flex-col rounded-lg overflow-hidden border-2 cursor-pointer p-0 bg-transparent text-left transition-colors duration-100', activeTheme === t.key ? 'border-accent' : 'border-[var(--c-252525)] hover:border-[var(--c-404040)]']"
              :title="t.description"
              @click="selectTheme(t.key)"
            >
              <!-- Mini preview: sidebar + window on the theme's background -->
              <span class="block h-[72px] relative" :style="{ background: t.swatch.bg }">
                <span class="absolute left-0 top-0 bottom-0 w-[22%]" :style="{ background: t.swatch.surface }" />
                <span class="absolute left-[30%] top-[14%] w-[58%] h-[62%] rounded" :style="{ background: t.swatch.surface }">
                  <span class="absolute left-[10%] top-[18%] w-[55%] h-[9%] rounded-full" :style="{ background: t.swatch.text, opacity: 0.85 }" />
                  <span class="absolute left-[10%] top-[42%] w-[75%] h-[9%] rounded-full" :style="{ background: t.swatch.text, opacity: 0.4 }" />
                  <span class="absolute left-[10%] top-[66%] w-[34%] h-[14%] rounded-sm" :style="{ background: t.swatch.accent }" />
                </span>
              </span>
              <span class="flex items-center gap-1.5 px-2.5 py-1.5 border-t border-[var(--c-252525)] bg-surface">
                <span class="text-[0.75rem] font-medium" :class="activeTheme === t.key ? 'text-fg' : 'text-[var(--c-a0a0a0)]'">{{ t.label }}</span>
                <svg v-if="activeTheme === t.key" class="ml-auto text-accent" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              </span>
            </button>
          </div>
        </section>
      </div>

      <!-- Users -->
      <div v-else-if="activeTab === 'users'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">Users</h2>

        <section class="flex flex-col gap-3">
          <ul v-if="userList.length" class="list-none flex flex-col gap-1.5">
            <li v-for="u in userList" :key="u.id" class="flex justify-between items-center bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-md px-3 py-2 gap-4">
              <div class="flex items-center gap-2 min-w-0">
                <span class="text-[0.8125rem] text-[var(--c-d0d0d0)]">{{ u.username }}</span>
                <span class="text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border" :class="u.role === 'admin' ? 'text-[var(--c-7ab0ff)] border-[var(--c-7ab0ff55)]' : 'text-[var(--c-9a9a9a)] border-[var(--c-9a9a9a55)]'">{{ u.role }}</span>
                <span v-if="u.status === 'disabled'" class="text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border text-[var(--c-df7a7a)] border-[var(--c-df7a7a55)]">disabled</span>
              </div>
              <div v-if="u.role !== 'admin'" class="flex items-center gap-1.5 shrink-0">
                <button v-if="u.status === 'active'" class="bg-[var(--c-16202e)] text-[var(--c-9ab4d4)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-1c2a3c)]" title="Act as this user to set things up for them" @click="authStore.impersonate(u.id)">Impersonate</button>
                <button v-if="u.status === 'active'" class="bg-[var(--c-2a2418)] text-[var(--c-e0b060)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-3a3020)]" @click="setUserStatus(u, 'disable')">Disable</button>
                <button v-else class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-254a35)]" @click="setUserStatus(u, 'enable')">Enable</button>
                <button class="bg-[var(--c-1e1010)] text-[var(--c-a06060)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-2a1515)] hover:text-[var(--c-d08080)]" @click="deleteUser(u)">Delete</button>
              </div>
            </li>
          </ul>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Invites</h3>
          <div class="flex gap-2">
            <input v-model="inviteLabelInput" class="flex-1 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" placeholder="Who's this for? (label, optional)" @keyup.enter="createInvite" />
            <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)]" @click="createInvite">Create invite</button>
          </div>
          <ul v-if="pendingInvites.length" class="list-none flex flex-col gap-1.5">
            <li v-for="inv in pendingInvites" :key="inv.code" class="flex flex-col gap-1 bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-md px-3 py-2">
              <div class="flex items-center justify-between gap-3">
                <span class="text-[0.8125rem] text-[var(--c-d0d0d0)]">{{ inv.label || '(no label)' }}</span>
                <div class="flex items-center gap-1.5">
                  <button class="bg-[var(--c-1a2a1e)] text-[var(--c-8edfae)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-1f3526)]" @click="copyInvite(inv)">{{ copiedCode === inv.code ? 'Copied ✓' : 'Copy link' }}</button>
                  <button class="bg-[var(--c-1e1010)] text-[var(--c-a06060)] border-none rounded px-2 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-2a1515)]" @click="revokeInvite(inv)">Revoke</button>
                </div>
              </div>
              <span class="text-[0.7rem] text-[var(--c-585858)] break-all">{{ inviteLink(inv) }} · expires {{ new Date(inv.expires_at).toLocaleDateString() }}</span>
            </li>
          </ul>
          <p v-else class="text-[var(--c-484848)] text-[0.8125rem]">No pending invites. Create one and email the link — it disappears here once redeemed.</p>
        </section>
      </div>

      <!-- System -->
      <div v-else-if="activeTab === 'system'" class="px-6 py-5 flex flex-col gap-6">
        <h2 class="text-[0.9375rem] font-semibold text-fg">System</h2>
        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Timezone</h3>
          <div class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]">
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Home timezone
              <select class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="timezone">
                <option v-for="tz in timezones" :key="tz" :value="tz">{{ tz }}</option>
              </select>
            </label>
            <p class="text-[0.75rem] text-[var(--c-585858)]">The AI resolves "today", "tomorrow at 3pm" and presents all calendar times in this timezone.</p>
            <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)] disabled:opacity-40 disabled:cursor-default" :disabled="tzSaving || !timezone" @click="saveTimezone">
              {{ tzSaving ? 'Saving…' : 'Save' }}
            </button>
            <p v-if="tzMsg" class="text-[0.775rem]" :class="tzMsg === 'Saved.' ? 'text-[var(--c-4caf6e)]' : 'text-[var(--c-c06060)]'">{{ tzMsg }}</p>
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <div class="flex items-center gap-2">
            <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Token usage</h3>
            <select v-model.number="usageDays" class="ml-auto bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer" @change="loadUsage">
              <option :value="7">7 days</option>
              <option :value="30">30 days</option>
              <option :value="90">90 days</option>
            </select>
            <button class="flex items-center justify-center bg-surface text-[var(--c-808080)] border border-raised rounded px-2 py-1 cursor-pointer transition-colors duration-100 hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" title="Refresh" @click="loadUsage">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
            </button>
          </div>
          <div class="bg-[var(--c-111111)] rounded-lg border border-[var(--c-222222)] overflow-hidden">
            <p v-if="usageRows.length === 0" class="text-[0.775rem] text-[var(--c-585858)] p-3.5">No usage recorded yet — counts accumulate as models report token totals.</p>
            <table v-else class="w-full text-[0.75rem] border-collapse">
              <thead>
                <tr class="text-left text-[var(--c-585858)] uppercase text-[0.62rem] tracking-[0.05em]">
                  <th class="px-3 py-2 font-semibold">User</th>
                  <th class="px-3 py-2 font-semibold">Provider</th>
                  <th class="px-3 py-2 font-semibold">Purpose</th>
                  <th class="px-3 py-2 font-semibold text-right">Requests</th>
                  <th class="px-3 py-2 font-semibold text-right">Prompt</th>
                  <th class="px-3 py-2 font-semibold text-right">Completion</th>
                  <th class="px-3 py-2 font-semibold text-right">Cost</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(r, i) in usageRows" :key="i" class="border-t border-[var(--c-1a1a1a)] text-[var(--c-c0c0c0)]">
                  <td class="px-3 py-1.5">{{ r.username }}</td>
                  <td class="px-3 py-1.5">{{ r.provider }} <span class="text-[var(--c-585858)]">{{ r.model_id }}</span></td>
                  <td class="px-3 py-1.5">{{ r.purpose }}</td>
                  <td class="px-3 py-1.5 text-right">{{ r.requests.toLocaleString() }}</td>
                  <td class="px-3 py-1.5 text-right">{{ r.prompt_tokens.toLocaleString() }}</td>
                  <td class="px-3 py-1.5 text-right">{{ r.completion_tokens.toLocaleString() }}</td>
                  <td class="px-3 py-1.5 text-right" :class="r.cost == null ? 'text-[var(--c-585858)]' : ''">{{ fmtCost(r.cost) }}</td>
                </tr>
                <tr v-if="usageTotalCost > 0" class="border-t border-[var(--c-252525)] text-fg font-semibold">
                  <td class="px-3 py-1.5" colspan="6">Total (priced models)</td>
                  <td class="px-3 py-1.5 text-right">{{ fmtCost(usageTotalCost) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em]">Model prices</h3>
          <div class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]">
            <p class="text-[0.75rem] text-[var(--c-585858)]">US$ per million tokens. A row applies to any model id containing its text (longest match wins) — e.g. "gpt-4o-mini" overrides "gpt-4o". Models with no row (local Ollama) show no cost.</p>
            <div v-for="(p, i) in priceRows" :key="i" class="flex items-center gap-2">
              <input class="flex-1 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="p.model" placeholder="model id contains…" />
              <input class="w-28 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] text-right focus:outline-none focus:border-[var(--c-3a6adf)]" v-model.number="p.prompt_per_mtok" type="number" min="0" step="0.01" title="$ per 1M prompt tokens" />
              <input class="w-28 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] text-right focus:outline-none focus:border-[var(--c-3a6adf)]" v-model.number="p.completion_per_mtok" type="number" min="0" step="0.01" title="$ per 1M completion tokens" />
              <button class="bg-none border-none text-[var(--c-585858)] cursor-pointer text-base leading-none hover:text-[var(--c-ff7070)]" title="Remove" @click="priceRows.splice(i, 1)">×</button>
            </div>
            <div v-if="priceRows.length" class="flex gap-2 text-[0.65rem] text-[var(--c-484848)] uppercase tracking-[0.05em]">
              <span class="flex-1">Model match</span><span class="w-28 text-right">$/M prompt</span><span class="w-28 text-right">$/M completion</span><span class="w-4"></span>
            </div>
            <div class="flex items-center gap-2">
              <button class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-282828)]" @click="addPriceRow">Add model</button>
              <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)]" @click="savePrices">Save prices</button>
              <span v-if="pricesMsg" class="text-[0.775rem]" :class="pricesMsg === 'Saved.' ? 'text-[var(--c-4caf6e)]' : 'text-[var(--c-c06060)]'">{{ pricesMsg }}</span>
            </div>
          </div>
        </section>
      </div>

    </div>
  </div>
</template>
