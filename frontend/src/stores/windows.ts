import { defineStore } from 'pinia'
import { ref, markRaw } from 'vue'
import type { Component } from 'vue'

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
  key?: string
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
  if (rem.w < 200 || rem.h < 120) return null

  const ex = Math.min(80, Math.floor(rem.w * 0.22))
  const ey = Math.min(60, Math.floor(rem.h * 0.22))

  const nearLeft   = mouseX <= rem.x + ex
  const nearRight  = mouseX >= rem.x + rem.w - ex
  const nearTop    = mouseY <= rem.y + ey
  const nearBottom = mouseY >= rem.y + rem.h - ey

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

export const useWindowsStore = defineStore('windows', () => {
  const windows = ref<WinState[]>([])
  const snapPreview = ref<SnapPreview | null>(null)

  function open(config: {
    key?: string
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

  return { windows, snapPreview, open, close, focus, move, snap, unsnap, setSize, setSnapPreview }
})
