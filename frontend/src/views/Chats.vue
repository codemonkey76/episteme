<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { useSessionsStore } from '../stores/sessions'
import { useWindowsStore } from '../stores/windows'
import * as api from '../api'

const store = useSessionsStore()
const winStore = useWindowsStore()

onMounted(() => store.fetchSessions())

function open(id: string) {
  winStore.openChat({ sessionId: id })
}

async function remove(id: string) {
  await api.sessions.delete(id)
  store.sessions.splice(store.sessions.findIndex((s) => s.id === id), 1)
}

// ── Full-text search across conversations ────────────────────────────────────
const query = ref('')
const hits = ref<api.SearchHit[]>([])
const searching = ref(false)
let debounce: ReturnType<typeof setTimeout> | null = null

watch(query, (q) => {
  if (debounce) clearTimeout(debounce)
  if (!q.trim()) {
    hits.value = []
    searching.value = false
    return
  }
  searching.value = true
  debounce = setTimeout(async () => {
    try {
      const res = await api.searchSessions(q.trim())
      hits.value = res.hits
    } catch {
      hits.value = []
    } finally {
      searching.value = false
    }
  }, 250)
})

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })
}
</script>

<template>
  <div class="p-5 max-w-[40rem] flex flex-col h-full overflow-hidden">
    <div class="mb-4 shrink-0">
      <h2 class="text-base font-semibold mb-2.5">Chats</h2>
      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.3rem] text-[var(--c-484848)]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="query" class="flex-1 bg-none border-none text-[var(--c-c0c0c0)] text-[0.8125rem] font-[inherit] outline-none min-w-0 placeholder:text-[var(--c-404040)]" placeholder="Search all conversations…" />
        <button v-if="query" class="bg-none border-none text-[var(--c-484848)] cursor-pointer text-[0.65rem] p-0 transition-colors duration-100 hover:text-muted" @click="query = ''">✕</button>
      </div>
    </div>

    <div class="flex-1 overflow-y-auto">
      <!-- Search results -->
      <template v-if="query.trim()">
        <div v-if="searching" class="text-[var(--c-505050)] text-sm py-2">Searching…</div>
        <p v-else-if="hits.length === 0" class="text-[var(--c-505050)] text-sm py-2">No matches.</p>
        <ul v-else class="list-none flex flex-col gap-1.5">
          <li v-for="h in hits" :key="h.message_id">
            <button class="w-full bg-surface rounded-md py-[0.55rem] px-[0.8rem] border-none text-left cursor-pointer font-[inherit] hover:bg-[var(--c-1a1a1a)]" @click="open(h.session_id)">
              <div class="flex items-center gap-2 mb-[0.2rem]">
                <span class="text-[0.8rem] text-[var(--c-d0d0d0)] font-medium truncate">{{ h.session_title }}</span>
                <span class="text-[0.68rem] text-[var(--c-505050)] whitespace-nowrap ml-auto">{{ fmtDate(h.created_at) }}</span>
              </div>
              <p class="text-[0.75rem] text-[var(--c-808080)] leading-[1.4] break-words">
                <span class="uppercase text-[0.62rem] text-[var(--c-585858)] mr-1">{{ h.role }}</span>{{ h.snippet }}
              </p>
            </button>
          </li>
        </ul>
      </template>

      <!-- Session list (default) -->
      <template v-else>
        <ul v-if="store.sessions.length" class="list-none flex flex-col gap-1.5">
          <li v-for="s in store.sessions" :key="s.id" class="flex items-center gap-3 bg-surface rounded-md py-[0.6rem] px-[0.8rem]">
            <button class="flex-1 bg-none border-none text-[var(--c-d0d0d0)] text-left cursor-pointer text-sm font-[inherit] hover:text-[var(--c-ffffff)]" @click="open(s.id)">{{ s.title }}</button>
            <span class="text-xs text-[var(--c-505050)] whitespace-nowrap">{{ new Date(s.updated_at).toLocaleDateString() }}</span>
            <button class="bg-none border-none text-[var(--c-505050)] cursor-pointer text-xs py-[0.2rem] px-[0.4rem] rounded-[0.2rem] hover:text-[var(--c-d08080)]" @click="remove(s.id)">✕</button>
          </li>
        </ul>
        <p v-else class="text-[var(--c-505050)] text-sm">No chats yet.</p>
      </template>
    </div>
  </div>
</template>
