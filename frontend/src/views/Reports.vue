<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import * as api from '../api'

const reports = ref<api.Report[]>([])
const selected = ref<api.Report | null>(null)
const loading = ref(false)
const error = ref('')

async function load() {
  loading.value = true
  try {
    const res = await api.reports.list()
    reports.value = res.reports
    // Keep the selection if it still exists; otherwise pick the newest.
    if (selected.value && !res.reports.some(r => r.id === selected.value!.id)) {
      selected.value = null
    }
    if (!selected.value && res.reports.length > 0) {
      selected.value = res.reports[0]
    }
    error.value = ''
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load reports'
  } finally {
    loading.value = false
  }
}

// Poll while open so freshly finished research appears without a refresh.
let poll: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  load()
  poll = setInterval(load, 5000)
})
onBeforeUnmount(() => {
  if (poll) clearInterval(poll)
})

function openInTab(r: api.Report) {
  window.open(api.reports.htmlPath(r.id), '_blank')
}

async function remove(r: api.Report) {
  try {
    await api.reports.remove(r.id)
    reports.value = reports.value.filter(x => x.id !== r.id)
    if (selected.value?.id === r.id) selected.value = reports.value[0] ?? null
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete report'
  }
}

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })
}

// ── New research launcher ────────────────────────────────────────────────────
const topic = ref('')
const depth = ref<'quick' | 'standard' | 'deep'>('standard')
const starting = ref(false)
const startedNote = ref('')

async function startResearch() {
  const t = topic.value.trim()
  if (!t || starting.value) return
  starting.value = true
  error.value = ''
  try {
    await api.reports.startResearch(t, depth.value)
    startedNote.value = `Researching "${t}" — the report will appear here when ready.`
    topic.value = ''
    setTimeout(() => (startedNote.value = ''), 8000)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to start research'
  } finally {
    starting.value = false
  }
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- New research launcher -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0">
      <input
        v-model="topic"
        class="flex-1 bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none min-w-0 focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]"
        placeholder="Research a topic… (e.g. compare Frigate vs Blue Iris for a self-hosted NVR)"
        @keydown.enter="startResearch"
      />
      <select v-model="depth" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1.5 text-xs font-[inherit] cursor-pointer">
        <option value="quick">quick</option>
        <option value="standard">standard</option>
        <option value="deep">deep</option>
      </select>
      <button
        class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1.5 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50 whitespace-nowrap"
        :disabled="starting || !topic.trim()"
        @click="startResearch"
      >{{ starting ? 'Starting…' : 'Research' }}</button>
    </div>
    <div v-if="startedNote" class="px-3 py-1.5 text-[0.75rem] text-[var(--c-7adfbb)] border-b border-[var(--c-1e1e1e)] shrink-0">{{ startedNote }}</div>

    <div class="flex flex-1 min-h-0">
    <!-- Report list -->
    <div class="w-[16rem] shrink-0 border-r border-[var(--c-1e1e1e)] flex flex-col">
      <div v-if="error" class="px-3 py-2 text-danger text-[0.75rem] border-b border-[var(--c-1e1e1e)]">{{ error }}</div>
      <div class="flex-1 overflow-y-auto">
        <div v-if="reports.length === 0" class="px-3 py-4 text-[var(--c-383838)] text-[0.8rem] leading-[1.5]">
          No reports yet. Ask the assistant to "deep research" a topic and the result lands here.
        </div>
        <button
          v-for="r in reports"
          :key="r.id"
          :class="['w-full text-left px-3 py-2.5 border-b border-[var(--c-161616)] cursor-pointer bg-none border-x-0 border-t-0 font-[inherit]', selected?.id === r.id ? 'bg-[var(--c-1a1a1a)]' : 'hover:bg-[var(--c-131313)]']"
          @click="selected = r"
        >
          <p :class="['text-[0.8125rem] leading-[1.35] break-words', selected?.id === r.id ? 'text-fg' : 'text-[var(--c-c0c0c0)]']">{{ r.title }}</p>
          <span class="text-[0.68rem] text-[var(--c-505050)]">{{ fmtDate(r.created_at) }}</span>
        </button>
      </div>
      <div class="px-3 py-[0.25rem] border-t border-surface text-[var(--c-505050)] text-[0.68rem] shrink-0">
        {{ reports.length }} report{{ reports.length === 1 ? '' : 's' }}
      </div>
    </div>

    <!-- Preview -->
    <div class="flex-1 flex flex-col min-w-0">
      <template v-if="selected">
        <div class="flex items-center gap-2 px-3 py-1.5 border-b border-[var(--c-1e1e1e)] shrink-0">
          <span class="text-[0.78rem] text-[var(--c-a0a0a0)] truncate">{{ selected.title }}</span>
          <div class="flex items-center gap-1.5 ml-auto shrink-0">
            <button class="bg-surface text-[var(--c-808080)] border border-raised rounded px-2 py-[0.2rem] text-[0.7rem] font-[inherit] cursor-pointer hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" @click="openInTab(selected)">Open in tab</button>
            <button class="bg-surface text-[var(--c-808080)] border border-raised rounded px-2 py-[0.2rem] text-[0.7rem] font-[inherit] cursor-pointer hover:text-[var(--c-d08080)]" @click="remove(selected)">Delete</button>
          </div>
        </div>
        <!-- Same-origin /api iframe: session cookie flows automatically. -->
        <iframe
          :src="api.reports.htmlPath(selected.id)"
          class="flex-1 w-full border-0 bg-white"
          sandbox="allow-popups allow-popups-to-escape-sandbox"
          :title="selected.title"
        />
      </template>
      <div v-else class="flex-1 flex items-center justify-center text-[var(--c-383838)] text-[0.8125rem]">
        {{ loading ? 'Loading…' : 'Select a report' }}
      </div>
    </div>
    </div>
  </div>
</template>
