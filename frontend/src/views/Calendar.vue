<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import * as api from '../api'
import { useLogsStore } from '../stores/logs'
import { useCalendarStore } from '../stores/calendar'

const logs = useLogsStore()
const calStore = useCalendarStore()

const connected = ref(false)
const checking = ref(true)
const events = ref<api.CalendarEvent[]>([])
const loading = ref(false)
const error = ref('')

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

// ── Date helpers ─────────────────────────────────────────────────────────────
function startOfMonth(d: Date) { return new Date(d.getFullYear(), d.getMonth(), 1) }
function stripTime(d: Date) { return new Date(d.getFullYear(), d.getMonth(), d.getDate()) }
function ymd(d: Date) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

const viewDate = ref(startOfMonth(new Date())) // first day of the viewed month
const selectedDay = ref(stripTime(new Date()))

const monthLabel = computed(() => viewDate.value.toLocaleDateString([], { month: 'long', year: 'numeric' }))

// 6-week grid starting on the Sunday on/before the 1st.
const gridStart = computed(() => {
  const s = startOfMonth(viewDate.value)
  const g = new Date(s)
  g.setDate(1 - s.getDay())
  return stripTime(g)
})
const gridDays = computed(() => {
  const days: Date[] = []
  for (let i = 0; i < 42; i++) {
    const d = new Date(gridStart.value)
    d.setDate(gridStart.value.getDate() + i)
    days.push(d)
  }
  return days
})

// Group events by the day they fall on (all-day anchored to its UTC date).
function eventDayKey(ev: api.CalendarEvent): string {
  return ev.is_all_day ? ev.start.slice(0, 10) : ymd(new Date(ev.start))
}
const eventsByDay = computed(() => {
  const map = new Map<string, api.CalendarEvent[]>()
  for (const ev of events.value) {
    const k = eventDayKey(ev)
    const arr = map.get(k)
    if (arr) arr.push(ev)
    else map.set(k, [ev])
  }
  for (const arr of map.values()) arr.sort((a, b) => a.start.localeCompare(b.start))
  return map
})
function dayEvents(d: Date): api.CalendarEvent[] {
  return eventsByDay.value.get(ymd(d)) ?? []
}
const selectedDayEvents = computed(() => dayEvents(selectedDay.value))

function isToday(d: Date) { return ymd(d) === ymd(new Date()) }
function inMonth(d: Date) { return d.getMonth() === viewDate.value.getMonth() }
function isSelected(d: Date) { return ymd(d) === ymd(selectedDay.value) }

function startTime(ev: api.CalendarEvent) {
  return ev.is_all_day ? 'All day' : new Date(ev.start).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}
function timeRange(ev: api.CalendarEvent) {
  if (ev.is_all_day) return 'All day'
  const s = new Date(ev.start).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  const e = new Date(ev.end).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  return `${s} – ${e}`
}
const selectedDayLabel = computed(() =>
  selectedDay.value.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' }),
)

// ── Data ─────────────────────────────────────────────────────────────────────
async function load() {
  if (!connected.value) return
  loading.value = true
  error.value = ''
  try {
    const start = new Date(gridStart.value).toISOString()
    const endD = new Date(gridStart.value)
    endD.setDate(endD.getDate() + 42)
    const res = await api.calendar.list({ start, end: endD.toISOString() })
    events.value = res.events
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load events'
  } finally {
    loading.value = false
  }
}

let pollTimer: number | undefined
onMounted(async () => {
  try {
    // Calendar uses the default Microsoft 365 account; show it if any is connected.
    const { integrations } = await api.integrations.list()
    connected.value = integrations.some(i => i.kind === 'microsoft' && i.connected)
  } finally {
    checking.value = false
  }
  if (connected.value) await load()
  // Gentle poll catches changes made elsewhere (e.g. Outlook) without a manual refresh.
  pollTimer = window.setInterval(() => { if (connected.value && !adding.value) load() }, 45000)
})
onUnmounted(() => { if (pollTimer) clearInterval(pollTimer) })

watch(viewDate, load)
// Live refresh when the chat AI creates/deletes an event.
watch(() => calStore.changeToken, () => { if (connected.value) load() })

function prevMonth() { const d = new Date(viewDate.value); d.setMonth(d.getMonth() - 1); viewDate.value = startOfMonth(d) }
function nextMonth() { const d = new Date(viewDate.value); d.setMonth(d.getMonth() + 1); viewDate.value = startOfMonth(d) }
function goToday() { viewDate.value = startOfMonth(new Date()); selectedDay.value = stripTime(new Date()) }
function selectDay(d: Date) { selectedDay.value = stripTime(d); if (!inMonth(d)) viewDate.value = startOfMonth(d); adding.value = false }

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
const form = ref({ subject: '', date: ymd(new Date()), start: '09:00', end: '10:00', allDay: false, location: '', reminder: 15 })
const saving = ref(false)

function startAdd() {
  adding.value = true
  form.value = { subject: '', date: ymd(selectedDay.value), start: '09:00', end: '10:00', allDay: false, location: '', reminder: 15 }
}

function nextDayStr(dateStr: string): string {
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
    const times = f.allDay
      ? { start: `${f.date}T00:00:00Z`, end: `${nextDayStr(f.date)}T00:00:00Z` }
      : { start: new Date(`${f.date}T${f.start}`).toISOString(), end: new Date(`${f.date}T${f.end}`).toISOString() }
    await api.calendar.create({
      subject: f.subject.trim(),
      is_all_day: f.allDay,
      location: f.location || undefined,
      reminder_minutes_before: f.reminder >= 0 ? f.reminder : undefined,
      ...times,
    })
    logs.info('Calendar', `Created event "${f.subject}"`)
    adding.value = false
    selectedDay.value = stripTime(new Date(`${f.date}T00:00:00`))
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
  <div v-if="checking" class="flex items-center justify-center h-full text-[var(--c-484848)]">
    <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
  </div>
  <div v-else-if="!connected" class="flex flex-col items-center justify-center h-full gap-3 text-[var(--c-484848)]">
    <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="opacity-35">
      <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>
    </svg>
    <p class="text-[0.9375rem] font-semibold text-[var(--c-585858)]">No calendar connected</p>
    <p class="text-[0.8125rem] text-center max-w-[24rem] leading-normal">Connect your Microsoft 365 account in Settings → Integrations (re-connect if you set it up before calendar support).</p>
  </div>

  <!-- Month grid -->
  <div v-else class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0">
      <button class="w-6 h-6 flex items-center justify-center rounded text-[var(--c-808080)] hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" title="Previous month" @click="prevMonth">‹</button>
      <span class="text-[0.875rem] font-semibold text-[var(--c-d0d0d0)] min-w-[9rem] text-center tabular-nums">{{ monthLabel }}</span>
      <button class="w-6 h-6 flex items-center justify-center rounded text-[var(--c-808080)] hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" title="Next month" @click="nextMonth">›</button>
      <button class="ml-1 text-[0.75rem] text-[var(--c-808080)] border border-raised rounded px-2 py-1 hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)]" @click="goToday">Today</button>
      <div class="ml-auto flex items-center gap-1.5">
        <button class="flex items-center gap-[0.35rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-2.5 py-1 text-xs font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254880)]" @click="startAdd">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          New event
        </button>
        <button class="flex items-center justify-center bg-surface text-[var(--c-808080)] border border-raised rounded px-2 py-1 cursor-pointer transition-colors duration-100 hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)] disabled:opacity-50" title="Refresh" :disabled="loading" @click="load">
          <svg :class="loading ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        </button>
      </div>
    </div>

    <!-- Weekday header -->
    <div class="grid grid-cols-7 shrink-0 border-b border-[var(--c-1e1e1e)]">
      <div v-for="w in WEEKDAYS" :key="w" class="px-2 py-1 text-[0.65rem] font-semibold uppercase tracking-[0.05em] text-[var(--c-585858)] text-center">{{ w }}</div>
    </div>

    <!-- Grid -->
    <div class="flex-1 min-h-0 grid grid-cols-7 grid-rows-6 gap-px bg-[var(--c-1a1a1a)]">
      <button
        v-for="d in gridDays"
        :key="d.toISOString()"
        :class="['flex flex-col items-stretch text-left p-1 overflow-hidden transition-colors duration-100',
                 inMonth(d) ? 'bg-bg' : 'bg-[var(--c-101010)]',
                 isSelected(d) ? 'ring-1 ring-inset ring-[var(--c-3a6adf)]' : '']"
        @click="selectDay(d)"
      >
        <span :class="['self-end text-[0.7rem] leading-none mb-1 w-[1.3rem] h-[1.3rem] flex items-center justify-center rounded-full',
                       isToday(d) ? 'bg-[var(--c-3a6adf)] text-white font-semibold' : inMonth(d) ? 'text-[var(--c-b0b0b0)]' : 'text-[var(--c-454545)]']">{{ d.getDate() }}</span>
        <div class="flex flex-col gap-[0.1rem] overflow-hidden">
          <span
            v-for="ev in dayEvents(d).slice(0, 2)"
            :key="ev.id"
            class="text-[0.62rem] leading-[1.2] line-clamp-2 break-words rounded px-1 py-[0.1rem] bg-[var(--c-1c2a3a)] text-[var(--c-9cc0f0)]"
            :title="`${timeRange(ev)} — ${ev.subject}`"
          >
            <span v-if="!ev.is_all_day" class="text-[var(--c-6a8ec0)]">{{ startTime(ev) }} </span>{{ ev.subject }}
          </span>
          <span v-if="dayEvents(d).length > 2" class="text-[0.6rem] text-[var(--c-585858)] px-1">+{{ dayEvents(d).length - 2 }} more</span>
        </div>
      </button>
    </div>

    <div v-if="error" class="px-4 py-1.5 text-danger text-[0.75rem] border-t border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

    <!-- Selected day detail -->
    <div class="shrink-0 border-t border-[var(--c-1e1e1e)] bg-[var(--c-0f0f0f)] h-[160px] flex flex-col">
      <div class="flex items-center justify-between px-4 py-2 shrink-0">
        <span class="text-[0.8rem] font-semibold text-[var(--c-c0c0c0)]">{{ selectedDayLabel }}</span>
        <button v-if="!adding" class="text-[0.72rem] text-[var(--c-7ab0ff)] hover:underline" @click="startAdd">+ Add</button>
      </div>

      <!-- Add form -->
      <div v-if="adding" class="flex flex-col gap-2 px-4 pb-3 overflow-y-auto">
        <input v-model="form.subject" class="bg-surface text-fg border border-raised rounded px-2 py-1.5 text-[0.8125rem] font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Event title" />
        <div class="flex flex-wrap items-center gap-2 text-[0.75rem] text-muted">
          <input v-model="form.date" type="date" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" />
          <template v-if="!form.allDay">
            <input v-model="form.start" type="time" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" />
            <span class="text-[var(--c-585858)]">–</span>
            <input v-model="form.end" type="time" class="bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit]" />
          </template>
          <label class="flex items-center gap-1.5 cursor-pointer"><input v-model="form.allDay" type="checkbox" class="accent-[var(--c-3a6adf)]" /> All day</label>
          <label class="flex items-center gap-1.5">Remind <input v-model.number="form.reminder" type="number" min="-1" class="w-[3.5rem] bg-surface text-fg border border-raised rounded px-1.5 py-1 text-xs font-[inherit]" /> min</label>
          <input v-model="form.location" class="flex-1 min-w-[8rem] bg-surface text-fg border border-raised rounded px-2 py-1 text-xs font-[inherit] outline-none focus:border-[var(--c-3a6adf)] placeholder:text-[var(--c-404040)]" placeholder="Location (optional)" />
        </div>
        <div class="flex items-center gap-2">
          <button class="bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-3 py-1 text-xs font-[inherit] cursor-pointer hover:not-disabled:bg-[var(--c-254880)] disabled:opacity-50" :disabled="saving || !form.subject.trim()" @click="saveEvent">{{ saving ? 'Saving…' : 'Create' }}</button>
          <button class="bg-transparent text-[var(--c-585858)] border-none px-2 py-1 text-xs font-[inherit] cursor-pointer hover:text-muted" @click="adding = false">Cancel</button>
        </div>
      </div>

      <!-- Event list for the day -->
      <div v-else class="flex-1 overflow-y-auto px-4 pb-3">
        <div v-if="selectedDayEvents.length === 0" class="text-[0.78rem] text-[var(--c-484848)] py-1">No events.</div>
        <div v-for="ev in selectedDayEvents" :key="ev.id" class="group flex items-start gap-3 py-1.5 border-b border-[var(--c-161616)] last:border-0">
          <span class="text-[0.72rem] text-[var(--c-7ab0ff)] min-w-[5.5rem] shrink-0 pt-[0.1rem] tabular-nums">{{ timeRange(ev) }}</span>
          <div class="flex-1 min-w-0">
            <p class="text-[0.8125rem] text-[var(--c-d0d0d0)] break-words">{{ ev.subject }}</p>
            <p v-if="ev.location" class="text-[0.7rem] text-[var(--c-585858)]">📍 {{ ev.location }}</p>
          </div>
          <button class="text-[var(--c-606060)] hover:text-[var(--c-d08080)] p-1 cursor-pointer bg-none border-none opacity-0 group-hover:opacity-100 transition-opacity duration-100" title="Delete" @click="remove(ev)">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
