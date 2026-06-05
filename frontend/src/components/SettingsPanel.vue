<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'
import { useAuthStore } from '../stores/auth'
import { THEMES, saveTheme, currentTheme } from '../theme'

const logs = useLogsStore()
const activeTheme = ref(currentTheme())
function selectTheme(key: string) {
  activeTheme.value = key
  saveTheme(key) // applies locally and persists to the account
}
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.role === 'admin')

const props = defineProps<{ initialTab?: string }>()

type Tab = 'account' | 'providers' | 'mcp' | 'tools' | 'integrations' | 'appearance' | 'users' | 'system'
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
  await loadEmailConfig()
  await loadCategorizer()
  await loadHelpdeskConfig()
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

// ── Integrations ──────────────────────────────────────────────────────────────
const emailConfig = ref<api.EmailConfigStatus>({
  configured: false, connected: false, tenant_id: '', client_id: '', connected_email: null,
})
const emailForm = ref({ tenant_id: '', client_id: '', client_secret: '' })
const emailMsg = ref('')
const emailSaving = ref(false)

const callbackUri = computed(() => window.location.origin + '/api/integrations/email/callback')

// Integration cards collapse to their header row (title + status pill).
// Unconfigured cards default open so first-time setup is visible; the user's
// choice persists per browser.
const openCards = ref<Record<string, boolean>>(
  JSON.parse(localStorage.getItem('settings:integrations-open') || '{}'),
)
function cardOpen(key: string, dflt = false): boolean {
  return openCards.value[key] ?? dflt
}
function toggleCard(key: string, dflt = false) {
  openCards.value[key] = !cardOpen(key, dflt)
  localStorage.setItem('settings:integrations-open', JSON.stringify(openCards.value))
}

async function loadEmailConfig() {
  const cfg = await api.integrations.email.getConfig()
  emailConfig.value = cfg
  emailForm.value.tenant_id = cfg.tenant_id
  emailForm.value.client_id = cfg.client_id
  emailForm.value.client_secret = ''
  if (cfg.connected) await loadShared()
}

async function saveEmailConfig() {
  emailSaving.value = true
  emailMsg.value = ''
  try {
    await api.integrations.email.saveConfig({
      tenant_id: emailForm.value.tenant_id,
      client_id: emailForm.value.client_id,
      client_secret: emailForm.value.client_secret || undefined,
    })
    await loadEmailConfig()
    emailMsg.value = 'Credentials saved.'
  } catch (e: unknown) {
    emailMsg.value = e instanceof Error ? e.message : 'Save failed.'
  } finally {
    emailSaving.value = false
  }
}

async function disconnectEmail() {
  await api.integrations.email.disconnect()
  await loadEmailConfig()
}

function connectEmail() {
  window.location.href = '/api/integrations/email/connect'
}

// ── Helpdesk integration ────────────────────────────────────────────────────────
const hdConfig = ref<api.HelpdeskStatus>({ connected: false, base_url: '', email: '' })
const hdForm = ref({ base_url: '', email: '', password: '' })
const hdMsg = ref('')
const hdSaving = ref(false)

async function loadHelpdeskConfig() {
  try {
    hdConfig.value = await api.integrations.helpdesk.getConfig()
    if (hdConfig.value.connected) {
      hdForm.value.base_url = hdConfig.value.base_url
      hdForm.value.email = hdConfig.value.email
    }
  } catch { /* not connected */ }
}

async function connectHelpdesk() {
  hdSaving.value = true
  hdMsg.value = ''
  try {
    hdConfig.value = await api.integrations.helpdesk.connect(
      hdForm.value.base_url.trim(),
      hdForm.value.email.trim(),
      hdForm.value.password,
    )
    hdForm.value.password = ''
    hdMsg.value = 'Connected.'
  } catch (e: unknown) {
    hdMsg.value = e instanceof Error ? e.message : 'Connection failed.'
  } finally {
    hdSaving.value = false
  }
}

async function disconnectHelpdesk() {
  await api.integrations.helpdesk.disconnect()
  hdConfig.value = { connected: false, base_url: '', email: '' }
  hdForm.value.password = ''
  hdMsg.value = ''
}

// ── Shared mailboxes ────────────────────────────────────────────────────────────
const sharedMailboxes = ref<api.SharedMailbox[]>([])
const sharedForm = ref({ address: '', name: '' })
const sharedMsg = ref('')
const sharedSaving = ref(false)

async function loadShared() {
  try {
    sharedMailboxes.value = (await api.integrations.email.listShared()).mailboxes
  } catch { /* not connected yet, or transient — leave the list empty */ }
}

async function addShared() {
  const address = sharedForm.value.address.trim()
  if (!address) return
  sharedSaving.value = true
  sharedMsg.value = ''
  try {
    const res = await api.integrations.email.addShared(address, sharedForm.value.name.trim() || undefined)
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
  await api.integrations.email.removeShared(address)
  sharedMailboxes.value = sharedMailboxes.value.filter(m => m.address !== address)
  // Drop its auto-sort task so we don't persist a row for a gone mailbox.
  catConfig.value.tasks = catConfig.value.tasks.filter(t => t.mailbox !== address)
}

// ── Email auto-sort (categorizer) ───────────────────────────────────────────────
const catConfig = ref<api.CategorizerConfig>({
  interval_secs: 300, batch_limit: 25, tasks: [],
})
const catMsg = ref('')
const catSaving = ref(false)
// The mailbox address currently running a manual sort, or null.
const catRunning = ref<string | null>(null)

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

async function loadCategorizer() {
  try {
    catConfig.value = await api.emailCategorizer.getConfig()
  } catch { /* not configured yet; keep defaults */ }
  ensureCatTasks()
}

async function saveCategorizer() {
  catSaving.value = true
  catMsg.value = ''
  try {
    catConfig.value = await api.emailCategorizer.saveConfig(catConfig.value)
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
  catRunning.value = mailbox
  catMsg.value = ''
  logs.info('Categorizer', `Manual run started (${mailbox || 'my mailbox'})`)
  try {
    const s = await api.emailCategorizer.runNow(mailbox || undefined)
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

async function toggleTwoFactor() {
  try {
    await api.auth.toggleTwoFactor(!twoFactorEnabled.value)
    twoFactorEnabled.value = !twoFactorEnabled.value
    accountMsg.value = twoFactorEnabled.value ? '2FA enabled.' : '2FA disabled.'
  } catch (e: unknown) {
    accountMsg.value = e instanceof Error ? e.message : 'Failed.'
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
        <button :class="['flex items-center gap-2 px-3 py-[0.45rem] rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-[120ms] whitespace-nowrap', activeTab === 'system' ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']" @click="activeTab = 'system'">
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
          <div class="flex items-center justify-between bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg px-3.5 py-3 gap-4">
            <div>
              <div class="text-[0.8125rem] text-[var(--c-d0d0d0)]">Authenticator app</div>
              <div class="text-[0.75rem] text-[var(--c-585858)] mt-[0.1rem]">{{ twoFactorEnabled ? 'Active' : 'Not configured' }}</div>
            </div>
            <button class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-282828)]" @click="toggleTwoFactor">
              {{ twoFactorEnabled ? 'Disable' : 'Enable' }}
            </button>
          </div>
          <p v-if="accountMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ accountMsg }}</p>
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

        <!-- Microsoft 365 card -->
        <section class="bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg p-3.5 flex flex-col gap-3.5">
          <div class="flex items-center justify-between gap-4 cursor-pointer select-none -m-1 p-1 rounded" @click="toggleCard('m365', !emailConfig.configured)">
            <div class="flex items-center gap-2.5">
              <!-- Microsoft "four squares" logo -->
              <svg width="20" height="20" viewBox="0 0 21 21" xmlns="http://www.w3.org/2000/svg">
                <rect x="1" y="1" width="9" height="9" fill="#f25022"/>
                <rect x="11" y="1" width="9" height="9" fill="#7fba00"/>
                <rect x="1" y="11" width="9" height="9" fill="#00a4ef"/>
                <rect x="11" y="11" width="9" height="9" fill="#ffb900"/>
              </svg>
              <div>
                <div class="text-[0.8125rem] text-[var(--c-d0d0d0)] font-medium">Microsoft 365</div>
                <div class="text-[0.72rem] text-[var(--c-585858)] mt-[0.1rem]">Work / School account via Entra ID</div>
              </div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <div v-if="emailConfig.connected" class="text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap bg-[var(--c-0d2a1a)] text-success border border-[var(--c-1a4030)] flex items-center gap-[0.35rem]">
                <span class="w-1.5 h-1.5 rounded-full bg-success shrink-0"></span>
                {{ emailConfig.connected_email ?? 'Connected' }}
              </div>
              <div v-else-if="emailConfig.configured" class="text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap bg-[var(--c-1a1a0d)] text-[var(--c-b0a030)] border border-[var(--c-3a3010)]">
                Credentials saved
              </div>
              <div v-else class="text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap bg-surface text-[var(--c-484848)] border border-[var(--c-282828)]">
                Not configured
              </div>
              <svg :class="['text-[var(--c-505050)] transition-transform duration-150', cardOpen('m365', !emailConfig.configured) ? 'rotate-180' : '']" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
            </div>
          </div>

          <template v-if="cardOpen('m365', !emailConfig.configured)">
          <!-- Setup instructions — each user registers their own Azure app. -->
          <details class="instructions border border-[var(--c-222222)] rounded-md overflow-hidden">
            <summary class="px-3 py-[0.45rem] text-[0.775rem] text-[var(--c-707070)] cursor-pointer select-none list-none hover:text-[var(--c-a0a0a0)]">Setup instructions</summary>
            <ol class="pt-3 pr-3.5 pb-3.5 pl-7 flex flex-col gap-2 text-[0.775rem] text-muted leading-[1.5] border-t border-[var(--c-1e1e1e)]">
              <li class="pl-1">
                Sign in to <strong class="text-[var(--c-c0c0c0)]">portal.azure.com</strong> with your Microsoft 365 account.
              </li>
              <li class="pl-1">
                Go to <strong class="text-[var(--c-c0c0c0)]">Microsoft Entra ID → App registrations → New registration</strong>.
              </li>
              <li class="pl-1">
                Set a name (e.g. <em class="text-[var(--c-a0a0a0)] not-italic">Episteme</em>), and for <em class="text-[var(--c-a0a0a0)] not-italic">Supported account types</em> choose
                <strong class="text-[var(--c-c0c0c0)]">"Accounts in any organizational directory (Any Microsoft Entra ID tenant)"</strong>.
              </li>
              <li class="pl-1">
                Under <em class="text-[var(--c-a0a0a0)] not-italic">Redirect URI</em>, select platform <strong class="text-[var(--c-c0c0c0)]">Web</strong> and enter:
                <code class="block mt-[0.3rem] font-mono text-[0.75rem] bg-surface px-2 py-[0.3rem] rounded text-[var(--c-a0c8ff)] break-all">{{ callbackUri }}</code>
              </li>
              <li class="pl-1">
                Click <strong class="text-[var(--c-c0c0c0)]">Register</strong>. From the overview page copy:
                <ul class="mt-[0.35rem] pl-5 flex flex-col gap-[0.2rem] list-disc">
                  <li><strong class="text-[var(--c-c0c0c0)]">Application (client) ID</strong> → paste into <em class="text-[var(--c-a0a0a0)] not-italic">Client ID</em> below</li>
                  <li><strong class="text-[var(--c-c0c0c0)]">Directory (tenant) ID</strong> → paste into <em class="text-[var(--c-a0a0a0)] not-italic">Tenant ID</em> below</li>
                </ul>
              </li>
              <li class="pl-1">
                Go to <strong class="text-[var(--c-c0c0c0)]">Certificates &amp; secrets → Client secrets → New client secret</strong>.
                Set a description and expiry, then copy the <strong class="text-[var(--c-c0c0c0)]">Value</strong> (not the Secret ID)
                → paste into <em class="text-[var(--c-a0a0a0)] not-italic">Client Secret</em> below.
              </li>
              <li class="pl-1">
                Go to <strong class="text-[var(--c-c0c0c0)]">API permissions → Add a permission → Microsoft Graph → Delegated permissions</strong>
                and add:
                <ul class="mt-[0.35rem] pl-5 flex flex-col gap-[0.2rem] list-disc">
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Mail.Read</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Mail.ReadWrite</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Mail.Send</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Mail.Read.Shared</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Mail.ReadWrite.Shared</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Mail.Send.Shared</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">Calendars.ReadWrite</code></li>
                  <li><code class="font-mono text-[0.75rem] bg-surface px-[0.3rem] py-[0.05rem] rounded-[0.2rem] text-[var(--c-a0c8ff)]">User.Read</code></li>
                </ul>
              </li>
              <li class="pl-1">
                Click <strong class="text-[var(--c-c0c0c0)]">Grant admin consent</strong> for your organisation (requires admin role),
                or ask your tenant administrator to do so.
              </li>
            </ol>
          </details>

          <!-- Credentials form — every user supplies their own Azure app. -->
          <form class="flex flex-col gap-2 bg-[var(--c-111111)] p-3.5 rounded-lg border border-[var(--c-222222)]" @submit.prevent="saveEmailConfig">
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Tenant ID (Directory ID)
              <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="emailForm.tenant_id" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" required />
            </label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Client ID (Application ID)
              <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="emailForm.client_id" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" required />
            </label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Client Secret
              <input
                class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]"
                v-model="emailForm.client_secret"
                type="password"
                autocomplete="new-password"
                :placeholder="emailConfig.configured ? 'Leave blank to keep existing secret' : 'Paste secret value here'"
              />
            </label>
            <p v-if="emailMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ emailMsg }}</p>
            <div class="flex items-center gap-2 flex-wrap">
              <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-254880)]" :disabled="emailSaving">
                {{ emailSaving ? 'Saving…' : 'Save credentials' }}
              </button>
              <button
                v-if="emailConfig.configured && !emailConfig.connected"
                type="button"
                class="bg-[var(--c-0d2a1a)] text-success border border-[var(--c-1a4030)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-122e1e)]"
                @click="connectEmail"
              >
                Connect Microsoft 365 →
              </button>
              <button
                v-if="emailConfig.connected"
                type="button"
                class="bg-[var(--c-2a1010)] text-[var(--c-ff7070)] border border-[var(--c-4a1a1a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] self-start font-[inherit] transition-[background] duration-[120ms] hover:bg-[var(--c-3a1515)]"
                @click="disconnectEmail"
              >
                Disconnect
              </button>
            </div>
            <p v-if="emailConfig.configured && !emailConfig.connected" class="text-[0.72rem] text-[var(--c-585858)] leading-[1.5]">
              After saving credentials, click <em class="text-[var(--c-a0a0a0)] not-italic">Connect</em> to authorise via Microsoft login.
              You'll be redirected back here when done.
            </p>
          </form>
          </template>
        </section>

        <!-- Shared mailboxes card -->
        <section v-if="emailConfig.connected" class="bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg p-3.5 flex flex-col gap-3.5">
          <div class="flex items-center justify-between gap-4 cursor-pointer select-none -m-1 p-1 rounded" @click="toggleCard('shared')">
            <div class="flex items-center gap-2.5">
              <svg class="text-[var(--c-7ab0ff)]" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/>
              </svg>
              <div>
                <div class="text-[0.8125rem] text-[var(--c-d0d0d0)] font-medium">Shared mailboxes</div>
                <div class="text-[0.72rem] text-[var(--c-585858)] mt-[0.1rem]">Add mailboxes you've been granted access to; pick one from the switcher in the Email window</div>
              </div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <div v-if="sharedMailboxes.length" class="text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap bg-surface text-[var(--c-808080)] border border-[var(--c-282828)]">
                {{ sharedMailboxes.length }}
              </div>
              <svg :class="['text-[var(--c-505050)] transition-transform duration-150', cardOpen('shared') ? 'rotate-180' : '']" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
            </div>
          </div>

          <template v-if="cardOpen('shared')">
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
          <p v-else class="text-[0.775rem] text-[var(--c-585858)]">No shared mailboxes yet.</p>

          <form class="flex flex-col gap-2" @submit.prevent="addShared">
            <div class="flex items-end gap-2 flex-wrap">
              <label class="flex flex-col gap-[0.2rem] text-[0.72rem] text-muted flex-1 min-w-[12rem]">
                Mailbox address
                <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="sharedForm.address" type="email" placeholder="team@company.com" />
              </label>
              <label class="flex flex-col gap-[0.2rem] text-[0.72rem] text-muted flex-1 min-w-[8rem]">
                Label (optional)
                <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="sharedForm.name" placeholder="Support" />
              </label>
              <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="sharedSaving || !sharedForm.address.trim()">
                {{ sharedSaving ? 'Checking…' : 'Add' }}
              </button>
            </div>
            <p v-if="sharedMsg" class="text-[0.775rem]" :class="sharedMsg === 'Mailbox added.' ? 'text-[var(--c-6ecf8e)]' : 'text-[var(--c-c06060)]'">{{ sharedMsg }}</p>
            <p class="text-[0.7rem] text-[var(--c-585858)] leading-[1.5]">
              We verify you can open the mailbox before saving. Access is granted by your admin in Microsoft 365 (Full Access permission).
              If adding fails even though you have access, <em class="not-italic text-[var(--c-a0a0a0)]">Disconnect</em> and reconnect above to grant shared-mailbox permissions.
            </p>
          </form>
          </template>
        </section>

        <!-- AI auto-sort card -->
        <section class="bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg p-3.5 flex flex-col gap-3.5">
          <div class="flex items-center justify-between gap-4 cursor-pointer select-none -m-1 p-1 rounded" @click="toggleCard('autosort')">
            <div class="flex items-center gap-2.5">
              <svg class="text-[var(--c-7ab0ff)]" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z"/>
              </svg>
              <div>
                <div class="text-[0.8125rem] text-[var(--c-d0d0d0)] font-medium">AI auto-sort</div>
                <div class="text-[0.72rem] text-[var(--c-585858)] mt-[0.1rem]">Sort low-priority inbox mail into folders; flag what needs you</div>
              </div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <div :class="['text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap flex items-center gap-[0.35rem] border', anyCatEnabled ? 'bg-[var(--c-0d2a1a)] text-success border-[var(--c-1a4030)]' : 'bg-surface text-[var(--c-484848)] border-[var(--c-282828)]']">
                <span v-if="anyCatEnabled" class="w-1.5 h-1.5 rounded-full bg-success shrink-0"></span>
                {{ anyCatEnabled ? 'Active' : 'Off' }}
              </div>
              <svg :class="['text-[var(--c-505050)] transition-transform duration-150', cardOpen('autosort') ? 'rotate-180' : '']" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
            </div>
          </div>

          <template v-if="cardOpen('autosort')">
          <p v-if="!emailConfig.connected" class="text-[0.775rem] text-[var(--c-b0a030)]">
            Connect a Microsoft 365 account above to use auto-sort.
          </p>

          <template v-else>
            <!-- Category → folder reference -->
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

            <!-- Per-mailbox sort tasks: the own mailbox + each shared mailbox,
                 each toggled and run independently. -->
            <div class="flex flex-col gap-2">
              <div v-for="row in mailboxRows" :key="row.address" class="flex flex-col gap-2 bg-[var(--c-0d0d0d)] border border-[var(--c-1e1e1e)] rounded-md p-3">
                <label class="flex items-center justify-between gap-4 text-[0.8125rem] text-[var(--c-d0d0d0)] cursor-pointer">
                  <span class="truncate">{{ row.label }}</span>
                  <input type="checkbox" v-model="catTaskFor(row.address).enabled" class="w-4 h-4 accent-[var(--c-3a6adf)] cursor-pointer shrink-0" />
                </label>
                <div class="flex items-center justify-between gap-3 flex-wrap">
                  <select v-model="catTaskFor(row.address).provider" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.78rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] min-w-[9rem]">
                    <option value="">First configured</option>
                    <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
                  </select>
                  <button type="button" class="bg-[var(--c-1e1e1e)] text-[var(--c-c0c0c0)] border border-[var(--c-303030)] rounded-md px-2.5 py-1 cursor-pointer text-[0.75rem] font-[inherit] transition-[background] duration-[120ms] hover:not-disabled:bg-[var(--c-282828)] disabled:opacity-50" :disabled="catRunning !== null" @click="runCategorizer(row.address)">
                    {{ catRunning === row.address ? 'Sorting…' : 'Run now' }}
                  </button>
                </div>
                <textarea
                  v-model="catTaskFor(row.address).instructions"
                  rows="2"
                  class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.775rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]"
                  placeholder="Custom sorting instructions for this mailbox (optional) — e.g. “File anything from suppliers into Invoices; flag mail mentioning contracts.”"
                />
              </div>
            </div>

            <div class="flex flex-col gap-2.5 bg-[var(--c-0d0d0d)] border border-[var(--c-1e1e1e)] rounded-md p-3">
              <label class="flex items-center justify-between gap-4 text-[0.775rem] text-muted">
                <span>Check interval (seconds)</span>
                <input type="number" min="60" v-model.number="catConfig.interval_secs" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] w-[6rem]" />
              </label>
              <label class="flex items-center justify-between gap-4 text-[0.775rem] text-muted">
                <span>Max emails per run</span>
                <input type="number" min="1" max="50" v-model.number="catConfig.batch_limit" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)] w-[6rem]" />
              </label>
            </div>

            <p v-if="catMsg" class="text-[0.775rem] text-[var(--c-888888)]">{{ catMsg }}</p>

            <div class="flex items-center gap-2 flex-wrap">
              <button type="button" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="catSaving" @click="saveCategorizer">
                {{ catSaving ? 'Saving…' : 'Save settings' }}
              </button>
            </div>
            <p class="text-[0.72rem] text-[var(--c-585858)] leading-[1.5]">
              Auto-sort moves and flags mail in your live mailbox. Every action is recorded in the Logs window.
            </p>
          </template>
          </template>
        </section>

        <!-- Helpdesk card -->
        <section class="bg-[var(--c-111111)] border border-[var(--c-222222)] rounded-lg p-3.5 flex flex-col gap-3.5">
          <div class="flex items-center justify-between gap-4 cursor-pointer select-none -m-1 p-1 rounded" @click="toggleCard('helpdesk', !hdConfig.connected)">
            <div class="flex items-center gap-2.5">
              <svg class="text-[var(--c-7ab0ff)]" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8z"/>
              </svg>
              <div>
                <div class="text-[0.8125rem] text-[var(--c-d0d0d0)] font-medium">Helpdesk</div>
                <div class="text-[0.72rem] text-[var(--c-585858)]">Tickets, replies, and time logging from chat</div>
              </div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <div v-if="hdConfig.connected" class="text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap bg-[var(--c-0d2a1a)] text-[var(--c-4caf6e)] border border-[var(--c-1a4a2e)]" :title="hdConfig.email">
                Connected — {{ hdConfig.email }}
              </div>
              <div v-else class="text-[0.72rem] px-[0.55rem] py-[0.2rem] rounded-full whitespace-nowrap bg-surface text-[var(--c-484848)] border border-[var(--c-282828)]">
                Not connected
              </div>
              <svg :class="['text-[var(--c-505050)] transition-transform duration-150', cardOpen('helpdesk', !hdConfig.connected) ? 'rotate-180' : '']" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
            </div>
          </div>

          <form v-if="cardOpen('helpdesk', !hdConfig.connected)" class="flex flex-col gap-2 bg-[var(--c-0d0d0d)] p-3.5 rounded-lg border border-[var(--c-1e1e1e)]" @submit.prevent="connectHelpdesk">
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Helpdesk URL
              <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="hdForm.base_url" placeholder="https://helpdesk.example.com" required />
            </label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Agent email
              <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="hdForm.email" type="email" autocomplete="off" required />
            </label>
            <label class="flex flex-col gap-[0.2rem] text-[0.775rem] text-muted">
              Password
              <input class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] focus:outline-none focus:border-[var(--c-3a6adf)]" v-model="hdForm.password" type="password" autocomplete="new-password" :placeholder="hdConfig.connected ? 'Enter to reconnect' : ''" required />
            </label>
            <p class="text-[0.72rem] text-[var(--c-585858)] leading-[1.5]">
              The password is exchanged for an API token and never stored. Ticket creation, customer replies, and time logging ask for approval in chat before running (change under Settings → Tools).
            </p>
            <div class="flex items-center gap-2">
              <button type="submit" class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded-md px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] transition-[background] duration-[120ms] hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="hdSaving">
                {{ hdSaving ? 'Connecting…' : hdConfig.connected ? 'Reconnect' : 'Connect' }}
              </button>
              <button v-if="hdConfig.connected" type="button" class="bg-[var(--c-1e1010)] text-[var(--c-a06060)] border-none rounded px-3 py-1.5 cursor-pointer text-[0.8rem] font-[inherit] hover:bg-[var(--c-2a1515)] hover:text-[var(--c-d08080)]" @click="disconnectHelpdesk">Disconnect</button>
              <span v-if="hdMsg" class="text-[0.775rem]" :class="hdMsg === 'Connected.' ? 'text-[var(--c-4caf6e)]' : 'text-[var(--c-c06060)]'">{{ hdMsg }}</span>
            </div>
          </form>
        </section>
      </div>

      <!-- Appearance -->
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
      </div>

    </div>
  </div>
</template>
