<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useWindowsStore } from '../stores/windows'

const winStore = useWindowsStore()
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

const openEmail = () => winStore.openKey('email')
const openChats = () => winStore.openKey('chats')
const openChat = () => winStore.openKey('chat')
const openNotes = () => winStore.openKey('notes')
const openTasks = () => winStore.openKey('tasks')
const openMemories = () => winStore.openKey('memories')
const openCalendar = () => winStore.openKey('calendar')
const openLogs = () => winStore.openKey('logs')
const openSettings = (tab: string) => winStore.openKey('settings', { initialTab: tab })

</script>

<template>
  <!-- `sidebar` is a marker class queried by stores/windows.ts to measure the dock area -->
  <aside ref="sidebarEl" :class="['sidebar flex flex-col bg-[#161616] border-r border-raised h-full transition-[width,min-width] duration-200 ease-[ease] overflow-hidden shrink-0', collapsed ? 'w-[52px] min-w-[52px]' : 'w-[200px] min-w-[200px]']">
    <div class="flex items-center gap-2.5 px-3.5 py-3 border-b border-raised min-h-[48px] shrink-0">
      <button class="flex items-center justify-center bg-none border-none cursor-pointer text-[#707070] p-1 rounded shrink-0 transition-colors duration-150 hover:text-fg" title="Toggle sidebar" @click="toggle">
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
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] text-[#5a9aff] mb-0.5 hover:bg-[#1a2a4a] hover:text-[#7ab5ff]" :title="collapsed ? 'Chat' : ''" @click="openChat">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
        </svg>
        <span v-if="!collapsed">Chat</span>
      </button>

      <!-- History -->
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'History' : ''" @click="openChats">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>
        </svg>
        <span v-if="!collapsed">History</span>
      </button>

      <div class="h-px bg-[#222] mx-1.5 my-1 shrink-0" />

      <!-- Email -->
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'Email' : ''" @click="openEmail">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/>
          <polyline points="22,6 12,13 2,6"/>
        </svg>
        <span v-if="!collapsed">Email</span>
      </button>

      <!-- Calendar -->
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'Calendar' : ''" @click="openCalendar">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/>
          <line x1="16" y1="2" x2="16" y2="6"/>
          <line x1="8" y1="2" x2="8" y2="6"/>
          <line x1="3" y1="10" x2="21" y2="10"/>
        </svg>
        <span v-if="!collapsed">Calendar</span>
      </button>

      <!-- Notes -->
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'Notes' : ''" @click="openNotes">
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
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'Logs' : ''" @click="openLogs">
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
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'Tasks' : ''" @click="openTasks">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="9 11 12 14 22 4"/>
          <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>
        </svg>
        <span v-if="!collapsed">Tasks</span>
      </button>

      <!-- Memories -->
      <button class="flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg" :title="collapsed ? 'Memories' : ''" @click="openMemories">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 2a4.5 4.5 0 0 0-4.5 4.5c0 .5.08.98.23 1.42A3.5 3.5 0 0 0 6 14.5a3.5 3.5 0 0 0 1.5 2.87V18a2.5 2.5 0 0 0 5 0V4.5A2.5 2.5 0 0 0 12 2z"/>
          <path d="M12 2a4.5 4.5 0 0 1 4.5 4.5c0 .5-.08.98-.23 1.42A3.5 3.5 0 0 1 18 14.5a3.5 3.5 0 0 1-1.5 2.87V18a2.5 2.5 0 0 1-5 0"/>
        </svg>
        <span v-if="!collapsed">Memories</span>
      </button>
    </nav>

    <div class="p-1.5 border-t border-raised shrink-0">
      <div :class="['flex items-center gap-0.5', collapsed && 'justify-center']">
        <button :class="['flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[#808080] no-underline text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left transition-[background,color] duration-150 whitespace-nowrap font-[inherit] hover:bg-[#222] hover:text-fg flex-1 min-w-0', collapsed && 'hidden']" :title="collapsed ? 'Account' : ''" @click="openSettings('account')">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/>
            <circle cx="12" cy="7" r="4"/>
          </svg>
          <span v-if="!collapsed">Account</span>
        </button>
        <button class="flex items-center justify-center bg-none border-none cursor-pointer text-[#707070] rounded shrink-0 transition-colors duration-150 hover:text-fg p-2" title="Settings" @click="openSettings('providers')">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"/>
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
          </svg>
        </button>
      </div>
    </div>
  </aside>
</template>
