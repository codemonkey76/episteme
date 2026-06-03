<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'

const logs = useLogsStore()

const connected = ref(false)
const checking = ref(true)
const events = ref<api.CalendarEvent[]>([])
const loading = ref(false)
const error = ref('')

onMounted(async () => {
  try {
    const cfg = await api.integrations.email.getConfig()
    connected.value = cfg.connected
  } finally {
    checking.value = false
  }
  if (connected.value) await load()
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    // Now .. +30 days.
    const start = new Date().toISOString()
    const end = new Date(Date.now() + 30 * 86400000).toISOString()
    const res = await api.calendar.list({ start, end })
    events.value = res.events
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load events'
  } finally {
    loading.value = false
  }
}

// Group sorted events by local calendar day.
const grouped = computed(() => {
  const groups: { key: string; label: string; items: api.CalendarEvent[] }[] = []
  const byKey = new Map<string, { key: string; label: string; items: api.CalendarEvent[] }>()
  for (const ev of [...events.value].sort((a, b) => a.start.localeCompare(b.start))) {
    const d = new Date(ev.start)
    const key = d.toDateString()
    let g = byKey.get(key)
    if (!g) {
      g = { key, label: dayLabel(d), items: [] }
      byKey.set(key, g)
      groups.push(g)
    }
    g.items.push(ev)
  }
  return groups
})

function dayLabel(d: Date): string {
  const today = new Date(); today.setHours(0, 0, 0, 0)
  const target = new Date(d); target.setHours(0, 0, 0, 0)
  const diff = Math.round((target.getTime() - today.getTime()) / 86400000)
  const base = d.toLocaleDateString([], { weekday: 'long', month: 'short', day: 'numeric' })
  if (diff === 0) return `Today · ${base}`
  if (diff === 1) return `Tomorrow · ${base}`
  return base
}

function timeLabel(ev: api.CalendarEvent): string {
  if (ev.is_all_day) return 'All day'
  const s = new Date(ev.start).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  const e = new Date(ev.end).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  return `${s} – ${e}`
}

async function remove(ev: api.CalendarEvent) {
  try {
    await api.calendar.remove(ev.id)
    events.value = events.value.filter(x => x.id !== ev.id)
    logs.info('Calendar', `Deleted event "${ev.subject}"`)
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to delete event'
  }
}

// ── New event form ───────────────────────────────────────────────────────────
const adding = ref(false)
const todayStr = new Date().toISOString().slice(0, 10)
const form = ref({
  subject: '', date: todayStr, start: '09:00', end: '10:00',
  allDay: false, location: '', reminder: 15,
})
const saving = ref(false)

function startAdd() {
  adding.value = true
  form.value = { subject: '', date: todayStr, start: '09:00', end: '10:00', allDay: false, location: '', reminder: 15 }
}

function nextDay(dateStr: string): string {
  const d = new Date(`${dateStr}T00:00:00Z`)
  d.setUTCDate(d.getUTCDate() + 1)
  return d.toISOString().slice(0, 10)
}

async function saveEvent() {
  if (!form.value.subject.trim()) return
  saving.value = true
  error.value = ''
  try {
    const f = form.value
    // All-day: anchor to UTC midnight of the chosen date, end at next day's midnight.
    // Timed: convert local date+time to a UTC instant.
    const times = f.allDay
      ? { start: `${f.date}T00:00:00Z`, end: `${nextDay(f.date)}T00:00:00Z` }
      : {
          start: new Date(`${f.date}T${f.start}`).toISOString(),
          end: new Date(`${f.date}T${f.end}`).toISOString(),
        }
    const payload: api.NewCalendarEvent = {
      subject: f.subject.trim(),
      is_all_day: f.allDay,
      location: f.location || undefined,
      reminder_minutes_before: f.reminder >= 0 ? f.reminder : undefined,
      ...times,
    }
    await api.calendar.create(payload)
    logs.info('Calendar', `Created event "${payload.subject}"`)
    adding.value = false
    await load()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to create event'
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <!-- Checking / not connected -->
  <div v-if="checking" class="flex items-center justify-center h-full text-[#484848]">
    <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[#505050] rounded-full animate-[spin_0.7s_linear_infinite]" />
  </div>
  <div v-else-if="!connected" class="flex flex-col items-center justify-center h-full gap-3 text-[#484848]">
    <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="opacity-35">
      <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>
    </svg>
    <p class="text-[0.9375rem] font-semibold text-[#585858]">No calendar connected</p>
    <p class="text-[0.8125rem] text-center max-w-[24rem] leading-normal">Connect your Microsoft 365 account in Settings → Integrations (re-connect if you set it up before calendar support).</p>
  </div>

  <!-- Agenda -->
  <div v-else class="flex flex-col h-full bg-bg overflow-hidden">
    <div class="flex items-center gap-2 px-4 py-2.5 border-b border-[#1e1e1e] shrink-0">
      <span class="text-[0.875rem] font-semibold text-[#c0c0c0]">Upcoming</span>
      <div class="ml-auto flex items-center gap-1.5">
        <button class="flex items-center gap-[0.35rem] bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[#254880]" @click="startAdd">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          New event
        </button>
        <button class="flex items-center justify-center bg-surface text-[#808080] border border-raised rounded px-2 py-1 cursor-pointer transition-colors duration-100 hover:bg-[#222] hover:text-[#c0c0c0] disabled:opacity-50" title="Refresh" :disabled="loading" @click="load">
          <svg :class="loading ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        </button>
      </div>
    </div>

    <!-- New event form -->
    <div v-if="adding" class="flex flex-col gap-2 px-4 py-3 border-b border-[#1e1e1e] bg-[#111] shrink-0">
      <input v-model="form.subject" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[#3a6adf] placeholder:text-[#404040]" placeholder="Event title" />
      <div class="flex flex-wrap items-center gap-2 text-[0.775rem] text-muted">
        <input v-model="form.date" type="date" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" />
        <template v-if="!form.allDay">
          <input v-model="form.start" type="time" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" />
          <span class="text-[#585858]">–</span>
          <input v-model="form.end" type="time" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" />
        </template>
        <label class="flex items-center gap-1.5 cursor-pointer"><input v-model="form.allDay" type="checkbox" class="accent-[#3a6adf]" /> All day</label>
        <label class="flex items-center gap-1.5">Remind <input v-model.number="form.reminder" type="number" min="-1" class="w-[4rem] bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" /> min before</label>
      </div>
      <input v-model="form.location" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[#3a6adf] placeholder:text-[#404040]" placeholder="Location (optional)" />
      <div class="flex items-center gap-2">
        <button class="bg-[#1e3a6e] text-[#7ab0ff] border border-[#2a4a8a] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:not-disabled:bg-[#254880] disabled:opacity-50" :disabled="saving || !form.subject.trim()" @click="saveEvent">{{ saving ? 'Saving…' : 'Create' }}</button>
        <button class="bg-transparent text-[#585858] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="adding = false">Cancel</button>
      </div>
    </div>

    <div v-if="error" class="px-4 py-2 text-danger text-[0.775rem] border-b border-[#1e1e1e] shrink-0">{{ error }}</div>

    <!-- Events -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading && events.length === 0" class="flex items-center justify-center h-full text-[#484848]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[#505050] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="grouped.length === 0" class="flex items-center justify-center h-full text-[#383838] text-[0.8125rem]">No upcoming events.</div>
      <div v-else class="py-2">
        <div v-for="g in grouped" :key="g.key" class="mb-1">
          <div class="px-4 py-1.5 text-[0.72rem] font-semibold uppercase tracking-[0.05em] text-[#6a6a6a] sticky top-0 bg-bg">{{ g.label }}</div>
          <div
            v-for="ev in g.items"
            :key="ev.id"
            class="group flex items-start gap-3 px-4 py-2 border-b border-[#161616] hover:bg-[#131313]"
          >
            <span class="text-[0.72rem] text-[#7ab0ff] min-w-[5.5rem] shrink-0 pt-[0.1rem] tabular-nums">{{ timeLabel(ev) }}</span>
            <div class="flex-1 min-w-0">
              <p class="text-[0.8125rem] text-[#d0d0d0] break-words">{{ ev.subject }}</p>
              <p v-if="ev.location" class="text-[0.72rem] text-[#585858] mt-[0.1rem]">📍 {{ ev.location }}</p>
            </div>
            <button class="text-[#606060] hover:text-[#d08080] p-1 cursor-pointer bg-none border-none opacity-0 group-hover:opacity-100 transition-opacity duration-100" title="Delete" @click="remove(ev)">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
