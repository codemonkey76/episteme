<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'

const logs = useLogsStore()

const items = ref<api.Document[]>([])
const loading = ref(false)
const error = ref('')
const uploading = ref(0)
const dragOver = ref(false)

const anyIndexing = computed(() => items.value.some(d => d.status === 'indexing'))

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.documents.list()
    items.value = res.documents
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load documents'
  } finally {
    loading.value = false
  }
}

// Poll while anything is still indexing so status flips live.
let poll: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  load()
  poll = setInterval(() => {
    if (anyIndexing.value) load()
  }, 3000)
})
onBeforeUnmount(() => {
  if (poll) clearInterval(poll)
})

// ── Upload ───────────────────────────────────────────────────────────────────
const fileInput = ref<HTMLInputElement | null>(null)

function pickFiles() {
  fileInput.value?.click()
}

function onPicked(e: Event) {
  const input = e.target as HTMLInputElement
  if (input.files) uploadFiles(Array.from(input.files))
  input.value = ''
}

function onDrop(e: DragEvent) {
  dragOver.value = false
  if (e.dataTransfer?.files) uploadFiles(Array.from(e.dataTransfer.files))
}

async function uploadFiles(files: File[]) {
  for (const file of files) {
    uploading.value++
    try {
      const b64 = await readAsBase64(file)
      const res = await api.documents.upload(file.name, file.type || 'application/octet-stream', b64)
      items.value.unshift(res.document)
      logs.info('Documents', `Uploaded: ${file.name}`)
    } catch (e: unknown) {
      error.value = e instanceof Error ? e.message : `Failed to upload ${file.name}`
    } finally {
      uploading.value--
    }
  }
}

function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      // data:<mime>;base64,<payload> → payload only.
      const result = reader.result as string
      resolve(result.slice(result.indexOf(',') + 1))
    }
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(file)
  })
}

async function remove(d: api.Document) {
  try {
    await api.documents.remove(d.id)
    items.value = items.value.filter(x => x.id !== d.id)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete document'
  }
}

// ── Display helpers ──────────────────────────────────────────────────────────
function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function fmtDate(iso: string): string {
  const d = new Date(iso)
  return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })
}
</script>

<template>
  <div
    class="flex flex-col h-full bg-bg overflow-hidden relative"
    @dragover.prevent="dragOver = true"
    @dragleave="dragOver = false"
    @drop.prevent="onDrop"
  >
    <!-- Drop overlay -->
    <div v-if="dragOver" class="absolute inset-0 z-10 flex items-center justify-center bg-[var(--c-111111)]/80 border-2 border-dashed border-[var(--c-3a6adf)] rounded text-[var(--c-7ab0ff)] text-sm pointer-events-none">
      Drop files to upload
    </div>

    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0">
      <span class="text-[0.72rem] text-[var(--c-585858)]">Text, Markdown, HTML, CSV, JSON, PDF — searchable by the assistant</span>
      <div class="flex items-center gap-1.5 ml-auto">
        <button class="flex items-center gap-[0.35rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254880)] disabled:opacity-50" :disabled="uploading > 0" @click="pickFiles">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
          {{ uploading > 0 ? 'Uploading…' : 'Upload' }}
        </button>
        <button class="flex items-center justify-center bg-surface text-[var(--c-808080)] border border-raised rounded px-2 py-1 cursor-pointer transition-colors duration-100 hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)] disabled:opacity-50" title="Refresh" :disabled="loading" @click="load">
          <svg :class="loading ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        </button>
      </div>
      <input ref="fileInput" type="file" multiple class="hidden" accept=".txt,.md,.html,.htm,.csv,.json,.yaml,.yml,.toml,.log,.pdf,text/*,application/pdf,application/json" @change="onPicked" />
    </div>

    <div v-if="error" class="px-3 py-2 text-danger text-[0.775rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading && items.length === 0" class="flex items-center justify-center h-full text-[var(--c-484848)] text-[0.8125rem]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="items.length === 0" class="flex items-center justify-center h-full text-[var(--c-383838)] text-[0.8125rem]">
        No documents yet. Upload files (or drop them here) and ask the assistant about them.
      </div>
      <div v-else>
        <div v-for="d in items" :key="d.id" class="group flex items-center gap-3 px-3.5 py-2.5 border-b border-[var(--c-161616)] hover:bg-[var(--c-131313)]">
          <svg class="shrink-0 text-[var(--c-606060)]" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/>
          </svg>
          <div class="flex-1 min-w-0">
            <p class="text-[0.8125rem] text-[var(--c-d0d0d0)] truncate">{{ d.filename }}</p>
            <div class="text-[0.68rem] text-[var(--c-505050)] mt-[0.1rem] flex items-center gap-1.5">
              <span>{{ fmtSize(d.size) }}</span>
              <span>·</span>
              <span>{{ fmtDate(d.created_at) }}</span>
              <template v-if="d.status === 'ready'">
                <span>·</span>
                <span>{{ d.chunk_count }} chunks</span>
              </template>
            </div>
          </div>
          <span v-if="d.status === 'indexing'" class="shrink-0 flex items-center gap-1.5 text-[0.68rem] text-[var(--c-7ab0ff)]">
            <span class="inline-block w-[10px] h-[10px] border border-[var(--c-2a4a8a)] border-t-[var(--c-7ab0ff)] rounded-full animate-[spin_0.7s_linear_infinite]" />
            indexing
          </span>
          <span v-else-if="d.status === 'error'" class="shrink-0 text-[0.68rem] text-danger" :title="d.error_message ?? ''">failed</span>
          <span v-else class="shrink-0 text-[0.68rem] text-[var(--c-7adfbb)]">ready</span>
          <button class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100" title="Delete" @click="remove(d)">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.25rem] border-t border-surface text-[var(--c-505050)] text-[0.68rem] shrink-0">
      {{ items.length }} document{{ items.length === 1 ? '' : 's' }}
    </div>
  </div>
</template>
