<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useWindowsStore } from '../stores/windows'
import { useAuthStore } from '../stores/auth'

const winStore = useWindowsStore()
const auth = useAuthStore()
const collapsed = ref(false)
const sidebarEl = ref<HTMLElement>()

let resizeObserver: ResizeObserver | null = null

onMounted(() => {
  const saved = localStorage.getItem('sidebar-collapsed')
  if (saved !== null) collapsed.value = saved === 'true'

  // Reflow docked windows whenever the sidebar's width changes (including
  // smoothly through the collapse/expand transition) so they track the edge.
  if (sidebarEl.value) {
    resizeObserver = new ResizeObserver(() => winStore.reflow())
    resizeObserver.observe(sidebarEl.value)
  }
})

onUnmounted(() => resizeObserver?.disconnect())

function toggle() {
  collapsed.value = !collapsed.value
  localStorage.setItem('sidebar-collapsed', String(collapsed.value))
}

// Nav icons are toggles: pressed while the window is open; clicking again
// (or closing the window itself) releases them.
const isOpen = (key: string) => winStore.windows.some(w => w.key === key)

function toggleKey(key: string) {
  const open = winStore.windows.filter(w => w.key === key)
  if (open.length > 0) {
    open.forEach(w => winStore.close(w.id))
  } else {
    winStore.openKey(key)
  }
}

function toggleSettings(tab: string) {
  const open = winStore.windows.filter(w => w.key === 'settings')
  if (open.length > 0) {
    open.forEach(w => winStore.close(w.id))
  } else {
    winStore.openKey('settings', { initialTab: tab })
  }
}

</script>

<template>
  <!-- `sidebar` is a marker class queried by stores/windows.ts to measure the dock area -->
  <aside ref="sidebarEl" :class="['sidebar flex flex-col bg-[var(--c-161616)] border-r border-raised h-full transition-[width,min-width] duration-200 ease-[ease] overflow-hidden shrink-0', collapsed ? 'w-[52px] min-w-[52px]' : 'w-[200px] min-w-[200px]']">
    <div class="flex items-center gap-2.5 px-3.5 py-3 border-b border-raised min-h-[48px] shrink-0">
      <button class="flex items-center justify-center bg-none border-none cursor-pointer text-[var(--c-707070)] p-1 rounded shrink-0 transition-colors duration-150 hover:text-fg" title="Toggle sidebar" @click="toggle">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <line x1="3" y1="6" x2="21" y2="6"/>
          <line x1="3" y1="12" x2="21" y2="12"/>
          <line x1="3" y1="18" x2="21" y2="18"/>
        </svg>
      </button>
      <span v-if="!collapsed" class="text-sm font-bold text-fg whitespace-nowrap tracking-[-0.01em]">Episteme</span>
    </div>

    <nav class="flex-1 flex flex-col gap-0.5 p-1.5 overflow-y-auto">
      <!-- Chat -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] mb-0.5 hover:bg-[var(--c-1a2a4a)] hover:text-[var(--c-7ab5ff)]', isOpen('chat') ? 'bg-[var(--c-1a2a4a)] text-[var(--c-7ab5ff)]' : 'text-[var(--c-5a9aff)]']" :title="collapsed ? 'Chat' : ''" :aria-pressed="isOpen('chat')" @click="toggleKey('chat')">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
        </svg>
        <span v-if="!collapsed">Chat</span>
      </button>

      <!-- History -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('chats') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'History' : ''" :aria-pressed="isOpen('chats')" @click="toggleKey('chats')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>
        </svg>
        <span v-if="!collapsed">History</span>
      </button>

      <div class="h-px bg-[var(--c-222222)] mx-1.5 my-1 shrink-0" />

      <!-- Email -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('email') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Email' : ''" :aria-pressed="isOpen('email')" @click="toggleKey('email')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/>
          <polyline points="22,6 12,13 2,6"/>
        </svg>
        <span v-if="!collapsed">Email</span>
      </button>

      <!-- Calendar -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('calendar') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Calendar' : ''" :aria-pressed="isOpen('calendar')" @click="toggleKey('calendar')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/>
          <line x1="16" y1="2" x2="16" y2="6"/>
          <line x1="8" y1="2" x2="8" y2="6"/>
          <line x1="3" y1="10" x2="21" y2="10"/>
        </svg>
        <span v-if="!collapsed">Calendar</span>
      </button>

      <!-- Notes -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('notes') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Notes' : ''" :aria-pressed="isOpen('notes')" @click="toggleKey('notes')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
          <line x1="16" y1="13" x2="8" y2="13"/>
          <line x1="16" y1="17" x2="8" y2="17"/>
          <polyline points="10 9 9 9 8 9"/>
        </svg>
        <span v-if="!collapsed">Notes</span>
      </button>

      <!-- Logs -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('logs') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Logs' : ''" :aria-pressed="isOpen('logs')" @click="toggleKey('logs')">
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
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('tasks') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Tasks' : ''" :aria-pressed="isOpen('tasks')" @click="toggleKey('tasks')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="9 11 12 14 22 4"/>
          <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>
        </svg>
        <span v-if="!collapsed">Tasks</span>
      </button>

      <!-- Documents -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('documents') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Documents' : ''" :aria-pressed="isOpen('documents')" @click="toggleKey('documents')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
          <line x1="16" y1="13" x2="8" y2="13"/>
          <line x1="16" y1="17" x2="8" y2="17"/>
        </svg>
        <span v-if="!collapsed">Documents</span>
      </button>

      <!-- Jobs -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('jobs') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Jobs' : ''" :aria-pressed="isOpen('jobs')" @click="toggleKey('jobs')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"/>
          <path d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83"/>
        </svg>
        <span v-if="!collapsed">Jobs</span>
      </button>

      <!-- Reports -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('reports') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Reports' : ''" :aria-pressed="isOpen('reports')" @click="toggleKey('reports')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="7"/>
          <line x1="21" y1="21" x2="16.65" y2="16.65"/>
          <line x1="8.5" y1="10" x2="13.5" y2="10"/>
          <line x1="8.5" y1="13" x2="12" y2="13"/>
        </svg>
        <span v-if="!collapsed">Reports</span>
      </button>

      <!-- Memories -->
      <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('memories') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Memories' : ''" :aria-pressed="isOpen('memories')" @click="toggleKey('memories')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 2a4.5 4.5 0 0 0-4.5 4.5c0 .5.08.98.23 1.42A3.5 3.5 0 0 0 6 14.5a3.5 3.5 0 0 0 1.5 2.87V18a2.5 2.5 0 0 0 5 0V4.5A2.5 2.5 0 0 0 12 2z"/>
          <path d="M12 2a4.5 4.5 0 0 1 4.5 4.5c0 .5-.08.98-.23 1.42A3.5 3.5 0 0 1 18 14.5a3.5 3.5 0 0 1-1.5 2.87V18a2.5 2.5 0 0 1-5 0"/>
        </svg>
        <span v-if="!collapsed">Memories</span>
      </button>

      <!-- Prompts (admin: the model prompts are instance-wide) -->
      <button v-if="auth.role === 'admin'" :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg', isOpen('prompts') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]']" :title="collapsed ? 'Prompts' : ''" :aria-pressed="isOpen('prompts')" @click="toggleKey('prompts')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="4 17 10 11 4 5"/>
          <line x1="12" y1="19" x2="20" y2="19"/>
        </svg>
        <span v-if="!collapsed">Prompts</span>
      </button>
    </nav>

    <div class="p-1.5 border-t border-raised shrink-0">
      <div :class="['flex items-center gap-0.5', collapsed && 'justify-center']">
        <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[var(--c-222222)] hover:text-fg flex-1 min-w-0', isOpen('settings') ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)]', collapsed && 'hidden']" :title="collapsed ? 'Account' : ''" @click="toggleSettings('account')">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/>
            <circle cx="12" cy="7" r="4"/>
          </svg>
          <span v-if="!collapsed">Account</span>
        </button>
        <button :class="['flex items-center justify-center bg-none border-none cursor-pointer rounded shrink-0 transition-colors duration-150 hover:text-fg p-2', isOpen('settings') ? 'text-fg bg-[var(--c-222222)]' : 'text-[var(--c-707070)]']" title="Settings" @click="toggleSettings('providers')">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"/>
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
          </svg>
        </button>
      </div>
    </div>
  </aside>
</template>
