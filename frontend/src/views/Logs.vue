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

function fmt(d: Date) {
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
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
  <div class="logs-panel">
    <!-- Toolbar -->
    <div class="toolbar">
      <select v-model="categoryFilter" class="filter-select">
        <option v-for="c in categories" :key="c" :value="c">{{ c }}</option>
      </select>

      <select v-model="levelFilter" class="filter-select level-select">
        <option value="all">All levels</option>
        <option value="debug">Debug</option>
        <option value="info">Info</option>
        <option value="warn">Warn</option>
        <option value="error">Error</option>
      </select>

      <div class="search-wrap">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input v-model="searchQuery" class="search-input" placeholder="Filter messages…" />
        <button v-if="searchQuery" class="clear-btn" @click="searchQuery = ''">✕</button>
      </div>

      <div class="toolbar-right">
        <label class="autoscroll-label" :title="autoScroll ? 'Auto-scroll on' : 'Auto-scroll off'">
          <input type="checkbox" v-model="autoScroll" />
          <span>Auto-scroll</span>
        </label>
        <button class="clear-all-btn" @click="store.clear()">Clear</button>
      </div>
    </div>

    <!-- Log list -->
    <div class="log-list" ref="listEl">
      <div v-if="filtered.length === 0" class="empty">No log entries.</div>
      <div
        v-for="e in filtered"
        :key="e.id"
        class="log-row"
        :style="{ background: LEVEL_BG[e.level] }"
      >
        <span class="ts">{{ fmt(e.ts) }}</span>
        <span class="cat" :style="{ color: catColor(e.category) }">{{ e.category }}</span>
        <span class="lvl" :style="{ color: LEVEL_COLOR[e.level] }">{{ e.level.toUpperCase() }}</span>
        <span class="msg">{{ e.message }}</span>
      </div>
    </div>

    <!-- Status bar -->
    <div class="status-bar">
      {{ filtered.length }} / {{ store.entries.length }} entries
    </div>
  </div>
</template>

<style scoped>
.logs-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #0d0d0f;
  font-family: ui-monospace, 'Cascadia Code', monospace;
  font-size: 0.775rem;
  overflow: hidden;
}

/* Toolbar */
.toolbar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid #1e1e1e;
  flex-shrink: 0;
  flex-wrap: wrap;
}

.filter-select {
  background: #1a1a1a;
  color: #c0c0c0;
  border: 1px solid #2a2a2a;
  border-radius: 0.25rem;
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
  font-family: inherit;
  cursor: pointer;
}
.level-select { min-width: 7rem; }

.search-wrap {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  background: #1a1a1a;
  border: 1px solid #2a2a2a;
  border-radius: 0.25rem;
  padding: 0.2rem 0.5rem;
  flex: 1;
  min-width: 10rem;
  color: #484848;
}
.search-input {
  flex: 1;
  background: none;
  border: none;
  color: #c0c0c0;
  font-size: 0.75rem;
  font-family: inherit;
  outline: none;
  min-width: 0;
}
.search-input::placeholder { color: #404040; }
.clear-btn {
  background: none;
  border: none;
  color: #484848;
  cursor: pointer;
  font-size: 0.65rem;
  padding: 0;
  transition: color 0.1s;
}
.clear-btn:hover { color: #909090; }

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  margin-left: auto;
}

.autoscroll-label {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  color: #606060;
  font-size: 0.72rem;
  cursor: pointer;
  font-family: system-ui, sans-serif;
}
.autoscroll-label input { cursor: pointer; }

.clear-all-btn {
  background: #1e1e1e;
  color: #808080;
  border: 1px solid #2a2a2a;
  border-radius: 0.25rem;
  padding: 0.2rem 0.5rem;
  font-size: 0.72rem;
  font-family: system-ui, sans-serif;
  cursor: pointer;
  transition: background 0.1s, color 0.1s;
}
.clear-all-btn:hover { background: #282828; color: #c0c0c0; }

/* Log list */
.log-list {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
}

.empty {
  padding: 1.5rem;
  color: #383838;
  text-align: center;
  font-family: system-ui, sans-serif;
}

.log-row {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
  padding: 0.15rem 0.75rem;
  border-bottom: 1px solid #111;
  line-height: 1.5;
  min-height: 1.5rem;
}
.log-row:hover { background: #141414 !important; }

.ts {
  color: #404050;
  flex-shrink: 0;
  font-size: 0.72rem;
  min-width: 6rem;
}
.cat {
  font-weight: 600;
  flex-shrink: 0;
  min-width: 6rem;
  font-size: 0.72rem;
}
.lvl {
  font-weight: 700;
  flex-shrink: 0;
  min-width: 3.5rem;
  font-size: 0.68rem;
  letter-spacing: 0.04em;
}
.msg {
  color: #c0c0c0;
  word-break: break-word;
  flex: 1;
  min-width: 0;
}

/* Status bar */
.status-bar {
  padding: 0.2rem 0.75rem;
  border-top: 1px solid #1a1a1a;
  color: #404040;
  font-size: 0.68rem;
  flex-shrink: 0;
  font-family: system-ui, sans-serif;
}
</style>
