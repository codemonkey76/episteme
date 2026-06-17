<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import * as api from '../api'
import ApprovalCard from '../components/ApprovalCard.vue'
import { useNotificationsStore } from '../stores/notifications'
import { useSessionsStore } from '../stores/sessions'
import { useWindowsStore } from '../stores/windows'

const notifStore = useNotificationsStore()
const sessionsStore = useSessionsStore()
const winStore = useWindowsStore()

const items = ref<api.Notification[]>([])
const loading = ref(false)
const error = ref('')
const busy = ref(false)

const unreadCount = computed(() => items.value.filter(n => !n.read_at).length)

async function load() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.notifications.list(200)
    items.value = res.notifications
    notifStore.unread = res.unread
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to load notifications'
  } finally {
    loading.value = false
  }
}

onMounted(load)
// Refresh when something elsewhere changes the notifications.
watch(() => notifStore.changeToken, load)

// Open the item a notification points at (currently only chat sessions), marking
// it read on the way.
async function open(n: api.Notification) {
  if (!n.read_at) await markRead(n)
  if (n.link_kind === 'session' && n.link_id) {
    try {
      await sessionsStore.loadSession(n.link_id)
      winStore.openKey('chat', undefined, 'fill')
    } catch (e: unknown) {
      error.value = e instanceof Error ? e.message : 'Could not open the linked session'
    }
  }
}

async function markRead(n: api.Notification) {
  if (n.read_at) return
  n.read_at = new Date().toISOString() // optimistic
  try {
    await api.notifications.markRead(n.id)
    notifStore.notifyChanged()
  } catch { /* best-effort; a reload will reconcile */ }
}

async function markAllRead() {
  if (busy.value) return
  busy.value = true
  try {
    await api.notifications.markAllRead()
    const now = new Date().toISOString()
    items.value.forEach(n => { if (!n.read_at) n.read_at = now })
    notifStore.notifyChanged()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to mark all read'
  } finally {
    busy.value = false
  }
}

async function remove(n: api.Notification) {
  items.value = items.value.filter(x => x.id !== n.id)
  try {
    await api.notifications.remove(n.id)
    notifStore.notifyChanged()
  } catch { /* best-effort */ }
}

async function clearAll() {
  if (busy.value || items.value.length === 0) return
  if (!confirm('Clear all notifications? This can’t be undone.')) return
  busy.value = true
  try {
    await api.notifications.clearAll()
    items.value = []
    notifStore.notifyChanged()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Failed to clear notifications'
  } finally {
    busy.value = false
  }
}

// ── Ticket-update actions ───────────────────────────────────────────────────
// Which notification's reply composer is expanded, and which have been replied.
const openReplyId = ref<string | null>(null)
const repliedIds = ref<Set<string>>(new Set())
const replyError = ref<Record<string, string>>({})

// Parse a ticket_update notification's `data` payload; null if absent/malformed.
function ticketData(n: api.Notification): api.TicketUpdateData | null {
  if (n.category !== 'ticket_update' || !n.data) return null
  try {
    return JSON.parse(n.data) as api.TicketUpdateData
  } catch {
    return null
  }
}

// JSON args seeding the ApprovalCard's helpdesk_reply_ticket editor.
function replyArgs(d: api.TicketUpdateData): string {
  return JSON.stringify({ ticket_id: d.ticket_id, type: 'reply', body: d.draft_reply })
}

async function sendReply(n: api.Notification, edited?: Record<string, unknown>) {
  const d = ticketData(n)
  if (!d) return
  const body = String(edited?.body ?? d.draft_reply)
  const type = (edited?.type as 'reply' | 'internal_note') ?? 'reply'
  delete replyError.value[n.id]
  try {
    await api.helpdesk.replyTicket(d.ticket_id, { body, type, integration: d.integration })
    repliedIds.value.add(n.id)
    openReplyId.value = null
    await markRead(n)
  } catch (e: unknown) {
    replyError.value[n.id] = e instanceof Error ? e.message : 'Failed to send reply'
  }
}

// ── Presentation ──────────────────────────────────────────────────────────────
interface Style { color: string; bg: string; path: string }
const STYLES: Record<string, Style> = {
  agent:      { color: 'var(--c-7ab0ff)', bg: 'var(--c-15233f)', path: 'clock' },
  approval:   { color: 'var(--c-e0b060)', bg: 'var(--c-2a2418)', path: 'alert' },
  error:      { color: 'var(--c-d08080)', bg: 'var(--c-2a1818)', path: 'alert' },
  suggestion: { color: 'var(--c-7ad08a)', bg: 'var(--c-15291b)', path: 'check' },
  email:      { color: 'var(--c-7ab0ff)', bg: 'var(--c-15233f)', path: 'mail' },
  ticket_update: { color: 'var(--c-7ad08a)', bg: 'var(--c-15291b)', path: 'ticket' },
  info:       { color: 'var(--c-9a9a9a)', bg: 'var(--c-1c1c1c)', path: 'bell' },
}
function styleFor(cat: string): Style {
  return STYLES[cat] ?? STYLES.info
}

function fmtTime(iso: string): string {
  const d = new Date(iso)
  const diff = (Date.now() - d.getTime()) / 1000
  if (diff < 60) return 'just now'
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' })
}
</script>

<template>
  <div class="flex flex-col h-full bg-bg overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-1e1e1e)] shrink-0">
      <span class="text-[0.8125rem] font-semibold text-[var(--c-c0c0c0)]">Notifications</span>
      <span v-if="unreadCount" class="text-[0.65rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] rounded-full px-1.5 py-[0.05rem] font-medium">{{ unreadCount }} new</span>
      <div class="ml-auto flex items-center gap-1.5">
        <button class="text-[0.7rem] text-[var(--c-808080)] hover:text-[var(--c-c0c0c0)] bg-surface border border-raised rounded px-2 py-1 cursor-pointer disabled:opacity-40" :disabled="busy || unreadCount === 0" @click="markAllRead">Mark all read</button>
        <button class="text-[0.7rem] text-[var(--c-808080)] hover:text-[var(--c-d08080)] bg-surface border border-raised rounded px-2 py-1 cursor-pointer disabled:opacity-40" :disabled="busy || items.length === 0" @click="clearAll">Clear all</button>
        <button class="flex items-center justify-center bg-surface text-[var(--c-808080)] border border-raised rounded px-1.5 py-1 cursor-pointer hover:bg-[var(--c-222222)] hover:text-[var(--c-c0c0c0)] disabled:opacity-50" title="Refresh" :disabled="loading" @click="load">
          <svg :class="loading ? 'animate-[spin_0.7s_linear_infinite]' : ''" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
        </button>
      </div>
    </div>

    <div v-if="error" class="px-3 py-1.5 text-danger text-[0.75rem] border-b border-[var(--c-1e1e1e)] shrink-0">{{ error }}</div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto min-h-0">
      <div v-if="loading && items.length === 0" class="flex items-center justify-center h-full text-[var(--c-484848)]">
        <span class="inline-block w-[18px] h-[18px] border-2 border-raised border-t-[var(--c-505050)] rounded-full animate-[spin_0.7s_linear_infinite]" />
      </div>
      <div v-else-if="items.length === 0" class="flex flex-col items-center justify-center h-full gap-2 text-[var(--c-383838)]">
        <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>
        <p class="text-[0.8125rem]">No notifications.</p>
      </div>
      <div v-else>
        <div
          v-for="n in items"
          :key="n.id"
          :class="['group flex items-start gap-3 px-3.5 py-2.5 border-b border-[var(--c-161616)] transition-colors duration-100', n.link_kind ? 'cursor-pointer' : '', !n.read_at ? 'bg-[var(--c-101620)] hover:bg-[var(--c-121a26)]' : 'hover:bg-[var(--c-131313)]']"
          @click="open(n)"
        >
          <!-- Unread dot + category icon -->
          <div class="relative shrink-0 mt-[0.1rem]">
            <span v-if="!n.read_at" class="absolute -left-2 top-1/2 -translate-y-1/2 w-[5px] h-[5px] rounded-full bg-[var(--c-7ab0ff)]" />
            <span class="flex items-center justify-center w-6 h-6 rounded-full" :style="{ background: styleFor(n.category).bg, color: styleFor(n.category).color }">
              <svg v-if="styleFor(n.category).path === 'alert'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
              <svg v-else-if="styleFor(n.category).path === 'check'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              <svg v-else-if="styleFor(n.category).path === 'mail'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="16" rx="2"/><path d="m22 7-10 5L2 7"/></svg>
              <svg v-else-if="styleFor(n.category).path === 'ticket'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 9V7a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v2a2 2 0 0 0 0 4v2a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-2a2 2 0 0 0 0-4z"/><line x1="13" y1="5" x2="13" y2="19"/></svg>
              <svg v-else-if="n.category === 'agent'" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><polyline points="12 7 12 12 15 14"/></svg>
              <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>
            </span>
          </div>

          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <p class="flex-1 min-w-0 truncate text-[0.8125rem] font-medium" :class="!n.read_at ? 'text-[var(--c-e0e8f4)]' : 'text-[var(--c-b0b0b0)]'">{{ n.title }}</p>
              <span class="shrink-0 text-[0.65rem] text-[var(--c-505050)]">{{ fmtTime(n.created_at) }}</span>
            </div>
            <p class="text-[0.75rem] text-[var(--c-808080)] leading-[1.45] break-words mt-[0.1rem] line-clamp-3">{{ n.body }}</p>
            <span v-if="n.link_kind === 'session'" class="inline-flex items-center gap-1 text-[0.68rem] text-[var(--c-5a7da0)] mt-[0.2rem]">
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M13 6l6 6-6 6"/></svg>
              Open chat
            </span>

            <!-- Ticket-update actions: review & send a customer reply on the ticket. -->
            <div v-if="ticketData(n)" class="mt-1.5" @click.stop>
              <span v-if="repliedIds.has(n.id)" class="inline-flex items-center gap-1 text-[0.7rem] text-[var(--c-7ad08a)]">
                <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                Reply sent to customer
              </span>
              <button
                v-else-if="openReplyId !== n.id"
                class="text-[0.7rem] text-[var(--c-7ad08a)] bg-[var(--c-15291b)] border border-[var(--c-2a5a3a)] rounded px-2 py-1 cursor-pointer hover:bg-[var(--c-1b3324)]"
                @click="openReplyId = n.id"
              >
                Review &amp; send reply
              </button>
              <ApprovalCard
                v-else
                :tool-name="'helpdesk_reply_ticket'"
                :tool-args="replyArgs(ticketData(n)!)"
                label="Reply to customer"
                @approve="(args) => sendReply(n, args)"
                @reject="openReplyId = null"
              />
              <p v-if="replyError[n.id]" class="text-[0.7rem] text-danger mt-1">{{ replyError[n.id] }}</p>
            </div>
          </div>

          <button class="shrink-0 text-[var(--c-505050)] hover:text-[var(--c-d08080)] p-1 bg-none border-none cursor-pointer opacity-0 group-hover:opacity-100 transition-opacity duration-100" title="Dismiss" @click.stop="remove(n)">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
