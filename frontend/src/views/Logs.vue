<script setup lang="ts">
import { ref, computed, watch, nextTick } from 'vue'
import { useLogsStore, type LogLevel } from '../stores/logs'

const store = useLogsStore()

const categoryFilter = ref('All')
const levelFilter = ref<'all' | LogLevel>('all')
const searchQuery = ref('')
const autoScroll = ref(true)
const listEl = ref<HTMLElement>()

const categories = computed(() => {
  const cats = new Set(store.entries.map(e => e.category))
  return ['All', ...Array.from(cats).sort()]
})

const filtered = computed(() => {
  const cat = categoryFilter.value
  const lvl = levelFilter.value
  const q = searchQuery.value.trim().toLowerCase()
  return store.entries.filter(e => {
    if (cat !== 'All' && e.category !== cat) return false
    if (lvl !== 'all' && e.level !== lvl) return false
    if (q && !e.message.toLowerCase().includes(q) && !e.category.toLowerCase().includes(q)) return false
    return true
  })
})

watch(() => store.entries.length, async () => {
  if (!autoScroll.value) return
  await nextTick()
  if (listEl.value) listEl.value.scrollTop = listEl.value.scrollHeight
})

function fmt(ts: number) {
  return new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

const LEVEL_COLOR: Record<LogLevel, string> = {
  debug: '#525260',
  info:  '#4a90d0',
  warn:  '#b8902a',
  error: '#c04040',
}
const LEVEL_BG: Record<LogLevel, string> = {
  debug: 'transparent',
  info:  'transparent',
  warn:  'rgba(184,144,42,0.08)',
  error: 'rgba(192,64,64,0.1)',
}

const CAT_PALETTE = ['#7ab0ff','#7adfbb','#c07aff','#ffb07a','#ff7a9a','#7affda','#d0ff7a','#ffda7a','#ff9f7a','#7abfff']
const _catCache: Record<string, string> = {}
function catColor(cat: string): string {
  if (!_catCache[cat]) {
    let h = 0
    for (const c of cat) h = Math.imul(h * 31, c.charCodeAt(0)) >>> 0
    _catCache[cat] = CAT_PALETTE[h % CAT_PALETTE.length]
  }
  return _catCache[cat]
}
</script>

<template>
  <div class="flex flex-col h-full bg-[#0d0d0f] overflow-hidden font-mono text-[0.775rem]" style="font-family: ui-monospace, 'Cascadia Code', monospace;">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[#1e1e1e] shrink-0 flex-wrap">
      <select v-model="categoryFilter" class="bg-surface text-[#c0c0c0] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer">
        <option v-for="c in categories" :key="c" :value="c">{{ c }}</option>
      </select>

      <select v-model="levelFilter" class="bg-surface text-[#c0c0c0] border border-raised rounded px-2 py-1 text-xs font-[inherit] cursor-pointer min-w-[7rem]">
        <option value="all">All levels</option>
        <option value="debug">Debug</option>
        <option value="info">Info</option>
        <option value="warn">Warn</option>
        <option value="error">Error</option>
      </select>

      <div class="flex items-center gap-[0.3rem] bg-surface border border-raised rounded px-2 py-[0.2rem] flex-1 min-w-[10rem] text-[#484848]">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="flex-1 bg-none border-none text-[#c0c0c0] text-xs font-[inherit] outline-none min-w-0 placeholder:text-[#404040]" placeholder="Filter messages…" />
        <button v-if="searchQuery" class="bg-none border-none text-[#484848] cursor-pointer text-[0.65rem] p-0 transition-colors duration-100 hover:text-muted" @click="searchQuery = ''">✕</button>
      </div>

      <div class="flex items-center gap-[0.625rem] ml-auto">
        <label class="flex items-center gap-[0.3rem] text-[#606060] text-[0.72rem] cursor-pointer font-sans" :title="autoScroll ? 'Auto-scroll on' : 'Auto-scroll off'">
          <input type="checkbox" v-model="autoScroll" class="cursor-pointer" />
          <span>Auto-scroll</span>
        </label>
        <button class="bg-[#1e1e1e] text-[#808080] border border-raised rounded px-2 py-[0.2rem] text-[0.72rem] font-sans cursor-pointer transition-colors duration-100 hover:bg-[#282828] hover:text-[#c0c0c0]" @click="store.clear()">Clear</button>
      </div>
    </div>

    <!-- Log list -->
    <div class="flex-1 overflow-y-auto overflow-x-hidden" ref="listEl">
      <div v-if="filtered.length === 0" class="p-6 text-[#383838] text-center font-sans">No log entries.</div>
      <div
        v-for="e in filtered"
        :key="e.id"
        class="group flex items-baseline gap-2 px-3 py-[0.15rem] border-b border-[#111] leading-normal min-h-[1.5rem] hover:!bg-[#141414]"
        :style="{ background: LEVEL_BG[e.level] }"
      >
        <span class="text-[#404050] shrink-0 text-[0.72rem] min-w-[6rem]">{{ fmt(e.ts) }}</span>
        <span class="font-semibold shrink-0 min-w-[6rem] text-[0.72rem]" :style="{ color: catColor(e.category) }">{{ e.category }}</span>
        <span class="font-bold shrink-0 min-w-[3.5rem] text-[0.68rem] tracking-[0.04em]" :style="{ color: LEVEL_COLOR[e.level] }">{{ e.level.toUpperCase() }}</span>
        <span class="text-[#c0c0c0] break-words flex-1 min-w-0">{{ e.message }}</span>
      </div>
    </div>

    <!-- Status bar -->
    <div class="px-3 py-[0.2rem] border-t border-surface text-[#404040] text-[0.68rem] shrink-0 font-sans flex items-center">
      <span
        class="inline-block w-[6px] h-[6px] rounded-full mr-[0.4rem] shrink-0"
        :class="store.connected ? 'bg-success' : 'bg-[#505050]'"
        :title="store.connected ? 'Live' : 'Disconnected'"
      />
      {{ filtered.length }} / {{ store.entries.length }} entries
    </div>
  </div>
</template>
