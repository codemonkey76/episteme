<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'

const logs = useLogsStore()

const items = ref<api.PromptInfo[]>([])
const loading = ref(false)
const error = ref('')

const selectedKey = ref<string | null>(null)
const draft = ref('')
const saving = ref(false)

const selected = computed(() => items.value.find(p => p.key === selectedKey.value) ?? null)
const dirty = computed(() => selected.value !== null && draft.value !== selected.value.content)
const isDefault = computed(() => selected.value !== null && draft.value === selected.value.default)

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.prompts.list()
    items.value = res.prompts
    if (!selectedKey.value && res.prompts.length) select(res.prompts[0].key)
    else if (selected.value) draft.value = selected.value.content
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load prompts'
  } finally {
    loading.value = false
  }
}

function select(key: string) {
  if (dirty.value && !confirm('Discard unsaved changes to this prompt?')) return
  selectedKey.value = key
  draft.value = items.value.find(p => p.key === key)?.content ?? ''
}

async function save() {
  const p = selected.value
  if (!p) return
  saving.value = true
  error.value = ''
  try {
    await api.prompts.save(p.key, draft.value)
    // The backend clears the override when the text matches the default.
    p.content = draft.value.trim() === '' ? p.default : draft.value.trimEnd()
    p.customized = p.content !== p.default
    draft.value = p.content
    logs.info('Prompts', `Saved prompt: ${p.name}`)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to save prompt'
  } finally {
    saving.value = false
  }
}

async function resetToDefault() {
  const p = selected.value
  if (!p) return
  if (p.customized && !confirm(`Reset "${p.name}" to the built-in default? Your custom version will be lost.`)) return
  saving.value = true
  error.value = ''
  try {
    await api.prompts.reset(p.key)
    p.content = p.default
    p.customized = false
    draft.value = p.default
    logs.info('Prompts', `Reset prompt to default: ${p.name}`)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to reset prompt'
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="flex h-full bg-bg overflow-hidden">
    <!-- Prompt list -->
    <nav class="w-[200px] min-w-[200px] border-r border-[var(--c-1e1e1e)] overflow-y-auto p-1.5 flex flex-col gap-0.5">
      <button
        v-for="p in items"
        :key="p.key"
        :class="['flex items-center gap-2 px-2.5 py-2 rounded-md text-[0.8125rem] bg-none border-none cursor-pointer w-full text-left font-[inherit] transition-[background,color] duration-120 whitespace-nowrap', selectedKey === p.key ? 'bg-[var(--c-222222)] text-fg' : 'text-[var(--c-808080)] hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-d0d0d0)]']"
        @click="select(p.key)"
      >
        <span class="flex-1 overflow-hidden text-ellipsis">{{ p.name }}</span>
        <span v-if="p.customized" class="w-1.5 h-1.5 rounded-full bg-[var(--c-7ab0ff)] shrink-0" title="Customized — differs from the built-in default" />
      </button>
      <div v-if="loading && items.length === 0" class="flex items-center justify-center py-6">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
    </nav>

    <!-- Editor -->
    <div class="flex-1 flex flex-col min-w-0">
      <div v-if="error" class="px-4 py-2 text-danger text-[0.775rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

      <template v-if="selected">
        <div class="px-4 pt-3.5 pb-2.5 border-b border-[var(--c-1e1e1e)] shrink-0">
          <div class="flex items-center gap-2">
            <h2 class="text-[0.9rem] font-semibold text-fg m-0">{{ selected.name }}</h2>
            <span v-if="selected.customized" class="text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border text-[var(--c-7ab0ff)] border-[var(--c-7ab0ff55)]">customized</span>
            <span v-else class="text-[0.62rem] font-semibold uppercase tracking-[0.04em] px-1.5 py-[0.1rem] rounded border text-[var(--c-606060)] border-[var(--c-333333)]">default</span>
          </div>
          <p class="text-[0.75rem] text-[var(--c-808080)] leading-[1.5] mt-1.5 mb-0">{{ selected.description }}</p>
          <div v-if="selected.variables.length" class="flex items-center gap-1.5 mt-2 flex-wrap">
            <span class="text-[0.68rem] text-[var(--c-505050)]">Placeholders filled in at runtime:</span>
            <code v-for="v in selected.variables" :key="v" class="text-[0.7rem] text-[var(--c-7adfbb)] bg-[var(--c-11201a)] border border-[var(--c-1c3a2c)] rounded px-1.5 py-[0.05rem]">{{ v }}</code>
          </div>
        </div>

        <textarea
          v-model="draft"
          class="flex-1 bg-[var(--c-111111)] text-[var(--c-d0d0d0)] border-none outline-none resize-none px-4 py-3 text-[0.78rem] leading-[1.55] font-mono min-h-0"
          spellcheck="false"
        />

        <div class="flex items-center gap-2 px-4 py-2.5 border-t border-[var(--c-1e1e1e)] shrink-0">
          <button
            class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3.5 py-1.5 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50"
            :disabled="!dirty || saving"
            @click="save"
          >{{ saving ? 'Saving…' : 'Save' }}</button>
          <button
            class="bg-surface text-[var(--c-909090)] border border-raised rounded px-3 py-1.5 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-222222)] hover:not-disabled:text-[var(--c-c0c0c0)] disabled:opacity-40"
            :disabled="(isDefault && !selected.customized) || saving"
            @click="resetToDefault"
          >Reset to default</button>
          <span v-if="dirty" class="text-[0.7rem] text-[var(--c-b08a4a)] ml-auto">Unsaved changes</span>
        </div>
      </template>

      <div v-else-if="!loading" class="flex items-center justify-center h-full text-[var(--c-383838)] text-[0.8125rem]">
        Select a prompt to edit.
      </div>
    </div>
  </div>
</template>
