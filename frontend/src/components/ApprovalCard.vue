<script setup lang="ts">
import { ref } from 'vue'

const props = withDefaults(
  defineProps<{
    toolName: string
    toolArgs: string
    /** Header text; defaults to the raw tool name. */
    label?: string
  }>(),
  { label: undefined },
)

const emit = defineEmits<{
  /** Approve, optionally with operator-edited args (undefined = run verbatim). */
  (e: 'approve', editedArgs?: Record<string, unknown>): void
  (e: 'reject'): void
}>()

type Field = {
  key: string
  label: string
  kind: 'textarea' | 'text' | 'number' | 'select' | 'date' | 'readonly'
  options?: string[]
  step?: number
}

// Friendly editors for the helpdesk write tools. Anything not listed falls
// back to a read-only JSON view (approve runs the model's draft verbatim).
const FIELD_SPECS: Record<string, Field[]> = {
  helpdesk_reply_ticket: [
    { key: 'ticket_id', label: 'Ticket', kind: 'readonly' },
    { key: 'type', label: 'Type', kind: 'select', options: ['reply', 'internal_note'] },
    { key: 'body', label: 'Message', kind: 'textarea' },
  ],
  helpdesk_log_time: [
    { key: 'ticket_id', label: 'Ticket', kind: 'readonly' },
    { key: 'duration_minutes', label: 'Minutes', kind: 'number', step: 15 },
    { key: 'work_type', label: 'Work type', kind: 'select', options: ['remote', 'on_site'] },
    { key: 'logged_at', label: 'Date', kind: 'date' },
    { key: 'description', label: 'Description', kind: 'textarea' },
  ],
  helpdesk_create_ticket: [
    { key: 'client_id', label: 'Client id', kind: 'readonly' },
    { key: 'user_id', label: 'Requester id', kind: 'readonly' },
    { key: 'subject', label: 'Subject', kind: 'text' },
    { key: 'priority', label: 'Priority', kind: 'select', options: ['low', 'medium', 'high', 'critical'] },
    { key: 'description', label: 'Description', kind: 'textarea' },
  ],
  helpdesk_update_ticket: [
    { key: 'ticket_id', label: 'Ticket', kind: 'readonly' },
    { key: 'status', label: 'Status', kind: 'select', options: ['open', 'in_progress', 'pending_user', 'resolved', 'closed'] },
    { key: 'priority', label: 'Priority', kind: 'select', options: ['low', 'medium', 'high', 'critical'] },
  ],
  helpdesk_delete_time: [
    { key: 'ticket_id', label: 'Ticket', kind: 'readonly' },
    { key: 'time_entry_id', label: 'Time entry to delete', kind: 'readonly' },
  ],
}

const fields = FIELD_SPECS[props.toolName]

function parseArgs(): Record<string, unknown> {
  try {
    const v = JSON.parse(props.toolArgs)
    return v && typeof v === 'object' ? v : {}
  } catch {
    return {}
  }
}

const parsed = parseArgs()

// Editable working copy, seeded from the model's draft. Missing select values
// default to the first option so the control is never blank.
const edited = ref<Record<string, unknown>>({ ...parsed })
if (fields) {
  for (const f of fields) {
    if (edited.value[f.key] == null && f.kind === 'select' && f.options) {
      edited.value[f.key] = f.options[0]
    }
  }
}

function prettyArgs(): string {
  try {
    return JSON.stringify(parsed, null, 2)
  } catch {
    return props.toolArgs
  }
}

function onApprove() {
  // Editable tools send the full (possibly edited) arg set; the backend uses it
  // verbatim. Unknown tools send nothing, so the model's draft runs as-is.
  emit('approve', fields ? { ...edited.value } : undefined)
}
</script>

<template>
  <div class="flex flex-col gap-2 bg-[var(--c-1a1610)] border border-[var(--c-4a3a1a)] rounded-lg py-2.5 px-3">
    <div class="flex items-center gap-2 text-[0.8rem] text-[var(--c-e0b060)]">
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" />
      </svg>
      <span class="font-medium">{{ label ?? toolName }} — approval required</span>
      <slot name="header-extra" />
    </div>

    <!-- Friendly per-field editors for the helpdesk write tools. -->
    <div v-if="fields" class="flex flex-col gap-2">
      <label v-for="f in fields" :key="f.key" class="flex flex-col gap-0.5">
        <span class="text-[0.66rem] uppercase tracking-[0.05em] text-[var(--c-8a7a50)]">{{ f.label }}</span>

        <span v-if="f.kind === 'readonly'" class="text-[0.78rem] text-[var(--c-a09070)]">{{ edited[f.key] }}</span>

        <textarea
          v-else-if="f.kind === 'textarea'"
          v-model="edited[f.key] as string"
          rows="4"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-2 font-[inherit] resize-y focus:outline-none focus:border-[var(--c-4a3a1a)]"
        />

        <select
          v-else-if="f.kind === 'select'"
          v-model="edited[f.key] as string"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] focus:outline-none focus:border-[var(--c-4a3a1a)]"
        >
          <option v-for="opt in f.options" :key="opt" :value="opt">{{ opt }}</option>
        </select>

        <input
          v-else-if="f.kind === 'number'"
          v-model.number="edited[f.key] as number"
          type="number"
          :step="f.step"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] w-32 focus:outline-none focus:border-[var(--c-4a3a1a)]"
        />

        <input
          v-else-if="f.kind === 'date'"
          v-model="edited[f.key] as string"
          type="date"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] w-44 focus:outline-none focus:border-[var(--c-4a3a1a)]"
        />

        <input
          v-else
          v-model="edited[f.key] as string"
          type="text"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] focus:outline-none focus:border-[var(--c-4a3a1a)]"
        />
      </label>
    </div>

    <!-- Fallback: read-only args for tools without a friendly editor. -->
    <pre v-else class="text-[0.72rem] text-[var(--c-a09070)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-2 overflow-x-auto whitespace-pre-wrap max-h-40">{{ prettyArgs() }}</pre>

    <div class="flex items-center gap-2">
      <button class="bg-[var(--c-1e3a2a)] text-[var(--c-6ecf8e)] border border-[var(--c-2a5a3a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254a35)]" @click="onApprove">{{ fields ? 'Approve & submit' : 'Approve' }}</button>
      <button class="bg-[var(--c-3a1e1e)] text-[var(--c-df7a7a)] border border-[var(--c-5a2a2a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-4a2525)]" @click="emit('reject')">Deny</button>
      <slot name="footer-extra" />
    </div>
  </div>
</template>
