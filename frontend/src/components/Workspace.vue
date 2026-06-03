<script setup lang="ts">
import { onMounted } from 'vue'
import { useWindowsStore } from '../stores/windows'
import { useLogsStore } from '../stores/logs'
import AppSidebar from './AppSidebar.vue'
import AppWindow from './AppWindow.vue'

const winStore = useWindowsStore()
useLogsStore().init()

onMounted(() => {
  // Restore the previous session's window layout; on first run, open Chat filled.
  if (!winStore.hydrate()) {
    winStore.openKey('chat', undefined, 'fill')
  }
})
</script>

<template>
  <div class="flex flex-col h-screen overflow-hidden">
    <div class="flex flex-row flex-1 min-h-0 overflow-hidden">
      <AppSidebar />
    </div>

    <!-- Fixed overlays — must be outside the content row so they aren't clipped -->
    <Transition
      enter-active-class="transition-opacity duration-100"
      leave-active-class="transition-opacity duration-100"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div
        v-if="winStore.snapPreview"
        class="fixed z-[9000] flex items-center justify-center rounded-lg border-2 border-accent/35 bg-accent/8 pointer-events-none"
        :style="{
          left: winStore.snapPreview.x + 'px',
          top: winStore.snapPreview.y + 'px',
          width: winStore.snapPreview.width + 'px',
          height: winStore.snapPreview.height + 'px',
        }"
      >
        <span class="text-[0.7rem] font-bold uppercase tracking-[0.1em] text-accent/60 pointer-events-none">
          {{ winStore.snapPreview.anchor }}
        </span>
      </div>
    </Transition>

    <AppWindow v-for="win in winStore.windows" :key="win.id" :win="win" />
  </div>
</template>
