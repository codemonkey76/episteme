<script setup lang="ts">
import { ref, computed } from 'vue'
import * as api from '../api'
import { useTerminalsStore } from '../stores/terminals'

const store = useTerminalsStore()

const request = ref('')
const loading = ref(false)
const error = ref('')
// The current suggestion: editable before it's pasted.
const suggestion = ref<{ request: string; command: string } | null>(null)

const hasTerminal = computed(() => store.activeId !== null)
const targetShell = computed(() => store.activeShell ?? 'bash')

async function ask() {
  const q = request.value.trim()
  if (!q || loading.value) return
  loading.value = true
  error.value = ''
  suggestion.value = null
  try {
    const ctx = store.activeScrollback()
    const res = await api.terminals.suggest(targetShell.value, q, ctx || undefined)
    suggestion.value = { request: q, command: res.command }
    request.value = ''
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Suggestion failed'
  } finally {
    loading.value = false
  }
}

function approve() {
  if (!suggestion.value) return
  if (!store.pasteToActive(suggestion.value.command)) {
    error.value = 'No active terminal — click into a terminal first.'
    return
  }
  suggestion.value = null
}

function dismiss() {
  suggestion.value = null
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <div class="px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex items-center gap-2">
      <span class="text-[0.8rem] font-medium text-fg">Terminal AI</span>
      <span class="text-[0.7rem] text-[var(--c-585858)] ml-auto">
        target: {{ hasTerminal ? (targetShell === 'pwsh' ? 'PowerShell' : 'bash') : 'no terminal open' }}
      </span>
    </div>

    <div class="flex-1 overflow-y-auto p-3 flex flex-col gap-3">
      <p v-if="!hasTerminal" class="text-[0.78rem] text-[var(--c-808080)]">
        Open a Terminal or PowerShell window and click into it — suggestions paste onto that prompt.
      </p>

      <p class="text-[0.78rem] text-[var(--c-707070)]">
        Ask for a command in plain language. The suggestion appears below; review or edit it, then
        <strong>Approve &amp; paste</strong> drops it onto the prompt — it does not run until you press Enter.
      </p>

      <div v-if="error" class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] text-[0.78rem] rounded px-3 py-2">{{ error }}</div>

      <!-- The editable suggestion -->
      <div v-if="suggestion" class="flex flex-col gap-2 bg-surface border border-raised rounded-lg p-3">
        <div class="text-[0.7rem] uppercase tracking-[0.05em] text-[var(--c-707070)]">Suggested command</div>
        <textarea
          v-model="suggestion.command"
          rows="2"
          class="font-mono text-[0.8rem] text-[var(--c-d4d4d4)] bg-[#0d0d0d] border border-[var(--c-2a2a2a)] rounded p-2 resize-y focus:outline-none focus:border-[var(--c-3a3a3a)]"
        />
        <div class="flex items-center gap-2">
          <button
            class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-3 py-1 text-xs cursor-pointer hover:bg-[var(--c-254a35)] disabled:opacity-40 disabled:cursor-not-allowed"
            :disabled="!hasTerminal"
            @click="approve"
          >Approve &amp; paste</button>
          <button class="bg-[var(--c-2a2a2a)] text-[var(--c-b0b0b0)] border border-[var(--c-333333)] rounded px-3 py-1 text-xs cursor-pointer hover:bg-[var(--c-333333)]" @click="dismiss">Dismiss</button>
        </div>
      </div>
    </div>

    <div class="border-t border-[var(--c-1e1e1e)] p-2 shrink-0 flex gap-2">
      <input
        v-model="request"
        :placeholder="loading ? 'Thinking…' : 'e.g. list files by size, biggest first'"
        :disabled="loading"
        class="flex-1 text-[0.82rem] text-fg bg-[var(--c-141414)] border border-[var(--c-2a2a2a)] rounded px-3 py-2 focus:outline-none focus:border-[var(--c-3a3a3a)]"
        @keydown.enter.prevent="ask"
      />
      <button
        class="bg-[var(--c-2a4a7a)] text-fg border-none rounded-md py-2 px-4 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
        :disabled="loading || !request.trim()"
        @click="ask"
      >Ask</button>
    </div>
  </div>
</template>
