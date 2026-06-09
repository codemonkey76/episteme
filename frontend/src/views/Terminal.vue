<script setup lang="ts">
import { ref, watch } from 'vue'
import TerminalTab from './TerminalTab.vue'

type Shell = 'bash' | 'pwsh'
interface Tab { id: string; num: number; title: string; shell: Shell }

// Tabs persist across refresh so each terminal keeps its identity (and thus its
// restorable scrollback). Only the lightweight tab metadata is stored here; the
// scrollback itself lives durably in the backend, keyed by the tab id.
const STORE_KEY = 'episteme.terminal.tabs'
interface Persisted { tabs: Tab[]; activeId: string; counter: number }

let counter = 0
const tabs = ref<Tab[]>([])
const activeId = ref('')
// Shared across tabs so the sidebar width stays consistent when switching.
const sidebarWidth = ref(340)

function addTab() {
  const id = crypto.randomUUID()
  const num = ++counter
  tabs.value.push({ id, num, title: `bash ${num}`, shell: 'bash' })
  activeId.value = id
}

function closeTab(id: string) {
  const i = tabs.value.findIndex((t) => t.id === id)
  if (i < 0) return
  tabs.value.splice(i, 1)
  if (tabs.value.length === 0) {
    addTab()
    return
  }
  if (activeId.value === id) {
    activeId.value = tabs.value[Math.max(0, i - 1)].id
  }
}

function setShell(id: string, shell: Shell) {
  const t = tabs.value.find((x) => x.id === id)
  if (t) {
    t.shell = shell
    t.title = `${shell === 'pwsh' ? 'PowerShell' : 'bash'} ${t.num}`
  }
}

function restore(): boolean {
  try {
    const raw = localStorage.getItem(STORE_KEY)
    if (!raw) return false
    const p = JSON.parse(raw) as Persisted
    if (!p.tabs?.length) return false
    tabs.value = p.tabs
    counter = p.counter ?? p.tabs.length
    activeId.value = p.tabs.some((t) => t.id === p.activeId) ? p.activeId : p.tabs[0].id
    return true
  } catch {
    return false
  }
}

watch(
  [tabs, activeId],
  () => {
    localStorage.setItem(
      STORE_KEY,
      JSON.stringify({ tabs: tabs.value, activeId: activeId.value, counter } satisfies Persisted),
    )
  },
  { deep: true },
)

if (!restore()) addTab()
</script>

<template>
  <div class="flex flex-col h-full bg-[#0d0d0d] overflow-hidden">
    <!-- Tab bar -->
    <div class="flex items-center gap-1 px-1.5 py-1 border-b border-[var(--c-1e1e1e)] shrink-0 overflow-x-auto">
      <button
        v-for="t in tabs"
        :key="t.id"
        class="group flex items-center gap-1.5 px-2.5 py-1 rounded text-[0.74rem] whitespace-nowrap shrink-0"
        :class="t.id === activeId ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1a1a1a)]'"
        @click="activeId = t.id"
      >
        {{ t.title }}
        <span
          class="opacity-50 hover:opacity-100 hover:text-[var(--c-df7a7a)] px-0.5"
          title="Close session"
          @click.stop="closeTab(t.id)"
        >×</span>
      </button>
      <button
        class="shrink-0 px-2 py-1 rounded text-[0.85rem] text-[var(--c-808080)] hover:bg-[var(--c-222222)] hover:text-fg"
        title="New session"
        @click="addTab"
      >+</button>
    </div>

    <!-- Tab panes: all kept alive (v-show) so sessions persist when switching. -->
    <div class="flex-1 min-h-0 relative">
      <TerminalTab
        v-for="t in tabs"
        v-show="t.id === activeId"
        :key="t.id"
        class="absolute inset-0"
        :active="t.id === activeId"
        :tab-id="t.id"
        :initial-shell="t.shell"
        :sidebar-width="sidebarWidth"
        @update:sidebar-width="sidebarWidth = $event"
        @shell="setShell(t.id, $event)"
      />
    </div>
  </div>
</template>
