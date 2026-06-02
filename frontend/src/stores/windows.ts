import { defineStore } from 'pinia'
import { ref, markRaw } from 'vue'
import type { Component } from 'vue'

export interface SnapPreview {
  x: number
  y: number
  width: number
  height: number
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
  minimized: boolean
  zIndex: number
  snapped: boolean
}

let nextZ = 10
let nextId = 0

const SNAP_THRESHOLD = 40

function contentBounds() {
  const sidebar = document.querySelector('.sidebar') as HTMLElement | null
  const taskbar = document.querySelector('.taskbar') as HTMLElement | null
  const sidebarW = sidebar ? sidebar.offsetWidth : 0
  const taskbarH = taskbar ? taskbar.offsetHeight : 0
  return {
    x: sidebarW,
    y: 0,
    w: window.innerWidth - sidebarW,
    h: window.innerHeight - taskbarH,
  }
}

export function snapPreviewForMouse(mouseX: number, mouseY: number): SnapPreview | null {
  const b = contentBounds()
  const snapW = Math.min(480, Math.max(360, Math.floor(b.w * 0.4)))
  if (mouseX <= b.x + SNAP_THRESHOLD) {
    return { x: b.x, y: b.y, width: snapW, height: b.h }
  }
  if (mouseX >= b.x + b.w - SNAP_THRESHOLD) {
    return { x: b.x + b.w - snapW, y: b.y, width: snapW, height: b.h }
  }
  if (mouseY <= b.y + SNAP_THRESHOLD) {
    return { x: b.x, y: b.y, width: b.w, height: b.h }
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
  }) {
    if (config.key) {
      const existing = windows.value.find((w) => w.key === config.key)
      if (existing) {
        existing.minimized = false
        focus(existing.id)
        return
      }
    }
    const w = Math.min(config.width ?? 520, window.innerWidth - 40)
    const h = config.height ? Math.min(config.height, window.innerHeight - 60) : null
    const count = windows.value.filter((win) => !win.minimized).length
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
      minimized: false,
      zIndex: ++nextZ,
      snapped: false,
    })
  }

  function close(id: string) {
    windows.value = windows.value.filter((w) => w.id !== id)
  }

  function minimize(id: string) {
    const w = windows.value.find((w) => w.id === id)
    if (w) w.minimized = true
  }

  function restore(id: string) {
    const w = windows.value.find((w) => w.id === id)
    if (w) {
      w.minimized = false
      focus(id)
    }
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
    w.x = preview.x
    w.y = preview.y
    w.width = preview.width
    w.height = preview.height
    w.snapped = true
    w.zIndex = ++nextZ
  }

  function unsnap(id: string) {
    const w = windows.value.find((w) => w.id === id)
    if (!w || !w.snapped) return
    w.width = w.defaultWidth
    w.height = null
    w.snapped = false
  }

  function setSize(id: string, width: number, height: number) {
    const w = windows.value.find((w) => w.id === id)
    if (w) { w.width = width; w.height = height }
  }

  function setSnapPreview(preview: SnapPreview | null) {
    snapPreview.value = preview
  }

  return { windows, snapPreview, open, close, minimize, restore, focus, move, snap, unsnap, setSize, setSnapPreview }
})
