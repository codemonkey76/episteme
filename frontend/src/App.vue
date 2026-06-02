<script setup lang="ts">
import { useWindowsStore } from './stores/windows'
import AppSidebar from './components/AppSidebar.vue'
import AppWindow from './components/AppWindow.vue'
import AppTaskbar from './components/AppTaskbar.vue'

const winStore = useWindowsStore()
</script>

<template>
  <div id="layout">
    <AppSidebar />
    <main>
      <RouterView />
    </main>

    <Transition name="snap-fade">
      <div
        v-if="winStore.snapPreview"
        class="snap-preview"
        :style="{
          left: winStore.snapPreview.x + 'px',
          top: winStore.snapPreview.y + 'px',
          width: winStore.snapPreview.width + 'px',
          height: winStore.snapPreview.height + 'px',
        }"
      />
    </Transition>

    <AppWindow v-for="win in winStore.windows" :key="win.id" :win="win" />
    <AppTaskbar />
  </div>
</template>

<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: system-ui, sans-serif; background: #0f0f0f; color: #e0e0e0; height: 100vh; overflow: hidden; }
#layout { display: flex; flex-direction: row; height: 100vh; overflow: hidden; }
main { flex: 1; overflow: hidden; min-width: 0; }

.snap-preview {
  position: fixed;
  background: rgba(74, 138, 255, 0.08);
  border: 2px solid rgba(74, 138, 255, 0.35);
  border-radius: 0.5rem;
  pointer-events: none;
  z-index: 9000;
}

.snap-fade-enter-active,
.snap-fade-leave-active {
  transition: opacity 0.1s ease;
}
.snap-fade-enter-from,
.snap-fade-leave-to {
  opacity: 0;
}
</style>
