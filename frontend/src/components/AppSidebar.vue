<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { RouterLink } from 'vue-router'
import { useWindowsStore } from '../stores/windows'
import SettingsPanel from './SettingsPanel.vue'
import Email from '../views/Email.vue'
import Logs from '../views/Logs.vue'
import Chat from '../views/Chat.vue'

const winStore = useWindowsStore()
const collapsed = ref(false)

onMounted(() => {
  const saved = localStorage.getItem('sidebar-collapsed')
  if (saved !== null) collapsed.value = saved === 'true'
})

function toggle() {
  collapsed.value = !collapsed.value
  localStorage.setItem('sidebar-collapsed', String(collapsed.value))
}

function openEmail() {
  winStore.open({
    key: 'email',
    title: 'Email',
    component: Email,
    width: 1100,
    height: 660,
  })
}

function openChat() {
  winStore.open({
    key: 'chat',
    title: 'Chat',
    component: Chat,
    width: 740,
    height: 560,
  })
}

function openLogs() {
  winStore.open({
    key: 'logs',
    title: 'Logs',
    component: Logs,
    width: 900,
    height: 520,
  })
}

function openSettings(tab: string) {
  winStore.open({
    key: 'settings',
    title: 'Settings',
    component: SettingsPanel,
    props: { initialTab: tab },
    width: 680,
  })
}


</script>

<template>
  <aside :class="['sidebar', { collapsed }]">
    <div class="top">
      <button class="icon-btn" title="Toggle sidebar" @click="toggle">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <line x1="3" y1="6" x2="21" y2="6"/>
          <line x1="3" y1="12" x2="21" y2="12"/>
          <line x1="3" y1="18" x2="21" y2="18"/>
        </svg>
      </button>
      <span v-if="!collapsed" class="brand">Episteme</span>
    </div>

    <nav>
      <!-- Chat -->
      <button class="nav-item new-chat-btn" :title="collapsed ? 'Chat' : ''" @click="openChat">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
        </svg>
        <span v-if="!collapsed">Chat</span>
      </button>

      <!-- History -->
      <RouterLink to="/chats" class="nav-item" :title="collapsed ? 'History' : ''">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>
        </svg>
        <span v-if="!collapsed">History</span>
      </RouterLink>

      <div class="nav-divider" />

      <!-- Email -->
      <button class="nav-item" :title="collapsed ? 'Email' : ''" @click="openEmail">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/>
          <polyline points="22,6 12,13 2,6"/>
        </svg>
        <span v-if="!collapsed">Email</span>
      </button>

      <!-- Calendar -->
      <RouterLink to="/calendar" class="nav-item" :title="collapsed ? 'Calendar' : ''">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/>
          <line x1="16" y1="2" x2="16" y2="6"/>
          <line x1="8" y1="2" x2="8" y2="6"/>
          <line x1="3" y1="10" x2="21" y2="10"/>
        </svg>
        <span v-if="!collapsed">Calendar</span>
      </RouterLink>

      <!-- Notes -->
      <RouterLink to="/notes" class="nav-item" :title="collapsed ? 'Notes' : ''">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
          <line x1="16" y1="13" x2="8" y2="13"/>
          <line x1="16" y1="17" x2="8" y2="17"/>
          <polyline points="10 9 9 9 8 9"/>
        </svg>
        <span v-if="!collapsed">Notes</span>
      </RouterLink>

      <!-- Logs -->
      <button class="nav-item" :title="collapsed ? 'Logs' : ''" @click="openLogs">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
          <line x1="16" y1="13" x2="8" y2="13"/>
          <line x1="16" y1="17" x2="8" y2="17"/>
          <line x1="10" y1="9" x2="8" y2="9"/>
        </svg>
        <span v-if="!collapsed">Logs</span>
      </button>

      <!-- Tasks -->
      <RouterLink to="/tasks" class="nav-item" :title="collapsed ? 'Tasks' : ''">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="9 11 12 14 22 4"/>
          <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>
        </svg>
        <span v-if="!collapsed">Tasks</span>
      </RouterLink>
    </nav>

    <div class="bottom">
      <div class="bottom-row">
        <button class="nav-item account-btn" :title="collapsed ? 'Account' : ''" @click="openSettings('account')">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/>
            <circle cx="12" cy="7" r="4"/>
          </svg>
          <span v-if="!collapsed">Account</span>
        </button>
        <button class="icon-btn cog-btn" title="Settings" @click="openSettings('providers')">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"/>
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
          </svg>
        </button>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  display: flex;
  flex-direction: column;
  width: 200px;
  min-width: 200px;
  background: #161616;
  border-right: 1px solid #2a2a2a;
  height: 100%;
  transition: width 0.2s ease, min-width 0.2s ease;
  overflow: hidden;
  flex-shrink: 0;
}

.sidebar.collapsed { width: 52px; min-width: 52px; }

.top {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  padding: 0.75rem 0.875rem;
  border-bottom: 1px solid #2a2a2a;
  min-height: 48px;
  flex-shrink: 0;
}

.brand {
  font-size: 0.875rem;
  font-weight: 700;
  color: #e0e0e0;
  white-space: nowrap;
  letter-spacing: -0.01em;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  background: none;
  border: none;
  cursor: pointer;
  color: #707070;
  padding: 0.25rem;
  border-radius: 0.25rem;
  flex-shrink: 0;
  transition: color 0.15s;
}

.icon-btn:hover { color: #e0e0e0; }

nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
  padding: 0.375rem;
  overflow-y: auto;
}

.nav-divider {
  height: 1px;
  background: #222;
  margin: 0.25rem 0.375rem;
  flex-shrink: 0;
}

.bottom {
  padding: 0.375rem;
  border-top: 1px solid #2a2a2a;
  flex-shrink: 0;
}

.bottom-row {
  display: flex;
  align-items: center;
  gap: 0.125rem;
}

.account-btn { flex: 1; min-width: 0; }
.cog-btn { flex-shrink: 0; padding: 0.5rem; }

.sidebar.collapsed .account-btn { display: none; }
.sidebar.collapsed .bottom-row { justify-content: center; }

.nav-item {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  padding: 0.5rem 0.625rem;
  border-radius: 0.375rem;
  color: #808080;
  text-decoration: none;
  font-size: 0.8125rem;
  background: none;
  border: none;
  cursor: pointer;
  width: 100%;
  text-align: left;
  transition: background 0.15s, color 0.15s;
  white-space: nowrap;
  font-family: inherit;
}

.nav-item:hover { background: #222; color: #e0e0e0; }
.nav-item.router-link-active { background: #222; color: #e0e0e0; }

.new-chat-btn {
  color: #5a9aff;
  margin-bottom: 0.125rem;
}

.new-chat-btn:hover { background: #1a2a4a; color: #7ab5ff; }
</style>
