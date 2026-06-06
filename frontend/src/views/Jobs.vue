<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import { useSessionsStore } from '../stores/sessions'
import { useWindowsStore } from '../stores/windows'
import { useLogsStore } from '../stores/logs'
import * as api from '../api'

const sessions = useSessionsStore()
const winStore = useWindowsStore()
const logs = useLogsStore()

const jobList = ref<api.Job[]>([])
const pending = ref<api.PendingActionGlobal[]>([])
const loading = ref(false)
const error = ref('')

async function load() {
  loading.value = true
  try {
    const [j, p] = await Promise.all([api.jobs.list(), api.jobs.pendingAll()])
    jobList.value = j.jobs
    pending.value = p.pending_actions
    error.value = ''
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load jobs'
  } finally {
    loading.value = false
  }
}

// Approving the last pending action of a session resumes its job server-side;
// keep polling while open so statuses flip live.
let poll: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  load()
  poll = setInterval(load, 3000)
})
onBeforeUnmount(() => {
  if (poll) clearInterval(poll)
})

async function decide(a: api.PendingActionGlobal, approved: boolean) {
  pending.value = pending.value.filter(x => x.id !== a.id)
  try {
    if (approved) await api.approvals.approve(a.id)
    else await api.approvals.reject(a.id)
    logs.info('Jobs', `${approved ? 'Approved' : 'Denied'}: ${a.tool_name} (${a.session_title})`)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Decision failed'
  }
  load()
}

async function openSession(id: string) {
  await sessions.loadSession(id)
  winStore.openKey('chat', undefined, 'fill')
}

function prettyArgs(args: string): string {
  try {
    return JSON.stringify(JSON.parse(args), null, 2)
  } catch {
    return args
  }
}

const STATUS_STYLE: Record<string, string> = {
  running: 'text-[var(--c-7ab0ff)] border-[var(--c-2a4a8a)]',
  needs_approval: 'text-[var(--c-e0b060)] border-[var(--c-4a3a1a)]',
  done: 'text-[var(--c-7adfbb)] border-[var(--c-2a5a3a)]',
  failed: 'text-[var(--c-df7a7a)] border-[var(--c-5a2a2a)]',
}

function fmtTime(iso: string): string {
  return new Date(iso).toLocaleString([], {
    month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
  })
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <div v-if="error" class="px-3 py-2 text-danger text-[0.775rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

    <div class="flex-1 overflow-y-auto p-3 flex flex-col gap-4">
      <!-- Approval queue -->
      <section>
        <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em] mb-2">Needs your approval</h3>
        <p v-if="pending.length === 0" class="text-[var(--c-383838)] text-[0.8125rem]">Nothing waiting. Gated tools from background runs appear here.</p>
        <div v-else class="flex flex-col gap-2">
          <div v-for="a in pending" :key="a.id" class="flex flex-col gap-2 bg-[var(--c-1a1610)] border border-[var(--c-4a3a1a)] rounded-lg p-2.5">
            <div class="flex items-center gap-2 text-[0.8rem] text-[var(--c-e0b060)]">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
              </svg>
              <span class="font-medium">{{ a.tool_name }}</span>
              <button class="ml-auto bg-none border-none text-[var(--c-8a7a50)] text-[0.7rem] cursor-pointer hover:text-[var(--c-e0b060)] truncate max-w-[12rem]" :title="a.session_title" @click="openSession(a.session_id)">{{ a.session_title }}</button>
            </div>
            <pre class="text-[0.72rem] text-[var(--c-a09070)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-2 overflow-x-auto whitespace-pre-wrap max-h-36">{{ prettyArgs(a.tool_args) }}</pre>
            <div class="flex items-center gap-2">
              <button class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254a35)]" @click="decide(a, true)">Approve</button>
              <button class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-4a2525)]" @click="decide(a, false)">Deny</button>
              <span class="ml-auto text-[0.68rem] text-[var(--c-585858)]">{{ fmtTime(a.created_at) }}</span>
            </div>
          </div>
        </div>
      </section>

      <!-- Jobs -->
      <section>
        <h3 class="text-[0.7rem] font-semibold text-[var(--c-585858)] uppercase tracking-[0.07em] mb-2">Recent runs</h3>
        <div v-if="loading && jobList.length === 0" class="text-[var(--c-484848)] text-[0.8125rem] py-2">Loading…</div>
        <p v-else-if="jobList.length === 0" class="text-[var(--c-383838)] text-[0.8125rem]">No background or scheduled runs yet. Ask the assistant to "do something in the background", or set up Settings → Agents.</p>
        <div v-else class="flex flex-col gap-1.5">
          <div v-for="j in jobList" :key="j.id" class="flex items-start gap-3 bg-surface rounded-md py-2 px-2.5">
            <span :class="['shrink-0 mt-[0.1rem] text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border', STATUS_STYLE[j.status] ?? '']">{{ j.status.replace('_', ' ') }}</span>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-1.5">
                <span class="text-[0.8125rem] text-[var(--c-d0d0d0)] truncate">{{ j.name }}</span>
                <span class="text-[0.65rem] text-[var(--c-505050)]">{{ j.kind === 'scheduled' ? '⏰' : '⚙' }}</span>
              </div>
              <p v-if="j.summary" class="text-[0.72rem] text-[var(--c-808080)] leading-[1.4] break-words line-clamp-2">{{ j.summary }}</p>
              <p v-else-if="j.error" class="text-[0.72rem] text-danger leading-[1.4] break-words">{{ j.error }}</p>
              <span class="text-[0.65rem] text-[var(--c-505050)]">{{ fmtTime(j.updated_at) }}</span>
            </div>
            <button class="shrink-0 bg-none border border-raised rounded text-[var(--c-808080)] text-[0.7rem] px-2 py-[0.2rem] cursor-pointer font-[inherit] hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" @click="openSession(j.session_id)">Open</button>
          </div>
        </div>
      </section>
    </div>

    <div class="px-3 py-[0.25rem] border-t border-surface text-[var(--c-505050)] text-[0.68rem] shrink-0">
      {{ pending.length }} awaiting approval · {{ jobList.length }} run{{ jobList.length === 1 ? '' : 's' }}
    </div>
  </div>
</template>
