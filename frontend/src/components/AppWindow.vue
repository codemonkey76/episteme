<script setup lang="ts">
import { ref, computed } from 'vue'
import { useWindowsStore, snapPreviewForMouse } from '../stores/windows'
import type { WinState } from '../stores/windows'

const props = defineProps<{ win: WinState }>()
const store = useWindowsStore()
const windowEl = ref<HTMLElement>()

// ── Drag ─────────────────────────────────────────────────────────────────────
let dragging = false
let startX = 0, startY = 0, startWinX = 0, startWinY = 0

function onDragDown(e: PointerEvent) {
  if ((e.target as HTMLElement).closest('.win-actions')) return
  dragging = true

  if (props.win.snapped) {
    store.unsnap(props.win.id)
    // Centre the restored window under the cursor so it doesn't stay at the snap edge
    startWinX = e.clientX - props.win.width / 2
    startWinY = props.win.y
    store.move(props.win.id, startWinX, startWinY)
  } else {
    startWinX = props.win.x
    startWinY = props.win.y
  }

  startX = e.clientX
  startY = e.clientY
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  store.focus(props.win.id)
}

function onDragMove(e: PointerEvent) {
  if (!dragging) return
  store.move(props.win.id, startWinX + e.clientX - startX, startWinY + e.clientY - startY)
  store.setSnapPreview(snapPreviewForMouse(e.clientX, e.clientY))
}

function onDragUp(e: PointerEvent) {
  if (!dragging) return
  dragging = false
  const preview = snapPreviewForMouse(e.clientX, e.clientY)
  if (preview) store.snap(props.win.id, preview)
  store.setSnapPreview(null)
}

// ── Resize ────────────────────────────────────────────────────────────────────
let resizing = false
let resizeDir: 'e' | 's' | 'se' = 'se'
let resizeStartX = 0, resizeStartY = 0, resizeStartW = 0, resizeStartH = 0

function onResizeDown(e: PointerEvent, dir: 'e' | 's' | 'se') {
  e.stopPropagation()
  resizing = true
  resizeDir = dir
  resizeStartX = e.clientX
  resizeStartY = e.clientY
  resizeStartW = props.win.width
  resizeStartH = props.win.height ?? windowEl.value?.offsetHeight ?? 400
  if (props.win.height === null) store.setSize(props.win.id, resizeStartW, resizeStartH)
  if (props.win.snapped) store.unsnap(props.win.id)
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  store.focus(props.win.id)
}

function onResizeMove(e: PointerEvent) {
  if (!resizing) return
  const dx = e.clientX - resizeStartX
  const dy = e.clientY - resizeStartY
  const w = resizeDir !== 's' ? Math.max(280, resizeStartW + dx) : resizeStartW
  const h = resizeDir !== 'e' ? Math.max(200, resizeStartH + dy) : resizeStartH
  store.setSize(props.win.id, w, h)
}

function onResizeUp() { resizing = false }

// ── Style ─────────────────────────────────────────────────────────────────────
const windowStyle = computed(() => ({
  left: props.win.x + 'px',
  top: props.win.y + 'px',
  width: props.win.width + 'px',
  ...(props.win.height !== null
    ? { height: props.win.height + 'px', maxHeight: 'none' }
    : {}),
  ...(props.win.snapped ? { borderRadius: '0' } : {}),
  zIndex: props.win.zIndex,
}))
</script>

<template>
  <div
    v-if="!win.minimized"
    ref="windowEl"
    class="app-window"
    :style="windowStyle"
    @pointerdown="store.focus(win.id)"
  >
    <div
      class="win-titlebar"
      @pointerdown="onDragDown"
      @pointermove="onDragMove"
      @pointerup="onDragUp"
    >
      <span class="win-title">{{ win.title }}</span>
      <div class="win-actions">
        <button class="win-btn" title="Minimize" @click.stop="store.minimize(win.id)">
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <line x1="5" y1="12" x2="19" y2="12"/>
          </svg>
        </button>
        <button class="win-btn close" title="Close" @click.stop="store.close(win.id)">
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <line x1="18" y1="6" x2="6" y2="18"/>
            <line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="win-body">
      <component :is="win.component" v-bind="win.props" />
    </div>

    <!-- Resize handles -->
    <div
      class="resize-handle resize-e"
      @pointerdown="onResizeDown($event, 'e')"
      @pointermove="onResizeMove"
      @pointerup="onResizeUp"
    />
    <div
      class="resize-handle resize-s"
      @pointerdown="onResizeDown($event, 's')"
      @pointermove="onResizeMove"
      @pointerup="onResizeUp"
    />
    <div
      class="resize-handle resize-se"
      @pointerdown="onResizeDown($event, 'se')"
      @pointermove="onResizeMove"
      @pointerup="onResizeUp"
    />
  </div>
</template>

<style scoped>
.app-window {
  position: fixed;
  background: #1a1a1a;
  border: 1px solid #383838;
  border-radius: 0.5rem;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.6);
  display: flex;
  flex-direction: column;
  max-height: 85vh;
  min-width: 280px;
  transition: border-radius 0.12s ease;
}

.win-titlebar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.625rem 0.5rem 0.875rem;
  background: #222;
  border-bottom: 1px solid #2a2a2a;
  cursor: grab;
  user-select: none;
  border-radius: 0.5rem 0.5rem 0 0;
  flex-shrink: 0;
  overflow: hidden;
}

.win-titlebar:active { cursor: grabbing; }

.win-title {
  font-size: 0.8rem;
  font-weight: 600;
  color: #c0c0c0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.win-actions {
  display: flex;
  gap: 0.375rem;
  flex-shrink: 0;
  margin-left: 0.5rem;
}

.win-btn {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #3a3a3a;
  color: #808080;
  transition: background 0.15s, color 0.15s;
}

.win-btn:hover { background: #505050; color: #e0e0e0; }
.win-btn.close:hover { background: #7a2a2a; color: #ffaaaa; }

.win-body {
  overflow-y: auto;
  flex: 1;
  border-radius: 0 0 0.5rem 0.5rem;
}

/* ── Resize handles ── */
.resize-e {
  position: absolute;
  right: 0;
  top: 30px;
  bottom: 18px;
  width: 6px;
  cursor: ew-resize;
  z-index: 20;
}

.resize-s {
  position: absolute;
  bottom: 0;
  left: 30px;
  right: 18px;
  height: 6px;
  cursor: ns-resize;
  z-index: 20;
}

.resize-se {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 18px;
  height: 18px;
  cursor: nwse-resize;
  z-index: 21;
}

/* Always-visible corner grip */
.resize-se::after {
  content: '';
  position: absolute;
  right: 4px;
  bottom: 4px;
  width: 9px;
  height: 9px;
  border-right: 2px solid #484848;
  border-bottom: 2px solid #484848;
  border-radius: 0 0 2px 0;
  transition: border-color 0.15s;
}

.resize-e:hover  { background: linear-gradient(to left,  rgba(74,138,255,0.2), transparent); }
.resize-s:hover  { background: linear-gradient(to top,   rgba(74,138,255,0.2), transparent); }
.resize-se:hover::after { border-color: rgba(74,138,255,0.8); }
</style>
