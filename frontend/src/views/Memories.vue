<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'

const logs = useLogsStore()

const CATEGORIES = ['preference', 'fact', 'feedback', 'project', 'other'] as const
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

// ── Display helpers ──────────────────────────────────────────────────────────
const CAT_COLOR: Record<string, string> = {
  preference: '#7ab0ff',
  fact: '#7adfbb',
  feedback: '#ffb07a',
  project: '#c07aff',
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
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[#1e1e1e] shrink-0 flex-wrap">
      <select v-model="categoryFilter" class="bg-surface text-[#c0c0c0] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
        <option value="All">All categories</option>
        <option v-for="c in CATEGORIES" :key="c" :value="c">{{ c }}</option>
      </select>

      <select v-model="sourceFilter" class="bg-surface text-[#c0c0c0] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
        <option value="all">All sources</option>
        <option value="auto">Auto</option>
        <option value="manual">Manual</option>
      </select>

      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-[10rem] text-[#484848]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="flex-1 bg-none border-none text-[#c0c0c0] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[#404040]" placeholder="Search memories…" />
        <button v-if="searchQuery" class="bg-none border-none text-[#484848] cursor-pointer text-[0.65rem] p-0 transition-colors duration-100 hover:text-muted" @click="searchQuery = ''">✕</button>
      </div>

      <div class="flex items-center gap-1.5 ml-auto">
        <button class="flex items-center gap-[0.35rem] bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[#254880]" @click="startAdd">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          Add
        </button>
        <button class="flex items-center justify-center bg-surface text-[#808080] border border-raised rounded px-2 py-1 cursor-pointer transition-colors duration-100 hover:bg-[#222] hover:text-[#c0c0c0] disabled:opacity-50" title="Refresh" :disabled="loading" @click="load">
          <svg :class="loading ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        </button>
      </div>
    </div>

    <!-- Add composer -->
    <div v-if="adding" class="flex flex-col gap-2 px-3 py-2.5 border-b border-[#1e1e1e] bg-[#111] shrink-0">
      <textarea v-model="draft.content" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[#3a6adf] placeholder:text-[#404040]" placeholder="Something to remember about you, your preferences, or your projects…" />
      <div class="flex items-center gap-2">
        <select v-model="draft.category" class="bg-surface text-[#c0c0c0] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
          <option v-for="c in CATEGORIES" :key="c" :value="c">{{ c }}</option>
        </select>
        <button class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[#254880] disabled:opacity-50" :disabled="!draft.content.trim()" @click="saveAdd">Save</button>
        <button class="bg-transparent text-[#585858] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="adding = false">Cancel</button>
      </div>
    </div>

    <div v-if="error" class="px-3 py-2 text-danger text-[0.775rem] border-b border-[#1e1e1e] shrink-0">{{ error }}</div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading && items.length === 0" class="flex items-center justify-center h-full text-[#484848] text-[0.8125rem]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[#505050] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="filtered.length === 0" class="flex items-center justify-center h-full text-[#383838] text-[0.8125rem]">
        {{ items.length === 0 ? 'No memories yet. They accumulate as you chat.' : 'No memories match your filters.' }}
      </div>
      <div v-else>
        <div v-for="m in filtered" :key="m.id" class="group flex items-start gap-3 px-3.5 py-2.5 border-b border-[#161616] hover:bg-[#131313]">
          <!-- Edit mode -->
          <div v-if="editingId === m.id" class="flex-1 flex flex-col gap-2 min-w-0">
            <textarea v-model="editDraft.content" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[#3a6adf]" />
            <div class="flex items-center gap-2">
              <select v-model="editDraft.category" class="bg-surface text-[#c0c0c0] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
                <option v-for="c in CATEGORIES" :key="c" :value="c">{{ c }}</option>
              </select>
              <button class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[#254880]" @click="saveEdit(m)">Save</button>
              <button class="bg-transparent text-[#585858] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="editingId = null">Cancel</button>
            </div>
          </div>

          <!-- View mode -->
          <template v-else>
            <span class="mt-[0.15rem] shrink-0 text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border" :style="{ color: catColor(m.category), borderColor: catColor(m.category) + '55' }">{{ m.category }}</span>
            <div class="flex-1 min-w-0">
              <p class="text-[0.8125rem] text-[#d0d0d0] leading-[1.45] break-words">{{ m.content }}</p>
              <div class="text-[0.68rem] text-[#505050] mt-[0.2rem] flex items-center gap-1.5">
                <span>{{ m.source === 'auto' ? '✨ learned' : '✎ manual' }}</span>
                <span>·</span>
                <span>{{ fmtDate(m.created_at) }}</span>
              </div>
            </div>
            <div class="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100">
              <button class="text-[#606060] hover:text-[#a0c0ff] p-1 cursor-pointer bg-none border-none" title="Edit" @click="startEdit(m)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>
              </button>
              <button class="text-[#606060] hover:text-[#d08080] p-1 cursor-pointer bg-none border-none" title="Delete" @click="remove(m)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          </template>
        </div>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.25rem] border-t border-surface text-[#505050] text-[0.68rem] shrink-0">
      {{ filtered.length }} / {{ items.length }} memories
    </div>
  </div>
</template>
