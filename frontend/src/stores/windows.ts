import { defineStore } from 'pinia'
import { ref, markRaw, watch } from 'vue'
import type { Component } from 'vue'
import { windowRegistry } from '../windows/registry'

export type DockAnchor = 'left' | 'right' | 'top' | 'bottom' | 'fill'

export interface SnapPreview {
  x: number
  y: number
  width: number
  height: number
  anchor: DockAnchor
}

export interface WinState {
  id: string
  /** Unique per instance. For singletons this equals `regKey`; multi-instance
   *  windows (chat) get a generated key like `chat:3`. */
  key?: string
  /** Registry key used to resolve the component (e.g. 'chat'). */
  regKey?: string
  title: string
  component: Component
  props: Record<string, unknown>
  x: number
  y: number
  width: number
  height: number | null
  defaultWidth: number
  zIndex: number
  snapped: boolean
  dockAnchor: DockAnchor | null
  dockOrder: number
}

let nextZ = 10
let nextId = 0

interface Area { x: number; y: number; w: number; h: number }

function contentBounds(): Area {
  const sidebar = document.querySelector('.sidebar') as HTMLElement | null
  const sidebarW = sidebar ? sidebar.offsetWidth : 0
  return {
    x: sidebarW,
    y: 0,
    w: window.innerWidth - sidebarW,
    h: window.innerHeight,
  }
}

// Replay all docked windows in order to reposition them after any layout change.
// Rules:
//   left/right  → x, y, and height derived from remaining area; width is preserved (user-set)
//   top/bottom  → x, y, and width derived from remaining area; height is preserved (user-set)
//   fill        → all four dimensions derived from remaining area
function relayout(wins: WinState[]) {
  const cb = contentBounds()
  // Use unambiguous scalar names to avoid any closure/shadowing issues
  let rx = cb.x, ry = cb.y, rw = cb.w, rh = cb.h

  const docked = wins
    .filter(w => w.snapped && w.dockAnchor)
    .sort((wa, wb) => wa.dockOrder - wb.dockOrder)

  for (const win of docked) {
    if (rw < 1 || rh < 1) break

    if (win.dockAnchor === 'left') {
      win.x      = rx
      win.y      = ry
      win.height = rh
      rx += win.width
      rw -= win.width

    } else if (win.dockAnchor === 'right') {
      win.x      = rx + rw - win.width
      win.y      = ry
      win.height = rh
      rw -= win.width

    } else if (win.dockAnchor === 'top') {
      const h    = win.height ?? Math.round(rh / 2)
      win.x      = rx
      win.y      = ry
      win.width  = rw
      win.height = h
      ry += h
      rh -= h

    } else if (win.dockAnchor === 'bottom') {
      const h    = win.height ?? Math.round(rh / 2)
      win.x      = rx
      win.y      = ry + rh - h
      win.width  = rw
      win.height = h
      rh -= h

    } else if (win.dockAnchor === 'fill') {
      win.x      = rx
      win.y      = ry
      win.width  = rw
      win.height = rh
      rw = 0
      rh = 0
    }
  }
}

// Compute the remaining area (for snap-preview) using stored widths/heights.
function calcRemainingArea(wins: WinState[]): Area {
  const cb = contentBounds()
  let rx = cb.x, ry = cb.y, rw = cb.w, rh = cb.h

  const docked = wins
    .filter(w => w.snapped && w.dockAnchor)
    .sort((wa, wb) => wa.dockOrder - wb.dockOrder)

  for (const win of docked) {
    if (rw < 60 || rh < 60) return { x: rx, y: ry, w: 0, h: 0 }
    const wh = win.height ?? 0
    if (win.dockAnchor === 'left')   { rx += win.width; rw -= win.width }
    if (win.dockAnchor === 'right')  { rw -= win.width }
    if (win.dockAnchor === 'top')    { ry += wh; rh -= wh }
    if (win.dockAnchor === 'bottom') { rh -= wh }
    if (win.dockAnchor === 'fill')   return { x: rx, y: ry, w: 0, h: 0 }
  }
  return { x: rx, y: ry, w: rw, h: rh }
}

// Zones within remaining area:
//   corners          → fill
//   near top/bottom  → top/bottom (half height)
//   near left/right  → left/right (half width)
export function snapPreviewForMouse(
  mouseX: number,
  mouseY: number,
  otherWindows: WinState[],
): SnapPreview | null {
  const rem = calcRemainingArea(otherWindows)
  // Only offer docking when there's a genuinely large free area to dock into.
  if (rem.w < 360 || rem.h < 280) return null

  // A small, fixed activation band at the very edge — docking is opt-in, so a
  // window stays floating unless the user drags its cursor right to an edge.
  // (Previously up to 80px / 22% of the area, which docked on a casual hover.)
  const EDGE = 24

  // Clamp to the content area so slamming the cursor against a screen edge
  // still counts as that edge (the classic dock gesture)…
  const cb = contentBounds()
  const mx = Math.max(cb.x, Math.min(mouseX, cb.x + cb.w))
  const my = Math.max(cb.y, Math.min(mouseY, cb.y + cb.h))

  // …but only arm the zones when the cursor is actually at the free area
  // (within the band). Without this, the half-plane tests below were true
  // across the whole region covered by docked windows — dragging anywhere
  // over a docked window previewed a dock at the free area's far-away edge.
  if (mx < rem.x - EDGE || mx > rem.x + rem.w + EDGE) return null
  if (my < rem.y - EDGE || my > rem.y + rem.h + EDGE) return null

  const nearLeft   = mx <= rem.x + EDGE
  const nearRight  = mx >= rem.x + rem.w - EDGE
  const nearTop    = my <= rem.y + EDGE
  const nearBottom = my >= rem.y + rem.h - EDGE

  if ((nearLeft || nearRight) && (nearTop || nearBottom)) {
    return { x: rem.x, y: rem.y, width: rem.w, height: rem.h, anchor: 'fill' }
  }
  if (nearLeft) {
    const w = Math.max(240, Math.round(rem.w / 2))
    return { x: rem.x, y: rem.y, width: w, height: rem.h, anchor: 'left' }
  }
  if (nearRight) {
    const w = Math.max(240, Math.round(rem.w / 2))
    return { x: rem.x + rem.w - w, y: rem.y, width: w, height: rem.h, anchor: 'right' }
  }
  if (nearTop) {
    const h = Math.max(120, Math.round(rem.h / 2))
    return { x: rem.x, y: rem.y, width: rem.w, height: h, anchor: 'top' }
  }
  if (nearBottom) {
    const h = Math.max(120, Math.round(rem.h / 2))
    return { x: rem.x, y: rem.y + rem.h - h, width: rem.w, height: h, anchor: 'bottom' }
  }
  return null
}

// ── Layout persistence ─────────────────────────────────────────────────────────
const STORAGE_KEY = 'window-layout'

// Serializable subset of a window. The `component` is rebuilt from the registry
// by `key` on load, so it (and the title) are intentionally not stored.
interface SavedWindow {
  key: string
  /** Registry key for the component; defaults to `key` for older saved layouts. */
  regKey?: string
  props: Record<string, unknown>
  x: number
  y: number
  width: number
  height: number | null
  defaultWidth: number
  zIndex: number
  snapped: boolean
  dockAnchor: DockAnchor | null
  dockOrder: number
}

function toSaved(wins: WinState[]): SavedWindow[] {
  return wins
    .filter(w => w.key && windowRegistry[w.regKey ?? w.key])
    .map(w => ({
      key: w.key as string,
      regKey: w.regKey ?? w.key,
      props: w.props,
      x: w.x,
      y: w.y,
      width: w.width,
      height: w.height,
      defaultWidth: w.defaultWidth,
      zIndex: w.zIndex,
      snapped: w.snapped,
      dockAnchor: w.dockAnchor,
      dockOrder: w.dockOrder,
    }))
}

let persistTimer: ReturnType<typeof setTimeout> | null = null
function persist(wins: WinState[]) {
  // Debounced: drags/resizes mutate state rapidly; one write per idle frame is plenty.
  if (persistTimer) clearTimeout(persistTimer)
  persistTimer = setTimeout(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(toSaved(wins)))
    } catch { /* storage full / unavailable — layout just won't persist */ }
  }, 200)
}

export const useWindowsStore = defineStore('windows', () => {
  const windows = ref<WinState[]>([])
  const snapPreview = ref<SnapPreview | null>(null)

  function open(config: {
    key?: string
    regKey?: string
    title: string
    component: Component
    props?: Record<string, unknown>
    width?: number
    height?: number
    initialDock?: DockAnchor
  }) {
    if (config.key) {
      const existing = windows.value.find((w) => w.key === config.key)
      if (existing) {
        focus(existing.id)
        return
      }
    }
    const w = Math.min(config.width ?? 520, window.innerWidth - 40)
    const h = config.height ? Math.min(config.height, window.innerHeight - 60) : null
    const count = windows.value.length
    windows.value.push({
      id: String(++nextId),
      key: config.key,
      regKey: config.regKey ?? config.key,
      title: config.title,
      component: markRaw(config.component),
      props: config.props ?? {},
      x: Math.max(60, (window.innerWidth - w) / 2 + count * 24),
      y: Math.max(0, (window.innerHeight - (h ?? 600)) / 2 + count * 24),
      width: w,
      height: h,
      defaultWidth: w,
      zIndex: ++nextZ,
      snapped: false,
      dockAnchor: null,
      dockOrder: 0,
    })

    if (config.initialDock) {
      const win = windows.value[windows.value.length - 1]
      win.snapped    = true
      win.dockAnchor = config.initialDock
      win.dockOrder  = Date.now()
      relayout(windows.value)
    }
  }

  // Open a registered window by key, pulling its title/component/defaults from
  // the registry. `propsOverride` varies props (e.g. Settings tab); `dock`
  // overrides the registry's initial dock (used to fill Chat on first run).
  function openKey(key: string, propsOverride?: Record<string, unknown>, dock?: DockAnchor) {
    const def = windowRegistry[key]
    if (!def) return
    // Multi-instance windows get a unique key per instance so each is its own
    // window (no focus-existing dedupe); singletons keep key === regKey.
    const instanceKey = def.multi ? `${key}:${nextId + 1}` : key
    open({
      key: instanceKey,
      regKey: key,
      title: def.title,
      component: def.component,
      props: propsOverride ?? def.props,
      width: def.width,
      height: def.height,
      initialDock: dock ?? def.initialDock,
    })
  }

  // Open (or focus) a chat conversation. If `sessionId` is already shown in a
  // chat window, focus it; otherwise open a new chat window carrying the props.
  // `forceNew` tells a fresh window to start a brand-new session.
  function openChat(opts?: { sessionId?: string; forceNew?: boolean }, dock?: DockAnchor) {
    if (opts?.sessionId) {
      const existing = windows.value.find(
        (w) => w.regKey === 'chat' && w.props.sessionId === opts.sessionId,
      )
      if (existing) {
        focus(existing.id)
        return
      }
    }
    openKey('chat', { sessionId: opts?.sessionId, forceNew: opts?.forceNew }, dock)
  }

  // Merge a patch into a window's props (e.g. a chat window recording the
  // session it resolved to, so a reload restores the same conversation).
  function updateProps(id: string, patch: Record<string, unknown>) {
    const w = windows.value.find((w) => w.id === id)
    if (w) w.props = { ...w.props, ...patch }
  }

  function close(id: string) {
    windows.value = windows.value.filter((w) => w.id !== id)
    relayout(windows.value)
  }

  function focus(id: string) {
    const w = windows.value.find((w) => w.id === id)
    if (w) w.zIndex = ++nextZ
  }

  function move(id: string, x: number, y: number) {
    const w = windows.value.find((w) => w.id === id)
    if (w) {
      w.x = Math.max(0, Math.min(x, window.innerWidth - 80))
      w.y = Math.max(0, Math.min(y, window.innerHeight - 60))
    }
  }

  function snap(id: string, preview: SnapPreview) {
    const w = windows.value.find((w) => w.id === id)
    if (!w) return
    w.snapped    = true
    w.dockAnchor = preview.anchor
    w.dockOrder  = Date.now()
    // Set the primary dimension the user chose; relayout will correct position and the other dimension.
    if (preview.anchor === 'left' || preview.anchor === 'right') w.width  = preview.width
    if (preview.anchor === 'top'  || preview.anchor === 'bottom') w.height = preview.height
    w.zIndex = ++nextZ
    relayout(windows.value)
  }

  function unsnap(id: string) {
    const w = windows.value.find((w) => w.id === id)
    if (!w || !w.snapped) return
    w.width      = w.defaultWidth
    w.height     = null
    w.snapped    = false
    w.dockAnchor = null
    w.dockOrder  = 0
    relayout(windows.value)
  }

  function setSize(id: string, width: number, height: number) {
    const w = windows.value.find((w) => w.id === id)
    if (!w) return
    // For docked windows, only update the primary dimension to avoid fighting relayout.
    if (w.snapped && (w.dockAnchor === 'left' || w.dockAnchor === 'right'))  w.width  = width
    else if (w.snapped && (w.dockAnchor === 'top' || w.dockAnchor === 'bottom')) w.height = height
    else { w.width = width; w.height = height }
    if (w.snapped) relayout(windows.value)
  }

  function setSnapPreview(preview: SnapPreview | null) {
    snapPreview.value = preview
  }

  // Re-derive docked window geometry from the current content area. Call after
  // anything that changes the available space without firing a window resize —
  // e.g. the sidebar collapsing/expanding.
  function reflow() {
    relayout(windows.value)
  }

  // Rebuild the saved window layout from localStorage. Returns true when a saved
  // layout exists (even an empty one), so the caller only falls back to the
  // default Chat on a genuine first run.
  function hydrate(): boolean {
    let saved: SavedWindow[]
    try {
      const raw = localStorage.getItem(STORAGE_KEY)
      if (raw === null) return false           // nothing ever saved → first run
      saved = JSON.parse(raw)
      if (!Array.isArray(saved)) return false  // corrupt → treat as first run
    } catch {
      return false
    }

    const restored: WinState[] = []
    for (const s of saved) {
      const regKey = s.regKey ?? s.key // older layouts stored only `key`
      const def = windowRegistry[regKey]
      if (!def) continue // window type no longer exists — skip it
      restored.push({
        id: String(++nextId),
        key: s.key,
        regKey,
        title: def.title,
        component: markRaw(def.component),
        props: s.props ?? def.props ?? {},
        x: s.x,
        y: s.y,
        width: s.width,
        height: s.height,
        defaultWidth: s.defaultWidth || def.width || s.width,
        zIndex: s.zIndex,
        snapped: s.snapped,
        dockAnchor: s.dockAnchor,
        dockOrder: s.dockOrder,
      })
    }
    // A present-but-empty layout means every window was closed last session —
    // honor that blank state instead of falling back to the default Chat.
    nextZ = restored.reduce((m, w) => Math.max(m, w.zIndex), nextZ) + 1
    windows.value = restored

    // Docked windows re-derive their geometry from the current viewport;
    // floating ones are clamped so a smaller screen doesn't push them off-edge.
    relayout(windows.value)
    for (const w of windows.value) {
      if (w.snapped) continue
      w.x = Math.max(0, Math.min(w.x, window.innerWidth - 80))
      w.y = Math.max(0, Math.min(w.y, window.innerHeight - 60))
      if (w.height !== null) w.height = Math.min(w.height, window.innerHeight - 8)
    }
    return true
  }

  // Reflow docked windows when the viewport changes (e.g. moving the app to a
  // shorter monitor). Without this a `fill`/`left`/`right` window keeps the
  // height derived from the old viewport and spills below the visible area,
  // hiding its bottom content.
  if (typeof window !== 'undefined') {
    window.addEventListener('resize', () => relayout(windows.value))
  }

  // Persist the layout whenever windows change (open/close/move/resize/dock).
  watch(windows, (wins) => persist(wins), { deep: true })

  return { windows, snapPreview, open, openKey, openChat, updateProps, close, focus, move, snap, unsnap, setSize, setSnapPreview, hydrate, reflow }
})
