<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'

const logs = useLogsStore()

const CATEGORIES = ['preference', 'fact', 'feedback', 'project', 'style', 'lesson', 'other'] as const
type Category = (typeof CATEGORIES)[number]

const items = ref<api.Memory[]>([])
const loading = ref(false)
const error = ref('')

const categoryFilter = ref('All')
const sourceFilter = ref<'all' | 'auto' | 'manual'>('all')
const searchQuery = ref('')

const filtered = computed(() => {
  const src = sourceFilter.value
  const q = searchQuery.value.trim().toLowerCase()
  return items.value.filter(m => {
    if (categoryFilter.value !== 'All' && m.category !== categoryFilter.value) return false
    if (src !== 'all' && m.source !== src) return false
    if (q && !m.content.toLowerCase().includes(q)) return false
    return true
  })
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.memories.list({ limit: 1000 })
    items.value = res.memories
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load memories'
  } finally {
    loading.value = false
  }
}

onMounted(load)

// ── Add ─────────────────────────────────────────────────────────────────────
const adding = ref(false)
const draft = ref<{ content: string; category: Category }>({ content: '', category: 'fact' })

function startAdd() {
  adding.value = true
  draft.value = { content: '', category: 'fact' }
}

async function saveAdd() {
  const content = draft.value.content.trim()
  if (!content) return
  try {
    const res = await api.memories.create(content, draft.value.category)
    items.value.unshift(res.memory)
    logs.info('Memory', `Added [${draft.value.category}]: ${content}`)
    adding.value = false
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to add memory'
  }
}

// ── Edit ────────────────────────────────────────────────────────────────────
const editingId = ref<string | null>(null)
const editDraft = ref<{ content: string; category: Category }>({ content: '', category: 'fact' })

function startEdit(m: api.Memory) {
  editingId.value = m.id
  editDraft.value = { content: m.content, category: (m.category as Category) }
}

async function saveEdit(m: api.Memory) {
  const content = editDraft.value.content.trim()
  if (!content) return
  try {
    await api.memories.update(m.id, content, editDraft.value.category)
    const idx = items.value.findIndex(x => x.id === m.id)
    if (idx !== -1) items.value[idx] = { ...items.value[idx], content, category: editDraft.value.category }
    editingId.value = null
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to update memory'
  }
}

async function remove(m: api.Memory) {
  try {
    await api.memories.remove(m.id)
    items.value = items.value.filter(x => x.id !== m.id)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete memory'
  }
}

// ── Dream (consolidation) ─────────────────────────────────────────────────────
const DREAM_PROVIDER_KEY = 'episteme.memories.dreamProvider'
const providers = ref<api.ProviderConfig[]>([])
// Remember the chosen dream model across refreshes.
const dreamProvider = ref(localStorage.getItem(DREAM_PROVIDER_KEY) ?? '')
const dreaming = ref(false)
const dreamMsg = ref('')

watch(dreamProvider, (v) => localStorage.setItem(DREAM_PROVIDER_KEY, v))

onMounted(async () => {
  try {
    providers.value = (await api.settings.listProviders()).providers
    // Drop a stale saved choice (provider since removed) back to default.
    if (dreamProvider.value && !providers.value.some(p => p.name === dreamProvider.value)) {
      dreamProvider.value = ''
    }
  } catch { /* leave empty */ }
})

async function dream() {
  if (dreaming.value) return
  dreaming.value = true
  dreamMsg.value = ''
  error.value = ''
  try {
    const { summary, provider } = await api.memories.consolidate(dreamProvider.value || undefined)
    dreamMsg.value = `Dreamt with ${provider} — merged ${summary.merged}, dropped ${summary.dropped}, ${summary.lessons} new lesson${summary.lessons === 1 ? '' : 's'}.`
    logs.info('Memory', dreamMsg.value)
    await load()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Consolidation failed'
  } finally {
    dreaming.value = false
  }
}

// ── Archive (soft-deleted) view ───────────────────────────────────────────────
const view = ref<'active' | 'archive'>('active')
const archived = ref<api.Memory[]>([])

async function toggleArchive() {
  if (view.value === 'archive') { view.value = 'active'; return }
  view.value = 'archive'
  try { archived.value = (await api.memories.listDeleted()).memories } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load archive'
  }
}

async function restore(m: api.Memory) {
  try {
    await api.memories.restore(m.id)
    archived.value = archived.value.filter(x => x.id !== m.id)
    await load()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to restore memory'
  }
}

// ── Display helpers ──────────────────────────────────────────────────────────
const CAT_COLOR: Record<string, string> = {
  preference: '#7ab0ff',
  fact: '#7adfbb',
  feedback: '#ffb07a',
  project: '#c07aff',
  style: '#6ecfcf',
  lesson: '#e0c060',
  other: '#9a9a9a',
}
function catColor(c: string): string {
  return CAT_COLOR[c] ?? '#9a9a9a'
}

function fmtDate(iso: string): string {
  const d = new Date(iso)
  return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex-wrap">
      <select v-model="categoryFilter" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
        <option value="All">All categories</option>
        <option v-for="c in CATEGORIES" :key="c" :value="c">{{ c }}</option>
      </select>

      <select v-model="sourceFilter" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
        <option value="all">All sources</option>
        <option value="auto">Auto</option>
        <option value="manual">Manual</option>
      </select>

      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-[10rem] text-[var(--c-484848)]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="flex-1 bg-none border-none text-[var(--c-c0c0c0)] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[var(--c-404040)]" placeholder="Search memories…" />
        <button v-if="searchQuery" class="bg-none border-none text-[var(--c-484848)] cursor-pointer text-[0.65rem] p-0 transition-colors duration-100 hover:text-muted" @click="searchQuery = ''">✕</button>
      </div>

      <div class="flex items-center gap-1.5 ml-auto">
        <select
          v-if="providers.length"
          v-model="dreamProvider"
          title="Model used to consolidate memories — point this at your smartest model"
          class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer max-w-[9rem]"
        >
          <option value="">dream model: default</option>
          <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
        </select>
        <button
          class="flex items-center gap-[0.35rem] bg-[var(--c-2a2150)] text-[var(--c-b69cff)] border border-[var(--c-3a2f70)] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-352a66)] disabled:opacity-50"
          title="Consolidate memories now: merge redundant, resolve conflicts, extract lessons"
          :disabled="dreaming"
          @click="dream"
        >
          <span v-if="dreaming" class="inline-block w-[11px] h-[11px] border-2 border-[var(--c-3a2f70)] border-t-[var(--c-b69cff)] rounded-full animate-[spin_0.7s_linear_infinite]" />
          <span v-else>💤</span>
          {{ dreaming ? 'Dreaming…' : 'Dream' }}
        </button>
        <button
          class="bg-surface text-[var(--c-808080)] border border-raised rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]"
          :class="view === 'archive' ? 'bg-[var(--c-222222)] text-[var(--c-c0c0c0)]' : ''"
          title="View archived (consolidated/dropped) memories and restore them"
          @click="toggleArchive"
        >Archive</button>
        <button class="flex items-center gap-[0.35rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254880)]" @click="startAdd">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          Add
        </button>
        <button class="flex items-center justify-center bg-surface text-[var(--c-808080)] border border-raised rounded px-2 py-1 cursor-pointer transition-colors duration-100 hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)] disabled:opacity-50" title="Refresh" :disabled="loading" @click="load">
          <svg :class="loading ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        </button>
      </div>
    </div>

    <!-- Add composer -->
    <div v-if="adding" class="flex flex-col gap-2 px-3 py-2.5 border-b border-[var(--c-1e1e1e)] bg-[var(--c-111111)] shrink-0">
      <textarea v-model="draft.content" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Something to remember about you, your preferences, or your projects…" />
      <div class="flex items-center gap-2">
        <select v-model="draft.category" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
          <option v-for="c in CATEGORIES" :key="c" :value="c">{{ c }}</option>
        </select>
        <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="!draft.content.trim()" @click="saveAdd">Save</button>
        <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="adding = false">Cancel</button>
      </div>
    </div>

    <div v-if="error" class="px-3 py-2 text-danger text-[0.775rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>
    <div v-if="dreamMsg" class="flex items-center gap-2 px-3 py-2 text-[var(--c-b69cff)] text-[0.775rem] border-b border-[var(--c-1e1e1e)] bg-[var(--c-17142c)] shrink-0">
      <span>💤</span><span class="flex-1">{{ dreamMsg }}</span>
      <button class="text-[var(--c-585858)] hover:text-muted bg-none border-none cursor-pointer text-[0.7rem]" @click="dreamMsg = ''">✕</button>
    </div>

    <!-- Archive (soft-deleted) view -->
    <div v-if="view === 'archive'" class="flex-1 overflow-y-auto">
      <div v-if="archived.length === 0" class="flex items-center justify-center h-full text-[var(--c-383838)] text-[0.8125rem]">
        Nothing archived. Consolidated or dropped memories appear here and can be restored.
      </div>
      <div v-else>
        <div v-for="m in archived" :key="m.id" class="group flex items-start gap-3 px-3.5 py-2.5 border-b border-[var(--c-161616)] hover:bg-[var(--c-131313)]">
          <span class="mt-[0.15rem] shrink-0 text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border" :style="{ color: catColor(m.category), borderColor: catColor(m.category) + '55' }">{{ m.category }}</span>
          <div class="flex-1 min-w-0">
            <p class="text-[0.8125rem] text-[var(--c-909090)] leading-[1.45] break-words line-through decoration-[var(--c-3a3a3a)]">{{ m.content }}</p>
            <div class="text-[0.68rem] text-[var(--c-505050)] mt-[0.2rem]">archived · {{ fmtDate(m.created_at) }}</div>
          </div>
          <button class="shrink-0 bg-surface text-[var(--c-7adfbb)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-222222)] opacity-0 group-hover:opacity-100 transition-opacity duration-100" @click="restore(m)">Restore</button>
        </div>
      </div>
    </div>

    <!-- List -->
    <div v-else class="flex-1 overflow-y-auto">
      <div v-if="loading && items.length === 0" class="flex items-center justify-center h-full text-[var(--c-484848)] text-[0.8125rem]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="filtered.length === 0" class="flex items-center justify-center h-full text-[var(--c-383838)] text-[0.8125rem]">
        {{ items.length === 0 ? 'No memories yet. They accumulate as you chat.' : 'No memories match your filters.' }}
      </div>
      <div v-else>
        <div v-for="m in filtered" :key="m.id" class="group flex items-start gap-3 px-3.5 py-2.5 border-b border-[var(--c-161616)] hover:bg-[var(--c-131313)]">
          <!-- Edit mode -->
          <div v-if="editingId === m.id" class="flex-1 flex flex-col gap-2 min-w-0">
            <textarea v-model="editDraft.content" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)]" />
            <div class="flex items-center gap-2">
              <select v-model="editDraft.category" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
                <option v-for="c in CATEGORIES" :key="c" :value="c">{{ c }}</option>
              </select>
              <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-254880)]" @click="saveEdit(m)">Save</button>
              <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="editingId = null">Cancel</button>
            </div>
          </div>

          <!-- View mode -->
          <template v-else>
            <span class="mt-[0.15rem] shrink-0 text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border" :style="{ color: catColor(m.category), borderColor: catColor(m.category) + '55' }">{{ m.category }}</span>
            <div class="flex-1 min-w-0">
              <p class="text-[0.8125rem] text-[var(--c-d0d0d0)] leading-[1.45] break-words">{{ m.content }}</p>
              <div class="text-[0.68rem] text-[var(--c-505050)] mt-[0.2rem] flex items-center gap-1.5">
                <span>{{ m.source === 'auto' ? '✨ learned' : '✎ manual' }}</span>
                <span>·</span>
                <span>{{ fmtDate(m.created_at) }}</span>
              </div>
            </div>
            <div class="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100">
              <button class="text-[var(--c-606060)] hover:text-[var(--c-a0c0ff)] p-1 cursor-pointer bg-none border-none" title="Edit" @click="startEdit(m)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>
              </button>
              <button class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none" title="Delete" @click="remove(m)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          </template>
        </div>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.25rem] border-t border-surface text-[var(--c-505050)] text-[0.68rem] shrink-0">
      <span v-if="view === 'archive'">{{ archived.length }} archived</span>
      <span v-else>{{ filtered.length }} / {{ items.length }} memories</span>
    </div>
  </div>
</template>
