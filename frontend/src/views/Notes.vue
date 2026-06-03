<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import * as api from '../api'
import { renderMarkdown } from '../lib/markdown'
import { useNotesStore } from '../stores/notes'

const notesStore = useNotesStore()

const items = ref<api.Note[]>([])
const loading = ref(false)
const error = ref('')

const searchQuery = ref('')
const expandedId = ref<string | null>(null)

const filtered = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return items.value
  return items.value.filter(
    n => n.title.toLowerCase().includes(q) || n.content.toLowerCase().includes(q),
  )
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.notes.list({ limit: 1000 })
    items.value = res.notes
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load notes'
  } finally {
    loading.value = false
  }
}

onMounted(load)
// Refresh when the chat AI touches the notes.
watch(() => notesStore.changeToken, load)

// ── Add / edit composer ──────────────────────────────────────────────────────
const adding = ref(false)
const editingId = ref<string | null>(null)
const draft = ref({ title: '', content: '' })

function startAdd() {
  editingId.value = null
  adding.value = true
  draft.value = { title: '', content: '' }
}

function startEdit(n: api.Note) {
  adding.value = false
  expandedId.value = null
  editingId.value = n.id
  draft.value = { title: n.title, content: n.content }
}

async function saveDraft() {
  const title = draft.value.title.trim()
  const content = draft.value.content.trim()
  if (!title || !content) return
  try {
    if (editingId.value) {
      const res = await api.notes.update(editingId.value, { title, content })
      const idx = items.value.findIndex(x => x.id === editingId.value)
      if (idx !== -1) items.value[idx] = res.note
      editingId.value = null
    } else {
      const res = await api.notes.create(title, content)
      items.value.unshift(res.note)
      adding.value = false
    }
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to save note'
  }
}

async function remove(n: api.Note) {
  try {
    await api.notes.remove(n.id)
    items.value = items.value.filter(x => x.id !== n.id)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete note'
  }
}

// ── Display helpers ──────────────────────────────────────────────────────────
function snippet(content: string): string {
  const flat = content.replace(/\s+/g, ' ').trim()
  return flat.length > 140 ? flat.slice(0, 140) + '…' : flat
}

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })
}

function toggleExpand(n: api.Note) {
  if (editingId.value === n.id) return
  expandedId.value = expandedId.value === n.id ? null : n.id
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[#1e1e1e] shrink-0 flex-wrap">
      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-[10rem] text-[#484848]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="flex-1 bg-none border-none text-[#c0c0c0] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[#404040]" placeholder="Search notes…" />
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
      <input v-model="draft.title" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[#3a6adf] placeholder:text-[#404040]" placeholder="Title" />
      <textarea v-model="draft.content" rows="5" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[#3a6adf] placeholder:text-[#404040]" placeholder="Write your note… (markdown supported)" />
      <div class="flex items-center gap-2">
        <button class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[#254880] disabled:opacity-50" :disabled="!draft.title.trim() || !draft.content.trim()" @click="saveDraft">Save</button>
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
        {{ items.length === 0 ? 'No notes yet. Add one, or ask the AI to remember something.' : 'No notes match your search.' }}
      </div>
      <div v-else>
        <div v-for="n in filtered" :key="n.id" class="group border-b border-[#161616] hover:bg-[#131313]">
          <!-- Edit mode -->
          <div v-if="editingId === n.id" class="flex flex-col gap-2 px-3.5 py-2.5">
            <input v-model="draft.title" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[#3a6adf]" />
            <textarea v-model="draft.content" rows="6" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[#3a6adf]" />
            <div class="flex items-center gap-2">
              <button class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[#254880]" @click="saveDraft">Save</button>
              <button class="bg-transparent text-[#585858] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="editingId = null">Cancel</button>
            </div>
          </div>

          <!-- View mode -->
          <template v-else>
            <div class="flex items-start gap-3 px-3.5 py-2.5 cursor-pointer" @click="toggleExpand(n)">
              <div class="flex-1 min-w-0">
                <p class="text-[0.8125rem] text-[#d0d0d0] font-medium break-words">{{ n.title }}</p>
                <p v-if="expandedId !== n.id" class="text-[0.75rem] text-[#707070] leading-[1.4] break-words mt-[0.1rem]">{{ snippet(n.content) }}</p>
                <div class="text-[0.68rem] text-[#505050] mt-[0.2rem]">{{ fmtDate(n.updated_at) }}</div>
              </div>
              <div class="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100">
                <button class="text-[#606060] hover:text-[#a0c0ff] p-1 cursor-pointer bg-none border-none" title="Edit" @click.stop="startEdit(n)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>
                </button>
                <button class="text-[#606060] hover:text-[#d08080] p-1 cursor-pointer bg-none border-none" title="Delete" @click.stop="remove(n)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                </button>
              </div>
            </div>
            <!-- Expanded: rendered markdown -->
            <div v-if="expandedId === n.id" class="md-body px-3.5 pb-3 text-[0.8125rem] leading-[1.5] text-[#c0c0c0]" v-html="renderMarkdown(n.content)" />
          </template>
        </div>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.25rem] border-t border-surface text-[#505050] text-[0.68rem] shrink-0">
      {{ filtered.length }} / {{ items.length }} notes
    </div>
  </div>
</template>

<style scoped>
.md-body :deep(code) {
  background: #181818;
  border: 1px solid #262626;
  border-radius: 4px;
  padding: 0.05rem 0.3rem;
  font-size: 0.85em;
}
.md-body :deep(pre.md-pre) {
  background: #0d0d0d;
  border: 1px solid #222;
  border-radius: 6px;
  padding: 0.5rem 0.65rem;
  overflow-x: auto;
  margin: 0.3rem 0;
}
.md-body :deep(pre.md-pre code) { background: none; border: none; padding: 0; }
.md-body :deep(.md-h) { font-weight: 600; color: #e8e8e8; margin: 0.35rem 0 0.15rem; }
.md-body :deep(.md-h1) { font-size: 1.12em; }
.md-body :deep(.md-h2) { font-size: 1.06em; }
.md-body :deep(a) { color: #7ab0ff; text-decoration: underline; }
.md-body :deep(ul.md-ul), .md-body :deep(ol.md-ol) { margin: 0.3rem 0; padding-left: 1.3rem; }
.md-body :deep(ul.md-ul) { list-style: disc; }
.md-body :deep(ol.md-ol) { list-style: decimal; }
</style>
