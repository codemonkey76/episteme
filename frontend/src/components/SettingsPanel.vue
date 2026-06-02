<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'

const logs = useLogsStore()

const props = defineProps<{ initialTab?: string }>()

type Tab = 'account' | 'providers' | 'mcp' | 'integrations' | 'appearance' | 'users' | 'system'
const activeTab = ref<Tab>((props.initialTab as Tab) ?? 'account')

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
  const [pRes, mRes] = await Promise.all([
    api.settings.listProviders(),
    api.settings.listMcpServers(),
  ])
  providers.value = pRes.providers
  mcpServers.value = mRes.mcp_servers
  await loadEmailConfig()
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

// ── Integrations ──────────────────────────────────────────────────────────────
const emailConfig = ref<api.EmailConfigStatus>({
  configured: false, connected: false, tenant_id: '', client_id: '', connected_email: null,
})
const emailForm = ref({ tenant_id: '', client_id: '', client_secret: '' })
const emailMsg = ref('')
const emailSaving = ref(false)

const callbackUri = computed(() => window.location.origin + '/api/integrations/email/callback')

async function loadEmailConfig() {
  const cfg = await api.integrations.email.getConfig()
  emailConfig.value = cfg
  emailForm.value.tenant_id = cfg.tenant_id
  emailForm.value.client_id = cfg.client_id
  emailForm.value.client_secret = ''
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
  await api.auth.logout()
  window.location.href = '/login'
}
</script>

<template>
  <div class="settings-panel">
    <!-- Vertical nav -->
    <nav class="side-nav">
      <button :class="['nav-item', { active: activeTab === 'account' }]" @click="activeTab = 'account'">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>
        </svg>
        Account
      </button>
      <button :class="['nav-item', { active: activeTab === 'providers' }]" @click="activeTab = 'providers'">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
        </svg>
        Providers
      </button>
      <button :class="['nav-item', { active: activeTab === 'mcp' }]" @click="activeTab = 'mcp'">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>
        </svg>
        MCP Servers
      </button>
      <button :class="['nav-item', { active: activeTab === 'integrations' }]" @click="activeTab = 'integrations'">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>
        </svg>
        Integrations
      </button>
      <button :class="['nav-item', { active: activeTab === 'appearance' }]" @click="activeTab = 'appearance'">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
        Appearance
      </button>

      <div class="nav-group">
        <span class="nav-group-label">Admin</span>
        <button :class="['nav-item', { active: activeTab === 'users' }]" @click="activeTab = 'users'">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>
          </svg>
          Users
        </button>
        <button :class="['nav-item', { active: activeTab === 'system' }]" @click="activeTab = 'system'">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/>
          </svg>
          System
        </button>
      </div>
    </nav>

    <!-- Content -->
    <div class="content">

      <!-- Account -->
      <div v-if="activeTab === 'account'" class="tab-content">
        <h2>Account</h2>

        <section>
          <h3>Change password</h3>
          <form class="form" @submit.prevent="changePassword">
            <label>Current password <input v-model="passwordForm.current" type="password" autocomplete="current-password" /></label>
            <label>New password <input v-model="passwordForm.next" type="password" autocomplete="new-password" /></label>
            <label>Confirm new password <input v-model="passwordForm.confirm" type="password" autocomplete="new-password" /></label>
            <p v-if="passwordMsg" class="msg">{{ passwordMsg }}</p>
            <button type="submit" class="btn-primary">Update password</button>
          </form>
        </section>

        <section>
          <h3>Two-factor authentication</h3>
          <div class="row">
            <div>
              <div class="row-title">Authenticator app</div>
              <div class="row-sub">{{ twoFactorEnabled ? 'Active' : 'Not configured' }}</div>
            </div>
            <button class="btn-secondary" @click="toggleTwoFactor">
              {{ twoFactorEnabled ? 'Disable' : 'Enable' }}
            </button>
          </div>
          <p v-if="accountMsg" class="msg">{{ accountMsg }}</p>
        </section>

        <section class="section-danger">
          <h3>Danger zone</h3>
          <button class="btn-danger" @click="logout">Sign out</button>
        </section>
      </div>

      <!-- Providers -->
      <div v-else-if="activeTab === 'providers'" class="tab-content">
        <h2>Model Providers</h2>

        <section>
          <ul v-if="providers.length" class="item-list">
            <li v-for="p in providers" :key="p.name">
              <div class="item-info">
                <span class="item-name">{{ p.name }}</span>
                <span class="item-sub">{{ p.provider }} · {{ p.model_id }}</span>
              </div>
              <button class="btn-remove" @click="deleteProvider(p.name)">Remove</button>
            </li>
          </ul>
          <p v-else class="empty">No providers configured.</p>
        </section>

        <section>
          <h3>Add / update provider</h3>
          <form class="form" @submit.prevent="saveProvider">
            <label>Name <input v-model="newProvider.name" placeholder="e.g. my-ollama" required /></label>
            <label>
              Provider
              <select v-model="newProvider.provider" @change="onProviderTypeChange">
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
              <label>
                Ollama URL
                <div class="input-row">
                  <input v-model="newProvider.base_url" placeholder="http://192.168.1.x:11434" />
                  <button type="button" class="btn-fetch" :disabled="!newProvider.base_url || ollamaFetching" @click="fetchOllamaModels">
                    {{ ollamaFetching ? '…' : 'Fetch models' }}
                  </button>
                </div>
              </label>
              <p v-if="ollamaFetchError" class="msg error">{{ ollamaFetchError }}</p>
              <label v-if="ollamaModels.length">
                Model
                <select v-model="newProvider.model_id">
                  <option v-for="m in ollamaModels" :key="m" :value="m">{{ m }}</option>
                </select>
              </label>
              <label v-else-if="!ollamaFetching">
                Model ID
                <input v-model="newProvider.model_id" placeholder="Fetch models above, or type manually" />
              </label>
            </template>

            <!-- All other providers -->
            <template v-else>
              <label>Model ID <input v-model="newProvider.model_id" required /></label>
              <label v-if="newProvider.provider === 'openai_compatible'">
                Base URL <input v-model="newProvider.base_url" placeholder="https://…/v1" />
              </label>
              <label v-if="newProvider.provider !== 'ollama'">
                API Key <input v-model="newProvider.api_key" type="password" />
              </label>
            </template>

            <button type="submit" class="btn-primary" :disabled="!newProvider.name || !newProvider.model_id">Save</button>
          </form>
        </section>
      </div>

      <!-- MCP Servers -->
      <div v-else-if="activeTab === 'mcp'" class="tab-content">
        <h2>MCP Servers</h2>

        <section>
          <ul v-if="mcpServers.length" class="item-list">
            <li v-for="s in mcpServers" :key="s.name">
              <div class="item-info">
                <span class="item-name">{{ s.name }}</span>
                <span class="item-sub">{{ s.transport.type }}</span>
              </div>
            </li>
          </ul>
          <p v-else class="empty">No MCP servers configured.</p>
        </section>
      </div>

      <!-- Integrations -->
      <div v-else-if="activeTab === 'integrations'" class="tab-content">
        <h2>Integrations</h2>

        <!-- Microsoft 365 card -->
        <section class="integration-card">
          <div class="integration-header">
            <div class="integration-identity">
              <!-- Microsoft "four squares" logo -->
              <svg width="20" height="20" viewBox="0 0 21 21" xmlns="http://www.w3.org/2000/svg">
                <rect x="1" y="1" width="9" height="9" fill="#f25022"/>
                <rect x="11" y="1" width="9" height="9" fill="#7fba00"/>
                <rect x="1" y="11" width="9" height="9" fill="#00a4ef"/>
                <rect x="11" y="11" width="9" height="9" fill="#ffb900"/>
              </svg>
              <div>
                <div class="integration-name">Microsoft 365</div>
                <div class="integration-sub">Work / School account via Entra ID</div>
              </div>
            </div>
            <div v-if="emailConfig.connected" class="badge badge-connected">
              <span class="badge-dot"></span>
              {{ emailConfig.connected_email ?? 'Connected' }}
            </div>
            <div v-else-if="emailConfig.configured" class="badge badge-configured">
              Credentials saved
            </div>
            <div v-else class="badge badge-none">
              Not configured
            </div>
          </div>

          <!-- Setup instructions -->
          <details class="instructions">
            <summary>Setup instructions</summary>
            <ol class="steps">
              <li>
                Sign in to <strong>portal.azure.com</strong> with your Microsoft 365 admin account.
              </li>
              <li>
                Go to <strong>Microsoft Entra ID → App registrations → New registration</strong>.
              </li>
              <li>
                Set a name (e.g. <em>Episteme</em>), and for <em>Supported account types</em> choose
                <strong>"Accounts in any organizational directory (Any Microsoft Entra ID tenant)"</strong>.
              </li>
              <li>
                Under <em>Redirect URI</em>, select platform <strong>Web</strong> and enter:
                <code class="uri">{{ callbackUri }}</code>
              </li>
              <li>
                Click <strong>Register</strong>. From the overview page copy:
                <ul>
                  <li><strong>Application (client) ID</strong> → paste into <em>Client ID</em> below</li>
                  <li><strong>Directory (tenant) ID</strong> → paste into <em>Tenant ID</em> below</li>
                </ul>
              </li>
              <li>
                Go to <strong>Certificates &amp; secrets → Client secrets → New client secret</strong>.
                Set a description and expiry, then copy the <strong>Value</strong> (not the Secret ID)
                → paste into <em>Client Secret</em> below.
              </li>
              <li>
                Go to <strong>API permissions → Add a permission → Microsoft Graph → Delegated permissions</strong>
                and add:
                <ul>
                  <li><code>Mail.Read</code></li>
                  <li><code>Mail.ReadWrite</code></li>
                  <li><code>Mail.Send</code></li>
                  <li><code>User.Read</code></li>
                </ul>
              </li>
              <li>
                Click <strong>Grant admin consent</strong> for your organisation (requires admin role),
                or ask your tenant administrator to do so.
              </li>
            </ol>
          </details>

          <!-- Credentials form -->
          <form class="form" @submit.prevent="saveEmailConfig">
            <label>
              Tenant ID (Directory ID)
              <input v-model="emailForm.tenant_id" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" required />
            </label>
            <label>
              Client ID (Application ID)
              <input v-model="emailForm.client_id" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" required />
            </label>
            <label>
              Client Secret
              <input
                v-model="emailForm.client_secret"
                type="password"
                autocomplete="new-password"
                :placeholder="emailConfig.configured ? 'Leave blank to keep existing secret' : 'Paste secret value here'"
              />
            </label>
            <p v-if="emailMsg" class="msg">{{ emailMsg }}</p>
            <div class="form-actions">
              <button type="submit" class="btn-primary" :disabled="emailSaving">
                {{ emailSaving ? 'Saving…' : 'Save credentials' }}
              </button>
              <button
                v-if="emailConfig.configured && !emailConfig.connected"
                type="button"
                class="btn-connect"
                @click="connectEmail"
              >
                Connect Microsoft 365 →
              </button>
              <button
                v-if="emailConfig.connected"
                type="button"
                class="btn-danger"
                @click="disconnectEmail"
              >
                Disconnect
              </button>
            </div>
            <p v-if="emailConfig.configured && !emailConfig.connected" class="hint">
              After saving credentials, click <em>Connect</em> to authorise via Microsoft login.
              You'll be redirected back here when done.
            </p>
          </form>
        </section>
      </div>

      <!-- Appearance -->
      <div v-else-if="activeTab === 'appearance'" class="tab-content">
        <h2>Appearance</h2>
        <section>
          <p class="empty">Appearance options coming soon.</p>
        </section>
      </div>

      <!-- Users -->
      <div v-else-if="activeTab === 'users'" class="tab-content">
        <h2>Users</h2>
        <section>
          <p class="empty">User management coming soon.</p>
        </section>
      </div>

      <!-- System -->
      <div v-else-if="activeTab === 'system'" class="tab-content">
        <h2>System</h2>
        <section>
          <p class="empty">System information coming soon.</p>
        </section>
      </div>

    </div>
  </div>
</template>

<style scoped>
.settings-panel {
  display: flex;
  flex-direction: row;
  height: 100%;
  min-height: 420px;
}

/* ── Side nav ── */
.side-nav {
  width: 156px;
  min-width: 156px;
  background: #141414;
  border-right: 1px solid #252525;
  display: flex;
  flex-direction: column;
  padding: 0.5rem;
  gap: 0.125rem;
  overflow-y: auto;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.45rem 0.75rem;
  border-radius: 0.375rem;
  color: #808080;
  font-size: 0.8125rem;
  background: none;
  border: none;
  cursor: pointer;
  width: 100%;
  text-align: left;
  font-family: inherit;
  transition: background 0.12s, color 0.12s;
  white-space: nowrap;
}

.nav-item svg { flex-shrink: 0; }

.nav-item:hover { background: #1e1e1e; color: #d0d0d0; }
.nav-item.active { background: #222; color: #e0e0e0; }

.nav-group {
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  border-top: 1px solid #252525;
}

.nav-group-label {
  font-size: 0.65rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: #484848;
  padding: 0.25rem 0.75rem 0.375rem;
  pointer-events: none;
}

/* ── Content ── */
.content {
  flex: 1;
  overflow-y: auto;
  min-width: 0;
}

.tab-content {
  padding: 1.25rem 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

h2 {
  font-size: 0.9375rem;
  font-weight: 600;
  color: #e0e0e0;
}

section {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

h3 {
  font-size: 0.7rem;
  font-weight: 600;
  color: #585858;
  text-transform: uppercase;
  letter-spacing: 0.07em;
}

/* ── Forms ── */
.form {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  background: #111;
  padding: 0.875rem;
  border-radius: 0.5rem;
  border: 1px solid #222;
}

label {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  font-size: 0.775rem;
  color: #909090;
}

input, select {
  background: #1a1a1a;
  color: #e0e0e0;
  border: 1px solid #2a2a2a;
  border-radius: 0.25rem;
  padding: 0.375rem 0.5rem;
  font-size: 0.8125rem;
  font-family: inherit;
}

input:focus, select:focus {
  outline: none;
  border-color: #3a6adf;
}

/* ── Buttons ── */
.btn-primary {
  background: #1e3a6e;
  color: #7ab0ff;
  border: 1px solid #2a4a8a;
  border-radius: 0.375rem;
  padding: 0.375rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  align-self: flex-start;
  font-family: inherit;
  transition: background 0.12s;
}
.btn-primary:hover { background: #254880; }

.btn-secondary {
  background: #1e1e1e;
  color: #c0c0c0;
  border: 1px solid #303030;
  border-radius: 0.375rem;
  padding: 0.375rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  font-family: inherit;
  transition: background 0.12s;
}
.btn-secondary:hover { background: #282828; }

.btn-danger {
  background: #2a1010;
  color: #ff7070;
  border: 1px solid #4a1a1a;
  border-radius: 0.375rem;
  padding: 0.375rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  align-self: flex-start;
  font-family: inherit;
  transition: background 0.12s;
}
.btn-danger:hover { background: #3a1515; }

.btn-remove {
  background: #1e1010;
  color: #a06060;
  border: none;
  border-radius: 0.25rem;
  padding: 0.2rem 0.5rem;
  cursor: pointer;
  font-size: 0.75rem;
  font-family: inherit;
  flex-shrink: 0;
}
.btn-remove:hover { background: #2a1515; color: #d08080; }

/* ── Row layout ── */
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: #111;
  border: 1px solid #222;
  border-radius: 0.5rem;
  padding: 0.75rem 0.875rem;
  gap: 1rem;
}

.row-title { font-size: 0.8125rem; color: #d0d0d0; }
.row-sub { font-size: 0.75rem; color: #585858; margin-top: 0.1rem; }

/* ── Lists ── */
.item-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

.item-list li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  background: #111;
  border: 1px solid #222;
  border-radius: 0.375rem;
  padding: 0.5rem 0.75rem;
  gap: 1rem;
}

.item-info {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
  min-width: 0;
}

.item-name { font-size: 0.8125rem; color: #d0d0d0; }
.item-sub { font-size: 0.75rem; color: #585858; }

/* ── Misc ── */
.empty { color: #484848; font-size: 0.8125rem; }
.msg { font-size: 0.775rem; color: #888; }
.msg.error { color: #c06060; }

.input-row {
  display: flex;
  gap: 0.375rem;
}
.input-row input { flex: 1; }

.btn-fetch {
  background: #1a1a2a;
  color: #7090d0;
  border: 1px solid #2a2a4a;
  border-radius: 0.25rem;
  padding: 0.375rem 0.6rem;
  cursor: pointer;
  font-size: 0.775rem;
  font-family: inherit;
  white-space: nowrap;
  transition: background 0.12s;
  flex-shrink: 0;
}
.btn-fetch:hover:not(:disabled) { background: #222240; }
.btn-fetch:disabled { opacity: 0.4; cursor: default; }
.section-danger { border-top: 1px solid #222; padding-top: 1.25rem; }

/* ── Integration cards ── */
.integration-card {
  background: #111;
  border: 1px solid #222;
  border-radius: 0.5rem;
  padding: 0.875rem;
  display: flex;
  flex-direction: column;
  gap: 0.875rem;
}

.integration-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}

.integration-identity {
  display: flex;
  align-items: center;
  gap: 0.625rem;
}

.integration-name { font-size: 0.8125rem; color: #d0d0d0; font-weight: 500; }
.integration-sub  { font-size: 0.72rem; color: #585858; margin-top: 0.1rem; }

/* Status badges */
.badge {
  font-size: 0.72rem;
  padding: 0.2rem 0.55rem;
  border-radius: 999px;
  white-space: nowrap;
  flex-shrink: 0;
}
.badge-connected {
  background: #0d2a1a;
  color: #4ec97a;
  border: 1px solid #1a4030;
  display: flex;
  align-items: center;
  gap: 0.35rem;
}
.badge-dot {
  width: 6px; height: 6px;
  border-radius: 50%;
  background: #4ec97a;
  flex-shrink: 0;
}
.badge-configured {
  background: #1a1a0d;
  color: #b0a030;
  border: 1px solid #3a3010;
}
.badge-none {
  background: #1a1a1a;
  color: #484848;
  border: 1px solid #282828;
}

/* Setup instructions */
.instructions {
  border: 1px solid #222;
  border-radius: 0.375rem;
  overflow: hidden;
}

.instructions summary {
  padding: 0.45rem 0.75rem;
  font-size: 0.775rem;
  color: #707070;
  cursor: pointer;
  user-select: none;
  list-style: none;
}
.instructions summary::-webkit-details-marker { display: none; }
.instructions summary::before {
  content: '▶ ';
  font-size: 0.6rem;
  opacity: 0.5;
}
details[open] .instructions summary::before,
.instructions[open] summary::before {
  content: '▼ ';
}
.instructions summary:hover { color: #a0a0a0; }

.steps {
  padding: 0.75rem 0.875rem 0.875rem 1.75rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  font-size: 0.775rem;
  color: #909090;
  line-height: 1.5;
  border-top: 1px solid #1e1e1e;
}
.steps li { padding-left: 0.25rem; }
.steps strong { color: #c0c0c0; }
.steps em { color: #a0a0a0; font-style: normal; }
.steps ul {
  margin-top: 0.35rem;
  padding-left: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  list-style: disc;
}
.steps code {
  font-family: ui-monospace, monospace;
  font-size: 0.75rem;
  background: #1a1a1a;
  padding: 0.05rem 0.3rem;
  border-radius: 0.2rem;
  color: #a0c8ff;
}
.uri {
  display: block;
  margin-top: 0.3rem;
  font-family: ui-monospace, monospace;
  font-size: 0.75rem;
  background: #1a1a1a;
  padding: 0.3rem 0.5rem;
  border-radius: 0.25rem;
  color: #a0c8ff;
  word-break: break-all;
}

/* form-actions row */
.form-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.btn-connect {
  background: #0d2a1a;
  color: #4ec97a;
  border: 1px solid #1a4030;
  border-radius: 0.375rem;
  padding: 0.375rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  font-family: inherit;
  transition: background 0.12s;
}
.btn-connect:hover { background: #122e1e; }

.hint {
  font-size: 0.72rem;
  color: #585858;
  line-height: 1.5;
}
</style>
