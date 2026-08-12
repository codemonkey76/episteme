<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import * as api from '../api'
import { useShipmentsStore } from '../stores/shipments'

const shipmentsStore = useShipmentsStore()

const items = ref<api.Shipment[]>([])
const loading = ref(false)
const error = ref('')

const searchQuery = ref('')
/** 'active' hides delivered/cancelled — the default, since an arrived parcel stops mattering. */
const filter = ref<'active' | 'all' | 'delivered'>('active')
const expandedId = ref<string | null>(null)

const STATUSES: { value: api.ShipmentStatus; label: string }[] = [
  { value: 'ordered', label: 'Ordered' },
  { value: 'in_transit', label: 'In transit' },
  { value: 'out_for_delivery', label: 'Out for delivery' },
  { value: 'delivered', label: 'Delivered' },
  { value: 'exception', label: 'Problem' },
  { value: 'cancelled', label: 'Cancelled' },
]

function statusLabel(s: api.ShipmentStatus): string {
  return STATUSES.find(x => x.value === s)?.label ?? s
}

/** Pill colours: blue while moving, green on arrival, amber for trouble. */
function statusClass(s: api.ShipmentStatus): string {
  switch (s) {
    case 'delivered':
      return 'bg-[var(--c-1c3a2c)] text-[var(--c-8edfae)] border-[var(--c-254a35)]'
    case 'exception':
      return 'bg-[var(--c-3a2a10)] text-[var(--c-e0b060)] border-[var(--c-5a4520)]'
    case 'cancelled':
      return 'bg-[var(--c-2a2a2a)] text-[var(--c-808080)] border-[var(--c-3a3a3a)]'
    case 'out_for_delivery':
      return 'bg-[var(--c-1e3a6e)] text-[var(--c-9cc0f0)] border-[var(--c-2a4a8a)]'
    default:
      return 'bg-[var(--c-1a2a4a)] text-[var(--c-7ab0ff)] border-[var(--c-23304a)]'
  }
}

const filtered = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return items.value
  return items.value.filter(s =>
    [s.label, s.description, s.merchant, s.carrier, s.tracking_number]
      .some(v => v?.toLowerCase().includes(q)),
  )
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    const status = filter.value === 'all' ? 'all' : filter.value
    const res = await api.shipments.list({ status })
    items.value = res.shipments
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load shipments'
  } finally {
    loading.value = false
  }
}

onMounted(load)
watch(filter, load)
// Refresh when the chat AI or the email categorizer touches the shipments.
watch(() => shipmentsStore.changeToken, load)

// ── Add / edit composer ──────────────────────────────────────────────────────
type Draft = {
  label: string
  merchant: string
  carrier: string
  tracking_number: string
  tracking_url: string
  order_ref: string
  status: api.ShipmentStatus
  eta: string
  description: string
}

const adding = ref(false)
const editingId = ref<string | null>(null)
const draft = ref<Draft>(emptyDraft())

function emptyDraft(): Draft {
  return {
    label: '',
    merchant: '',
    carrier: '',
    tracking_number: '',
    tracking_url: '',
    order_ref: '',
    status: 'ordered',
    eta: '',
    description: '',
  }
}

/** ISO (UTC) → the local `datetime-local` value the input expects. */
function toLocalInput(iso: string | null): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function fromLocalInput(v: string): string | null {
  if (!v) return null
  const d = new Date(v)
  return Number.isNaN(d.getTime()) ? null : d.toISOString()
}

function startAdd() {
  editingId.value = null
  draft.value = emptyDraft()
  adding.value = true
}

function startEdit(s: api.Shipment) {
  adding.value = false
  expandedId.value = null
  editingId.value = s.id
  draft.value = {
    label: s.label,
    merchant: s.merchant ?? '',
    carrier: s.carrier ?? '',
    tracking_number: s.tracking_number ?? '',
    tracking_url: s.tracking_url ?? '',
    order_ref: s.order_ref ?? '',
    status: s.status,
    eta: toLocalInput(s.eta),
    description: s.description ?? '',
  }
}

function cancelEdit() {
  adding.value = false
  editingId.value = null
}

/** Blank text fields are sent as null so the backend clears them. */
function draftFields(): api.ShipmentFields {
  const d = draft.value
  const orNull = (v: string) => (v.trim() ? v.trim() : null)
  return {
    label: d.label.trim(),
    merchant: orNull(d.merchant),
    carrier: orNull(d.carrier),
    tracking_number: orNull(d.tracking_number),
    tracking_url: orNull(d.tracking_url),
    order_ref: orNull(d.order_ref),
    status: d.status,
    eta: fromLocalInput(d.eta),
    description: orNull(d.description),
  }
}

async function saveDraft() {
  if (!draft.value.label.trim()) return
  error.value = ''
  try {
    if (editingId.value) {
      const res = await api.shipments.update(editingId.value, draftFields())
      const idx = items.value.findIndex(x => x.id === editingId.value)
      if (idx !== -1) items.value[idx] = res.shipment
      editingId.value = null
    } else {
      const res = await api.shipments.create({ ...draftFields(), label: draft.value.label.trim() })
      items.value.unshift(res.shipment)
      adding.value = false
    }
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to save shipment'
  }
}

async function remove(s: api.Shipment) {
  try {
    await api.shipments.remove(s.id)
    items.value = items.value.filter(x => x.id !== s.id)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete shipment'
  }
}

/** One-tap "it arrived" — the common case, without opening the editor. */
async function markDelivered(s: api.Shipment) {
  try {
    const res = await api.shipments.update(s.id, { status: 'delivered' })
    if (filter.value === 'active') {
      items.value = items.value.filter(x => x.id !== s.id)
    } else {
      const idx = items.value.findIndex(x => x.id === s.id)
      if (idx !== -1) items.value[idx] = res.shipment
    }
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to update shipment'
  }
}

// ── Photo ────────────────────────────────────────────────────────────────────
const photoTarget = ref<string | null>(null)
const photoInput = ref<HTMLInputElement | null>(null)

function pickPhoto(s: api.Shipment) {
  photoTarget.value = s.id
  photoInput.value?.click()
}

async function onPhotoPicked(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  const id = photoTarget.value
  input.value = ''
  photoTarget.value = null
  if (!file || !id) return
  try {
    const bytes = await fileToBase64(file)
    await api.shipments.setPhoto(id, file.type || 'image/jpeg', bytes)
    // Re-fetch so `updated_at` (the photo URL's cache-buster) moves too.
    await load()
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : 'Failed to upload photo'
  }
}

async function removePhoto(s: api.Shipment) {
  try {
    await api.shipments.removePhoto(s.id)
    await load()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to remove photo'
  }
}

/** Read a File as bare base64 — the `data:…;base64,` prefix is stripped. */
function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = String(reader.result)
      resolve(result.slice(result.indexOf(',') + 1))
    }
    reader.onerror = () => reject(new Error('Could not read the file'))
    reader.readAsDataURL(file)
  })
}

// ── Display helpers ──────────────────────────────────────────────────────────
/** "Tomorrow", "in 3 days", "2 days ago" — an ETA is only useful relative to now. */
function etaText(iso: string | null): string {
  if (!iso) return 'No ETA'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return 'No ETA'
  const date = d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })
  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const days = Math.round((startOfDay(d) - startOfDay(new Date())) / 86_400_000)
  if (days === 0) return `${date} · today`
  if (days === 1) return `${date} · tomorrow`
  if (days === -1) return `${date} · yesterday`
  if (days > 1) return `${date} · in ${days} days`
  return `${date} · ${-days} days ago`
}

/** True when an undelivered parcel is past its promised date. */
function isLate(s: api.Shipment): boolean {
  if (!s.eta || s.status === 'delivered' || s.status === 'cancelled') return false
  return new Date(s.eta).getTime() < Date.now()
}

function fmtEventTime(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return d.toLocaleString([], { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' })
}

function subtitle(s: api.Shipment): string {
  return [s.merchant, s.carrier].filter(Boolean).join(' · ')
}

function toggleExpand(s: api.Shipment) {
  if (editingId.value === s.id) return
  expandedId.value = expandedId.value === s.id ? null : s.id
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0 flex-wrap">
      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-[10rem] text-[var(--c-484848)]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="flex-1 bg-none border-none text-[var(--c-c0c0c0)] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[var(--c-404040)]" placeholder="Search shipments…" />
        <button v-if="searchQuery" class="bg-none border-none text-[var(--c-484848)] cursor-pointer text-[0.65rem] p-0 transition-colors duration-100 hover:text-muted" @click="searchQuery = ''">✕</button>
      </div>

      <div class="flex items-center gap-1 bg-surface border border-raised rounded p-[0.1rem]">
        <button
          v-for="f in (['active', 'delivered', 'all'] as const)"
          :key="f"
          :class="['px-2 py-[0.2rem] text-[0.7rem] rounded bg-none border-none cursor-pointer font-[inherit] transition-colors duration-100', filter === f ? 'bg-[var(--c-222222)] text-[var(--c-c0c0c0)]' : 'text-[var(--c-606060)] hover:text-[var(--c-909090)]']"
          @click="filter = f"
        >{{ f === 'active' ? 'On the way' : f === 'delivered' ? 'Delivered' : 'All' }}</button>
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
      <input v-model="draft.label" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="What's on the way?" />
      <div class="grid grid-cols-2 gap-2">
        <input v-model="draft.merchant" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="From (shop)" />
        <input v-model="draft.carrier" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Carrier" />
        <input v-model="draft.tracking_number" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Tracking number" />
        <input v-model="draft.tracking_url" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Tracking link" />
        <select v-model="draft.status" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)]">
          <option v-for="s in STATUSES" :key="s.value" :value="s.value">{{ s.label }}</option>
        </select>
        <input v-model="draft.eta" type="datetime-local" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)]" />
      </div>
      <div class="flex items-center gap-2">
        <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="!draft.label.trim()" @click="saveDraft">Save</button>
        <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="cancelEdit">Cancel</button>
      </div>
    </div>

    <div v-if="error" class="px-3 py-2 text-danger text-[0.775rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

    <!-- Hidden picker, shared by every card's photo button -->
    <input ref="photoInput" type="file" accept="image/*" class="hidden" @change="onPhotoPicked" />

    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading && items.length === 0" class="flex items-center justify-center h-full text-[var(--c-484848)] text-[0.8125rem]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="filtered.length === 0" class="flex items-center justify-center h-full text-[var(--c-383838)] text-[0.8125rem] text-center px-6">
        {{ items.length === 0
          ? 'Nothing on the way. Add a parcel, or let the email auto-sort pick up your next shipping notice.'
          : 'No shipments match your search.' }}
      </div>
      <div v-else>
        <div v-for="s in filtered" :key="s.id" class="group border-b border-[var(--c-161616)] hover:bg-[var(--c-131313)]">
          <!-- Edit mode -->
          <div v-if="editingId === s.id" class="flex flex-col gap-2 px-3.5 py-2.5">
            <input v-model="draft.label" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[var(--c-3a6adf)]" />
            <div class="grid grid-cols-2 gap-2">
              <input v-model="draft.merchant" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="From (shop)" />
              <input v-model="draft.carrier" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Carrier" />
              <input v-model="draft.tracking_number" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Tracking number" />
              <input v-model="draft.tracking_url" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Tracking link" />
              <select v-model="draft.status" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)]">
                <option v-for="opt in STATUSES" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
              </select>
              <input v-model="draft.eta" type="datetime-local" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)]" />
            </div>
            <textarea v-model="draft.description" rows="2" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-xs font-[inherit] outline-none resize-y focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Notes" />
            <div class="flex items-center gap-2">
              <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:bg-[var(--c-254880)]" @click="saveDraft">Save</button>
              <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="cancelEdit">Cancel</button>
            </div>
          </div>

          <!-- View mode -->
          <template v-else>
            <div class="flex items-start gap-3 px-3.5 py-2.5 cursor-pointer" @click="toggleExpand(s)">
              <!-- Photo of what's on the way -->
              <button
                class="w-12 h-12 shrink-0 rounded border border-raised bg-[var(--c-141414)] overflow-hidden flex items-center justify-center cursor-pointer p-0"
                :title="s.has_photo ? 'Replace photo' : 'Add a photo'"
                @click.stop="pickPhoto(s)"
              >
                <img v-if="s.has_photo" :src="api.shipments.photoUrl(s)" :alt="s.label" class="w-full h-full object-cover" />
                <svg v-else width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="text-[var(--c-3a3a3a)]">
                  <rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/>
                </svg>
              </button>

              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2 flex-wrap">
                  <p class="text-[0.8125rem] text-[var(--c-d0d0d0)] font-medium break-words">{{ s.label }}</p>
                  <span :class="['text-[0.62rem] px-1.5 py-[0.1rem] rounded-full border leading-none', statusClass(s.status)]">{{ statusLabel(s.status) }}</span>
                </div>
                <p v-if="subtitle(s)" class="text-[0.72rem] text-[var(--c-707070)] mt-[0.15rem] break-words">{{ subtitle(s) }}</p>
                <div class="flex items-center gap-2 mt-[0.2rem] flex-wrap">
                  <span :class="['text-[0.68rem]', isLate(s) ? 'text-[var(--c-e0b060)]' : 'text-[var(--c-505050)]']">
                    {{ isLate(s) ? 'Overdue — ' : '' }}{{ etaText(s.eta) }}
                  </span>
                  <a
                    v-if="s.tracking_url"
                    :href="s.tracking_url"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="text-[0.68rem] text-[var(--c-7ab0ff)] no-underline hover:underline"
                    @click.stop
                  >Track ↗</a>
                  <span v-else-if="s.tracking_number" class="text-[0.68rem] text-[var(--c-505050)] font-mono">{{ s.tracking_number }}</span>
                </div>
              </div>

              <div class="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-100">
                <button v-if="s.status !== 'delivered' && s.status !== 'cancelled'" class="text-[var(--c-606060)] hover:text-[var(--c-8edfae)] p-1 cursor-pointer bg-none border-none" title="Mark delivered" @click.stop="markDelivered(s)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                </button>
                <button class="text-[var(--c-606060)] hover:text-[var(--c-a0c0ff)] p-1 cursor-pointer bg-none border-none" title="Edit" @click.stop="startEdit(s)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>
                </button>
                <button class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none" title="Delete" @click.stop="remove(s)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                </button>
              </div>
            </div>

            <!-- Expanded: full photo, details and the update history -->
            <div v-if="expandedId === s.id" class="px-3.5 pb-3 flex flex-col gap-2">
              <div v-if="s.has_photo" class="relative w-fit">
                <img :src="api.shipments.photoUrl(s)" :alt="s.label" class="max-h-52 rounded border border-raised" />
                <button class="absolute top-1 right-1 bg-[var(--c-141414)] text-[var(--c-c0c0c0)] border border-raised rounded px-1.5 py-[0.1rem] text-[0.65rem] cursor-pointer hover:text-[var(--c-d08080)]" @click.stop="removePhoto(s)">Remove</button>
              </div>
              <p v-if="s.description" class="text-[0.78rem] text-[var(--c-b0b0b0)] leading-[1.45] whitespace-pre-wrap">{{ s.description }}</p>
              <div v-if="s.tracking_number || s.order_ref" class="text-[0.7rem] text-[var(--c-707070)] flex gap-4 flex-wrap">
                <span v-if="s.tracking_number">Tracking: <span class="font-mono text-[var(--c-909090)]">{{ s.tracking_number }}</span></span>
                <span v-if="s.order_ref">Order: <span class="font-mono text-[var(--c-909090)]">{{ s.order_ref }}</span></span>
              </div>
              <div v-if="s.events.length" class="flex flex-col gap-1 mt-1">
                <div v-for="e in s.events" :key="e.id" class="flex gap-2 text-[0.72rem]">
                  <span class="text-[var(--c-454545)] shrink-0 w-[6.5rem]">{{ fmtEventTime(e.occurred_at) }}</span>
                  <span class="text-[var(--c-9a9a9a)] break-words">{{ e.detail }}</span>
                </div>
              </div>
              <p v-else class="text-[0.72rem] text-[var(--c-454545)]">No updates yet.</p>
            </div>
          </template>
        </div>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.25rem] border-t border-surface text-[var(--c-505050)] text-[0.68rem] shrink-0">
      {{ filtered.length }} / {{ items.length }} shipments
    </div>
  </div>
</template>
