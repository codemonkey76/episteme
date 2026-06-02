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

function otherWindows() {
  return store.windows.filter(w => w.id !== props.win.id)
}

function onDragMove(e: PointerEvent) {
  if (!dragging) return
  store.move(props.win.id, startWinX + e.clientX - startX, startWinY + e.clientY - startY)
  store.setSnapPreview(snapPreviewForMouse(e.clientX, e.clientY, otherWindows()))
}

function onDragUp(e: PointerEvent) {
  if (!dragging) return
  dragging = false
  const preview = snapPreviewForMouse(e.clientX, e.clientY, otherWindows())
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
    ref="windowEl"
    class="fixed flex flex-col bg-surface border border-[#383838] rounded-lg shadow-[0_12px_40px_rgba(0,0,0,0.6)] max-h-[85vh] min-w-[280px] transition-[border-radius] duration-[120ms] ease-[ease]"
    :style="windowStyle"
    @pointerdown="store.focus(win.id)"
  >
    <div
      :class="[
        'win-titlebar flex items-center justify-between cursor-grab active:cursor-grabbing select-none rounded-t-lg shrink-0 overflow-hidden transition-[padding] duration-150 ease-[ease]',
        win.snapped
          ? 'py-[0.2rem] pr-2 pl-[0.625rem] bg-[#1c1c1c]'
          : 'pt-2 pr-2.5 pb-2 pl-3.5 bg-[#222] border-b border-raised',
      ]"
      @pointerdown="onDragDown"
      @pointermove="onDragMove"
      @pointerup="onDragUp"
    >
      <span
        :class="[
          'win-title font-semibold whitespace-nowrap overflow-hidden text-ellipsis',
          win.snapped ? 'text-[0.72rem] text-[#808080]' : 'text-[0.8rem] text-[#c0c0c0]',
        ]"
      >{{ win.title }}</span>
      <div class="win-actions flex gap-1.5 shrink-0 ml-2">
        <button
          :class="[
            'win-btn rounded-full flex items-center justify-center cursor-pointer bg-[#3a3a3a] text-[#808080] transition-[background,color] duration-150 hover:bg-[#7a2a2a] hover:text-[#ffaaaa]',
            win.snapped ? 'w-4 h-4' : 'w-5 h-5',
          ]"
          title="Close"
          @click.stop="store.close(win.id)"
        >
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <line x1="18" y1="6" x2="6" y2="18"/>
            <line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="overflow-y-auto flex-1 rounded-b-lg">
      <component :is="win.component" v-bind="win.props" />
    </div>

    <!-- Resize handles -->
    <div
      class="absolute right-0 top-[30px] bottom-[18px] w-1.5 cursor-[ew-resize] z-20 hover:bg-[linear-gradient(to_left,rgba(74,138,255,0.2),transparent)]"
      @pointerdown="onResizeDown($event, 'e')"
      @pointermove="onResizeMove"
      @pointerup="onResizeUp"
    />
    <div
      class="absolute bottom-0 left-[30px] right-[18px] h-1.5 cursor-[ns-resize] z-20 hover:bg-[linear-gradient(to_top,rgba(74,138,255,0.2),transparent)]"
      @pointerdown="onResizeDown($event, 's')"
      @pointermove="onResizeMove"
      @pointerup="onResizeUp"
    />
    <div
      class="resize-se absolute right-0 bottom-0 w-[18px] h-[18px] cursor-nwse-resize z-[21]"
      @pointerdown="onResizeDown($event, 'se')"
      @pointermove="onResizeMove"
      @pointerup="onResizeUp"
    />
  </div>
</template>
