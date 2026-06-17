<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import * as api from '../api'

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
  kind:
    | 'textarea'
    | 'text'
    | 'number'
    | 'select'
    | 'date'
    | 'checkbox'
    | 'readonly'
    // Helpdesk pickers, resolved to names via /helpdesk/clients (create ticket).
    | 'client-select'
    | 'requester-select'
    // Ticket category, resolved via /helpdesk/categories (create ticket).
    | 'category-select'
  options?: string[]
  /** Friendly labels for select options (value → label); falls back to the value. */
  optionLabels?: Record<string, string>
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
    {
      key: 'status',
      label: 'Set status',
      kind: 'select',
      options: ['', 'open', 'in_progress', 'pending_user', 'resolved', 'closed'],
      optionLabels: {
        '': '— No change —',
        in_progress: 'In progress',
        pending_user: 'Pending user',
      },
    },
    { key: 'description', label: 'Description', kind: 'textarea' },
  ],
  helpdesk_create_ticket: [
    { key: 'client_id', label: 'Client', kind: 'client-select' },
    { key: 'user_id', label: 'Requester', kind: 'requester-select' },
    { key: 'category_id', label: 'Category', kind: 'category-select' },
    { key: 'subject', label: 'Subject', kind: 'text' },
    { key: 'priority', label: 'Priority', kind: 'select', options: ['low', 'medium', 'high', 'critical'] },
    { key: 'description', label: 'Description', kind: 'textarea' },
    { key: 'silent', label: 'Silent (no customer emails)', kind: 'checkbox' },
  ],
  helpdesk_update_ticket: [
    { key: 'ticket_id', label: 'Ticket', kind: 'readonly' },
    { key: 'status', label: 'Status', kind: 'select', options: ['open', 'in_progress', 'pending_user', 'resolved', 'closed'] },
    { key: 'priority', label: 'Priority', kind: 'select', options: ['low', 'medium', 'high', 'critical'] },
    { key: 'assign_to_me', label: 'Assign to me', kind: 'checkbox' },
    { key: 'silent', label: 'Silent (no customer emails)', kind: 'checkbox' },
  ],
  helpdesk_create_user: [
    { key: 'name', label: 'Name', kind: 'text' },
    { key: 'email', label: 'Email', kind: 'text' },
    { key: 'mobile', label: 'Mobile (optional)', kind: 'text' },
    { key: 'client_ids', label: 'Client ids', kind: 'readonly' },
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

// Today's date as YYYY-MM-DD in the operator's local timezone — the default for
// empty date fields (e.g. time-entry "logged at").
function todayLocal(): string {
  const d = new Date()
  const mm = String(d.getMonth() + 1).padStart(2, '0')
  const dd = String(d.getDate()).padStart(2, '0')
  return `${d.getFullYear()}-${mm}-${dd}`
}

// Editable working copy, seeded from the model's draft. Missing select values
// default to the first option so the control is never blank; empty date fields
// default to today.
const edited = ref<Record<string, unknown>>({ ...parsed })
if (fields) {
  for (const f of fields) {
    if (edited.value[f.key] == null && f.kind === 'select' && f.options) {
      edited.value[f.key] = f.options[0]
    }
    if (f.kind === 'date' && !edited.value[f.key]) {
      edited.value[f.key] = todayLocal()
    }
  }
}

// Create-ticket card: load the helpdesk clients (each with their contacts) so
// the client_id/user_id ids can be shown as names and changed via dropdowns.
const isCreateTicket = props.toolName === 'helpdesk_create_ticket'
const clients = ref<api.HelpdeskClient[]>([])
const clientsLoaded = ref(false)
const clientsError = ref<string | null>(null)
const categories = ref<api.HelpdeskCategory[]>([])

// Normalize the category to a number or null so the dropdown (which offers a
// "no category" option) always has a matching selection.
if (isCreateTicket) {
  edited.value.category_id =
    typeof parsed.category_id === 'number' ? parsed.category_id : null
}

onMounted(async () => {
  if (!isCreateTicket) return
  const integration = typeof parsed.integration === 'string' ? parsed.integration : undefined
  try {
    const res = await api.helpdesk.listClients({ integration })
    clients.value = res.clients
    // If the draft's requester isn't a contact of the (resolved) client, snap it
    // to a valid one so the dropdown isn't left on a stale, unselectable id.
    if (selectedClient.value) onClientChange()
  } catch (e) {
    clientsError.value = e instanceof Error ? e.message : String(e)
  } finally {
    clientsLoaded.value = true
  }
  // Categories are best-effort — a failure just leaves the picker empty.
  try {
    const res = await api.helpdesk.listCategories({ integration })
    categories.value = res.categories
  } catch { /* ignore — category stays optional */ }
})

function categoryName(id: unknown): string {
  const c = categories.value.find((x) => x.id === Number(id))
  return c ? c.name : id == null ? 'None' : String(id)
}

const selectedClient = computed(() =>
  clients.value.find((c) => c.id === Number(edited.value.client_id)),
)
const requesterOptions = computed(() => selectedClient.value?.users ?? [])

function clientName(id: unknown): string {
  const c = clients.value.find((x) => x.id === Number(id))
  return c ? c.name : String(id ?? '')
}
function requesterName(id: unknown): string {
  const u = requesterOptions.value.find((x) => x.id === Number(id))
  return u ? u.name || u.email : String(id ?? '')
}

// When the client changes, keep the requester valid — reset to the first
// contact of the newly chosen client if the current one doesn't belong to it.
function onClientChange() {
  const users = requesterOptions.value
  if (!users.some((u) => u.id === Number(edited.value.user_id))) {
    edited.value.user_id = users[0]?.id
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

        <!-- Helpdesk client picker (names). Falls back to the raw id while the
             list loads or if it can't be fetched. -->
        <select
          v-else-if="f.kind === 'client-select' && clientsLoaded && !clientsError && clients.length"
          v-model.number="edited[f.key]"
          @change="onClientChange"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] focus:outline-none focus:border-[var(--c-4a3a1a)]"
        >
          <option v-for="c in clients" :key="c.id" :value="c.id">{{ c.name }}</option>
        </select>

        <!-- Helpdesk requester picker — the chosen client's contacts. -->
        <select
          v-else-if="f.kind === 'requester-select' && clientsLoaded && !clientsError && requesterOptions.length"
          v-model.number="edited[f.key]"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] focus:outline-none focus:border-[var(--c-4a3a1a)]"
        >
          <option v-for="u in requesterOptions" :key="u.id" :value="u.id">{{ u.name || u.email }}</option>
        </select>

        <span
          v-else-if="f.kind === 'client-select' || f.kind === 'requester-select'"
          class="text-[0.78rem] text-[var(--c-a09070)]"
        >
          {{ f.kind === 'client-select' ? clientName(edited[f.key]) : requesterName(edited[f.key]) }}
          <span v-if="!clientsLoaded" class="text-[var(--c-8a7a50)]"> · loading…</span>
          <span v-else-if="clientsError" class="text-[var(--c-8a7a50)]"> · (couldn't load list)</span>
        </span>

        <!-- Helpdesk category picker — optional; "No category" leaves it unset. -->
        <select
          v-else-if="f.kind === 'category-select' && categories.length"
          v-model="edited[f.key]"
          class="text-[0.78rem] text-[var(--c-d0c8b0)] bg-[var(--c-12100a)] border border-[var(--c-2a2418)] rounded p-1.5 font-[inherit] focus:outline-none focus:border-[var(--c-4a3a1a)]"
        >
          <option :value="null">— No category —</option>
          <option v-for="c in categories" :key="c.id" :value="c.id" :title="c.description">{{ c.name }}</option>
        </select>

        <span v-else-if="f.kind === 'category-select'" class="text-[0.78rem] text-[var(--c-a09070)]">
          {{ categoryName(edited[f.key]) }}
        </span>

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
          <option v-for="opt in f.options" :key="opt" :value="opt">{{ f.optionLabels?.[opt] ?? opt }}</option>
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
          v-else-if="f.kind === 'checkbox'"
          v-model="edited[f.key] as boolean"
          type="checkbox"
          class="w-4 h-4 self-start accent-[var(--c-e0b060)]"
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
