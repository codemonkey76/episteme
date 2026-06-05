<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import * as api from '../api'
import { useTasksStore } from '../stores/tasks'
import { renderMarkdown, renderInlineMarkdown } from '../lib/markdown'

const tasksStore = useTasksStore()

const PRIORITIES = ['low', 'normal', 'high'] as const
type Priority = (typeof PRIORITIES)[number]

const items = ref<api.Task[]>([])
const loading = ref(false)
const error = ref('')

const statusFilter = ref<'open' | 'done' | 'all'>('open')
const searchQuery = ref('')

const filtered = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  return items.value.filter(t => {
    if (statusFilter.value !== 'all' && t.status !== statusFilter.value) return false
    if (q && !t.title.toLowerCase().includes(q) && !(t.notes ?? '').toLowerCase().includes(q)) return false
    return true
  })
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.tasks.list({ status: 'all', limit: 1000 })
    items.value = res.tasks
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load tasks'
  } finally {
    loading.value = false
  }
}

onMounted(load)
// Refresh when the chat AI touches the task list.
watch(() => tasksStore.changeToken, load)

// ── Add / edit composer ──────────────────────────────────────────────────────
interface Draft {
  title: string
  notes: string
  due: string // datetime-local value, '' = none
  priority: Priority
}
const emptyDraft = (): Draft => ({ title: '', notes: '', due: '', priority: 'normal' })

const adding = ref(false)
const editingId = ref<string | null>(null)
const draft = ref<Draft>(emptyDraft())

function startAdd() {
  editingId.value = null
  adding.value = true
  draft.value = emptyDraft()
}

function startEdit(t: api.Task) {
  adding.value = false
  editingId.value = t.id
  draft.value = {
    title: t.title,
    notes: t.notes ?? '',
    due: t.due_at ? toLocalInput(t.due_at) : '',
    priority: t.priority,
  }
}

// RFC3339 UTC ↔ datetime-local (browser local time)
function toLocalInput(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}
function fromLocalInput(v: string): string {
  return new Date(v).toISOString()
}

async function saveDraft() {
  const title = draft.value.title.trim()
  if (!title) return
  const due_at = draft.value.due ? fromLocalInput(draft.value.due) : null
  const notes = draft.value.notes.trim() || null
  try {
    if (editingId.value) {
      const res = await api.tasks.update(editingId.value, {
        title, notes, due_at, priority: draft.value.priority,
      })
      const idx = items.value.findIndex(x => x.id === editingId.value)
      if (idx !== -1) items.value[idx] = res.task
      editingId.value = null
    } else {
      const res = await api.tasks.create({
        title,
        notes: notes ?? undefined,
        due_at: due_at ?? undefined,
        priority: draft.value.priority,
      })
      items.value.unshift(res.task)
      adding.value = false
    }
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to save task'
  }
}

async function toggleDone(t: api.Task) {
  const status = t.status === 'done' ? 'open' : 'done'
  try {
    const res = await api.tasks.update(t.id, { status })
    const idx = items.value.findIndex(x => x.id === t.id)
    if (idx !== -1) items.value[idx] = res.task
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to update task'
  }
}

async function remove(t: api.Task) {
  try {
    await api.tasks.remove(t.id)
    items.value = items.value.filter(x => x.id !== t.id)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete task'
  }
}

// ── Display helpers ──────────────────────────────────────────────────────────
const PRIO_COLOR: Record<string, string> = {
  high: '#ff8080',
  normal: '#7ab0ff',
  low: '#9a9a9a',
}

function fmtDue(iso: string): string {
  const d = new Date(iso)
  return d.toLocaleString([], { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })
}

function isOverdue(t: api.Task): boolean {
  return t.status === 'open' && !!t.due_at && new Date(t.due_at).getTime() < Date.now()
}

const openCount = computed(() => items.value.filter(t => t.status === 'open').length)
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex-wrap">
      <select v-model="statusFilter" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
        <option value="open">Open</option>
        <option value="done">Done</option>
        <option value="all">All</option>
      </select>

      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-[10rem] text-[var(--c-484848)]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="flex-1 bg-none border-none text-[var(--c-c0c0c0)] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[var(--c-404040)]" placeholder="Search tasks…" />
        <button v-if="searchQuery" class="bg-none border-none text-[var(--c-484848)] cursor-pointer text-[0.65rem] p-0 transition-colors duration-100 hover:text-muted" @click="searchQuery = ''">✕</button>
      </div>

      <div class="flex items-center gap-1.5 ml-auto">
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
      <input v-model="draft.title" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="What needs doing?" @keyup.enter="saveDraft" />
      <textarea v-model="draft.notes" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Notes (markdown supported)" />
      <div class="flex items-center gap-2 flex-wrap">
        <input v-model="draft.due" type="datetime-local" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer [color-scheme:dark]" />
        <select v-model="draft.priority" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
          <option v-for="p in PRIORITIES" :key="p" :value="p">{{ p }}</option>
        </select>
        <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="!draft.title.trim()" @click="saveDraft">Save</button>
        <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="adding = false">Cancel</button>
      </div>
    </div>

    <div v-if="error" class="px-3 py-2 text-danger text-[0.775rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading && items.length === 0" class="flex items-center justify-center h-full text-[var(--c-484848)] text-[0.8125rem]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="filtered.length === 0" class="flex items-center justify-center h-full text-[var(--c-383838)] text-[0.8125rem]">
        {{ items.length === 0 ? 'No tasks yet. Add one, or ask the AI.' : 'No tasks match your filters.' }}
      </div>
      <div v-else>
        <div v-for="t in filtered" :key="t.id" class="group flex items-start gap-3 px-3.5 py-2.5 border-b border-[var(--c-161616)] hover:bg-[var(--c-131313)]">
          <!-- Edit mode -->
          <div v-if="editingId === t.id" class="flex-1 flex flex-col gap-2 min-w-0">
            <input v-model="draft.title" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[var(--c-3a6adf)]" @keyup.enter="saveDraft" />
            <textarea v-model="draft.notes" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)]" placeholder="Notes (markdown supported)" />
            <div class="flex items-center gap-2 flex-wrap">
              <input v-model="draft.due" type="datetime-local" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer [color-scheme:dark]" />
              <select v-model="draft.priority" class="bg-surface text-[var(--c-c0c0c0)] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
                <option v-for="p in PRIORITIES" :key="p" :value="p">{{ p }}</option>
              </select>
              <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-254880)]" @click="saveDraft">Save</button>
              <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="editingId = null">Cancel</button>
            </div>
          </div>

          <!-- View mode -->
          <template v-else>
            <button
              class="mt-[0.1rem] shrink-0 w-[16px] h-[16px] rounded border cursor-pointer bg-transparent transition-colors duration-100 flex items-center justify-center"
              :class="t.status === 'done' ? 'border-[var(--c-4caf6e)] text-[var(--c-4caf6e)]' : 'border-[var(--c-404040)] text-transparent hover:border-[var(--c-4caf6e)]'"
              :title="t.status === 'done' ? 'Reopen' : 'Mark done'"
              @click="toggleDone(t)"
            >
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
            </button>
            <div class="flex-1 min-w-0">
              <p class="md-inline text-[0.8125rem] leading-[1.45] break-words" :class="t.status === 'done' ? 'text-[var(--c-585858)] line-through' : 'text-[var(--c-d0d0d0)]'" v-html="renderInlineMarkdown(t.title)" />
              <div v-if="t.notes" class="md-body text-[0.75rem] text-[var(--c-707070)] leading-[1.4] break-words mt-[0.1rem]" v-html="renderMarkdown(t.notes)" />
              <div class="text-[0.68rem] mt-[0.2rem] flex items-center gap-1.5">
                <span class="font-semibold uppercase tracking-[0.04em]" :style="{ color: PRIO_COLOR[t.priority] }">{{ t.priority }}</span>
                <template v-if="t.due_at">
                  <span class="text-[var(--c-505050)]">·</span>
                  <span :class="isOverdue(t) ? 'text-[var(--c-ff7070)]' : 'text-[var(--c-505050)]'">due {{ fmtDue(t.due_at) }}</span>
                </template>
              </div>
            </div>
            <div class="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100">
              <button class="text-[var(--c-606060)] hover:text-[var(--c-a0c0ff)] p-1 cursor-pointer bg-none border-none" title="Edit" @click="startEdit(t)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>
              </button>
              <button class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none" title="Delete" @click="remove(t)">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          </template>
        </div>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.25rem] border-t border-surface text-[var(--c-505050)] text-[0.68rem] shrink-0">
      {{ openCount }} open / {{ items.length }} tasks
    </div>
  </div>
</template>

<style scoped>
/* Markdown rendered into task titles/notes (v-html → needs :deep). */
.md-inline :deep(code), .md-body :deep(code) {
  background: #181818;
  border: 1px solid #262626;
  border-radius: 4px;
  padding: 0.05rem 0.3rem;
  font-size: 0.85em;
}
.md-inline :deep(a), .md-body :deep(a) { color: #7ab0ff; text-decoration: underline; }
.md-body :deep(pre.md-pre) {
  background: #0d0d0d;
  border: 1px solid #222;
  border-radius: 6px;
  padding: 0.5rem 0.65rem;
  overflow-x: auto;
  margin: 0.3rem 0;
}
.md-body :deep(pre.md-pre code) { background: none; border: none; padding: 0; }
.md-body :deep(.md-h) { font-weight: 600; color: #c8c8c8; margin: 0.25rem 0 0.1rem; }
.md-body :deep(.md-h1) { font-size: 1.1em; }
.md-body :deep(.md-h2) { font-size: 1.05em; }
.md-body :deep(ul.md-ul), .md-body :deep(ol.md-ol) { margin: 0.2rem 0; padding-left: 1.2rem; }
.md-body :deep(ul.md-ul) { list-style: disc; }
.md-body :deep(ol.md-ol) { list-style: decimal; }
</style>
