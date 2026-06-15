<script setup lang="ts">
import { ref, computed, onMounted, watch, nextTick } from 'vue'
import * as api from '../api'
import { renderMarkdown } from '../lib/markdown'

// ── Document list ─────────────────────────────────────────────────────────────
const items = ref<api.WriterDoc[]>([])
const loading = ref(false)
const error = ref('')
const searchQuery = ref('')

const filtered = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return items.value
  return items.value.filter(
    d => d.title.toLowerCase().includes(q) || d.content.toLowerCase().includes(q),
  )
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.writer.list({ limit: 1000 })
    items.value = res.docs
    // Open the most recent document on first load.
    if (!selectedId.value && items.value.length) selectDoc(items.value[0])
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load documents'
  } finally {
    loading.value = false
  }
}

// ── AI provider ───────────────────────────────────────────────────────────────
const aiProvider = ref('')
const providers = ref<api.ProviderConfig[]>([])

onMounted(async () => {
  await load()
  try {
    const { providers: list } = await api.settings.listProviders()
    providers.value = list
    if (list.length) aiProvider.value = list[0].name
  } catch { /* no providers; Improve will surface the error */ }
})

// ── Selected document + editor state ──────────────────────────────────────────
const selectedId = ref<string | null>(null)
const title = ref('')
const content = ref('')
const showPreview = ref(false)
const textareaEl = ref<HTMLTextAreaElement | null>(null)

const selected = computed(() => items.value.find(d => d.id === selectedId.value) ?? null)

function selectDoc(d: api.WriterDoc) {
  if (d.id === selectedId.value) return
  // Leaving a doc mid-improve discards the in-flight rewrite.
  if (improveBackup.value != null) cancelImprove()
  flushSave()
  selectedId.value = d.id
  title.value = d.title
  content.value = d.content
  showPreview.value = false
  resetSelection()
}

async function newDoc() {
  try {
    const { doc } = await api.writer.create('Untitled', '')
    items.value.unshift(doc)
    selectedId.value = null // force selectDoc to run
    selectDoc(doc)
    await nextTick()
    titleEl.value?.focus()
    titleEl.value?.select()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to create document'
  }
}

async function removeDoc(d: api.WriterDoc) {
  if (!confirm(`Delete "${d.title || 'Untitled'}"? This can't be undone.`)) return
  try {
    await api.writer.remove(d.id)
    items.value = items.value.filter(x => x.id !== d.id)
    if (selectedId.value === d.id) {
      selectedId.value = null
      if (items.value.length) selectDoc(items.value[0])
      else { title.value = ''; content.value = '' }
    }
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete document'
  }
}

const titleEl = ref<HTMLInputElement | null>(null)

// ── Autosave ──────────────────────────────────────────────────────────────────
// Debounced persistence of title/content edits. Suspended while an AI rewrite is
// streaming so half-written improvements aren't saved; an explicit save runs once
// the user accepts or reverts.
const saveState = ref<'idle' | 'saving' | 'saved'>('idle')
let saveTimer: ReturnType<typeof setTimeout> | null = null

function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = setTimeout(flushSave, 700)
}

async function flushSave() {
  if (saveTimer) { clearTimeout(saveTimer); saveTimer = null }
  const id = selectedId.value
  if (!id) return
  const d = items.value.find(x => x.id === id)
  if (!d || (d.title === title.value && d.content === content.value)) return
  saveState.value = 'saving'
  try {
    const res = await api.writer.update(id, { title: title.value, content: content.value })
    const idx = items.value.findIndex(x => x.id === id)
    if (idx !== -1) items.value[idx] = res.doc
    saveState.value = 'saved'
    setTimeout(() => { if (saveState.value === 'saved') saveState.value = 'idle' }, 1500)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to save'
    saveState.value = 'idle'
  }
}

watch([title, content], () => {
  if (!selectedId.value || improving.value) return
  scheduleSave()
})

// ── Text selection tracking (for "improve this section") ──────────────────────
const selStart = ref(0)
const selEnd = ref(0)
const hasSelection = computed(() => selEnd.value > selStart.value)

function updateSelection() {
  const el = textareaEl.value
  if (!el) return
  selStart.value = el.selectionStart
  selEnd.value = el.selectionEnd
}
function resetSelection() {
  selStart.value = 0
  selEnd.value = 0
}

// ── AI Improve ────────────────────────────────────────────────────────────────
// Rewrite either the whole document or the selected passage. The pre-improve text
// is stashed so the user can revert; a non-null backup means a preview is active.
const improving = ref(false)
const improveWarming = ref(false)
const improveBackup = ref<string | null>(null)
const improveScope = ref<'document' | 'selection'>('document')

async function improve() {
  if (improving.value || improveBackup.value != null) return
  if (!aiProvider.value) {
    error.value = 'No AI provider configured — add one in Settings.'
    return
  }
  // A selection (in edit mode) improves just that span; otherwise the whole doc.
  const useSel = hasSelection.value && !showPreview.value
  const start = useSel ? selStart.value : 0
  const end = useSel ? selEnd.value : content.value.length
  const target = content.value.slice(start, end).trim()
  if (!target) {
    error.value = useSel ? 'The selection is empty.' : 'Nothing to improve — write something first.'
    return
  }

  improveScope.value = useSel ? 'selection' : 'document'
  improveBackup.value = content.value
  improving.value = true
  improveWarming.value = false
  error.value = ''
  showPreview.value = false

  const prefix = content.value.slice(0, start)
  const suffix = content.value.slice(end)
  let firstToken = false
  const warmTimer = setTimeout(() => { if (!firstToken) improveWarming.value = true }, 4000)
  let improved = ''
  content.value = prefix + suffix
  try {
    await api.streamWriterImprove(
      {
        provider: aiProvider.value,
        text: target,
        // Give the model the full document as context only when improving a span.
        context: useSel ? improveBackup.value : '',
      },
      (text) => {
        if (!firstToken) { firstToken = true; improveWarming.value = false }
        improved += text
        content.value = prefix + improved + suffix
      },
    )
  } catch (e: unknown) {
    error.value = `Improve failed: ${e instanceof Error ? e.message : e}`
    if (improveBackup.value != null) content.value = improveBackup.value
    improveBackup.value = null
  } finally {
    clearTimeout(warmTimer)
    improveWarming.value = false
    improving.value = false
  }
}

// Keep the rewrite; persist and drop the undo snapshot.
function acceptImprove() {
  improveBackup.value = null
  resetSelection()
  flushSave()
}

// Discard the rewrite and restore what the user had.
function cancelImprove() {
  if (improveBackup.value != null) content.value = improveBackup.value
  improveBackup.value = null
  resetSelection()
}

// ── Display helpers ───────────────────────────────────────────────────────────
function snippet(c: string): string {
  const flat = c.replace(/\s+/g, ' ').trim()
  return flat.length > 90 ? flat.slice(0, 90) + '…' : flat || 'Empty'
}
function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric' })
}
const wordCount = computed(() => content.value.trim() ? content.value.trim().split(/\s+/).length : 0)
</script>

<template>
  <div class="flex h-full bg-bg overflow-hidden">
    <!-- ── Sidebar: document list ── -->
    <div class="w-[15rem] shrink-0 border-r border-[var(--c-1e1e1e)] flex flex-col min-h-0">
      <div class="flex items-center gap-2 px-2.5 py-2 border-b border-[var(--c-1e1e1e)] shrink-0">
        <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-0 text-[var(--c-484848)]">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
          <input v-model="searchQuery" class="flex-1 bg-none border-none text-[var(--c-c0c0c0)] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[var(--c-404040)]" placeholder="Search…" />
        </div>
        <button class="flex items-center justify-center bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-1.5 py-1 cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254880)]" title="New document" @click="newDoc">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
        </button>
      </div>

      <div class="flex-1 overflow-y-auto min-h-0">
        <div v-if="loading && items.length === 0" class="flex items-center justify-center h-20 text-[var(--c-484848)]">
          <span class="inline-block w-[16px] h-[16px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
        </div>
        <div v-else-if="filtered.length === 0" class="px-3 py-6 text-center text-[var(--c-383838)] text-[0.75rem]">
          {{ items.length === 0 ? 'No documents yet.' : 'No matches.' }}
        </div>
        <button
          v-for="d in filtered"
          :key="d.id"
          :class="['group w-full text-left px-3 py-2 border-b border-[var(--c-161616)] cursor-pointer bg-none border-x-0 border-t-0 transition-colors duration-100', d.id === selectedId ? 'bg-[var(--c-15233f)]' : 'hover:bg-[var(--c-131313)]']"
          @click="selectDoc(d)"
        >
          <div class="flex items-center gap-2">
            <span class="flex-1 min-w-0 truncate text-[0.8125rem] font-medium" :class="d.id === selectedId ? 'text-[var(--c-cfe0ff)]' : 'text-[var(--c-c0c0c0)]'">{{ d.title || 'Untitled' }}</span>
            <span class="text-[0.65rem] text-[var(--c-505050)] shrink-0">{{ fmtDate(d.updated_at) }}</span>
            <span
              class="text-[var(--c-505050)] hover:text-[var(--c-d08080)] shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100"
              title="Delete"
              @click.stop="removeDoc(d)"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
            </span>
          </div>
          <p class="text-[0.7rem] text-[var(--c-606060)] truncate mt-[0.1rem]">{{ snippet(d.content) }}</p>
        </button>
      </div>
    </div>

    <!-- ── Editor pane ── -->
    <div class="flex-1 flex flex-col min-w-0 min-h-0">
      <template v-if="selected">
        <!-- Toolbar -->
        <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex-wrap">
          <!-- Improve / accept-revert -->
          <template v-if="improveBackup != null && !improving">
            <span class="text-[0.7rem] text-[var(--c-7ab0ff)]">Improved {{ improveScope }} — keep it?</span>
            <button class="inline-flex items-center gap-[0.35rem] py-1 px-2.5 bg-[var(--c-15401e)] text-[var(--c-7ad08a)] border border-[var(--c-1e5a2a)] rounded text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-1a4d24)]" @click="acceptImprove">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              Keep
            </button>
            <button class="bg-transparent text-[var(--c-585858)] border-none py-1 px-2 cursor-pointer text-xs font-[inherit] hover:text-muted" @click="cancelImprove">Revert</button>
          </template>
          <button
            v-else
            class="inline-flex items-center gap-[0.35rem] py-1 px-2.5 bg-surface text-[var(--c-7ab0ff)] border border-raised rounded text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-222222)] disabled:opacity-50"
            :disabled="improving"
            :title="aiProvider ? `Improve with ${aiProvider}` : 'Improve your writing'"
            @click="improve"
          >
            <span v-if="improving" class="inline-block w-[11px] h-[11px] border-2 border-[var(--c-2a4a8a)] border-t-[var(--c-7ab0ff)] rounded-full animate-[spin_0.7s_linear_infinite]" />
            <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.9 4.6L18.5 9.5 13.9 11.4 12 16l-1.9-4.6L5.5 9.5l4.6-1.9z"/><path d="M19 14l.8 2 2 .8-2 .8-.8 2-.8-2-2-.8 2-.8z"/></svg>
            {{ improving ? (improveWarming ? 'Loading model…' : 'Improving…') : (hasSelection && !showPreview ? 'Improve selection' : 'Improve') }}
          </button>

          <select v-if="providers.length > 1" v-model="aiProvider" class="bg-surface text-[var(--c-a0a0a0)] border border-raised rounded px-1.5 py-1 text-[0.7rem] font-[inherit] cursor-pointer outline-none" title="AI model">
            <option v-for="p in providers" :key="p.name" :value="p.name">{{ p.name }}</option>
          </select>

          <div class="ml-auto flex items-center gap-2">
            <span class="text-[0.68rem] text-[var(--c-505050)]">
              <template v-if="saveState === 'saving'">Saving…</template>
              <template v-else-if="saveState === 'saved'">Saved</template>
              <template v-else>{{ wordCount }} words</template>
            </span>
            <div class="flex items-center rounded border border-raised overflow-hidden">
              <button :class="['px-2 py-1 text-[0.7rem] font-[inherit] cursor-pointer border-none', !showPreview ? 'bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)]' : 'bg-surface text-[var(--c-707070)] hover:text-[var(--c-c0c0c0)]']" @click="showPreview = false">Edit</button>
              <button :class="['px-2 py-1 text-[0.7rem] font-[inherit] cursor-pointer border-none', showPreview ? 'bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)]' : 'bg-surface text-[var(--c-707070)] hover:text-[var(--c-c0c0c0)]']" @click="showPreview = true">Preview</button>
            </div>
          </div>
        </div>

        <div v-if="error" class="px-3 py-1.5 text-danger text-[0.75rem] border-b border-[var(--c-1e1e1e)] shrink-0 flex items-center gap-2">
          <span class="flex-1">{{ error }}</span>
          <button class="text-[var(--c-585858)] hover:text-muted bg-none border-none cursor-pointer" @click="error = ''">✕</button>
        </div>

        <!-- Title -->
        <input
          v-model="title"
          class="bg-transparent text-[var(--c-e8e8e8)] border-none border-b border-[var(--c-1e1e1e)] px-4 py-2.5 text-[1.05rem] font-semibold font-[inherit] outline-none shrink-0 placeholder:text-[var(--c-404040)]"
          placeholder="Untitled"
        />

        <!-- Body: editor or preview -->
        <textarea
          v-show="!showPreview"
          ref="textareaEl"
          v-model="content"
          class="flex-1 min-h-0 bg-bg text-[var(--c-d0d0d0)] border-none px-4 py-3 text-[0.85rem] leading-[1.6] font-[inherit] outline-none resize-none placeholder:text-[var(--c-404040)]"
          placeholder="Start writing… markdown supported. Select a passage and hit Improve to polish just that part."
          spellcheck="true"
          @select="updateSelection"
          @mouseup="updateSelection"
          @keyup="updateSelection"
          @click="updateSelection"
        />
        <div v-show="showPreview" class="flex-1 min-h-0 overflow-y-auto md-body px-4 py-3 text-[0.85rem] leading-[1.6] text-[var(--c-d0d0d0)]" v-html="renderMarkdown(content)" />
      </template>

      <!-- Empty state -->
      <div v-else class="flex-1 flex flex-col items-center justify-center gap-3 text-[var(--c-484848)]">
        <svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/></svg>
        <p class="text-[0.85rem]">No document selected.</p>
        <button class="flex items-center gap-[0.35rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1.5 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-254880)]" @click="newDoc">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          New document
        </button>
      </div>
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
  margin: 0.4rem 0;
}
.md-body :deep(pre.md-pre code) { background: none; border: none; padding: 0; }
.md-body :deep(.md-h) { font-weight: 600; color: #e8e8e8; margin: 0.6rem 0 0.25rem; }
.md-body :deep(.md-h1) { font-size: 1.4em; }
.md-body :deep(.md-h2) { font-size: 1.2em; }
.md-body :deep(.md-h3) { font-size: 1.08em; }
.md-body :deep(p) { margin: 0.4rem 0; }
.md-body :deep(a) { color: #7ab0ff; text-decoration: underline; }
.md-body :deep(ul.md-ul), .md-body :deep(ol.md-ol) { margin: 0.4rem 0; padding-left: 1.4rem; }
.md-body :deep(ul.md-ul) { list-style: disc; }
.md-body :deep(ol.md-ol) { list-style: decimal; }
</style>
