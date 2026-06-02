<script setup lang="ts">
import { useWindowsStore } from '../stores/windows'

const store = useWindowsStore()

function onClick(id: string, minimized: boolean) {
  if (minimized) {
    store.restore(id)
  } else {
    store.minimize(id)
  }
}
</script>

<template>
  <div v-if="store.windows.length" class="taskbar">
    <button
      v-for="win in store.windows"
      :key="win.id"
      class="task-btn"
      :class="{ minimized: win.minimized }"
      @click="onClick(win.id, win.minimized)"
    >
      {{ win.title }}
    </button>
  </div>
</template>

<style scoped>
.taskbar {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  height: 36px;
  background: #141414;
  border-top: 1px solid #2a2a2a;
  display: flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0 0.5rem;
  z-index: 500;
}

.task-btn {
  height: 26px;
  padding: 0 0.75rem;
  background: #252525;
  border: 1px solid #333;
  border-radius: 0.25rem;
  color: #c0c0c0;
  font-size: 0.75rem;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
  white-space: nowrap;
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: inherit;
}

.task-btn:hover {
  background: #303030;
  color: #e0e0e0;
}

.task-btn.minimized {
  color: #606060;
  background: #1c1c1c;
  border-color: #282828;
}
</style>
