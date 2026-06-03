import { defineStore } from 'pinia'
import { ref } from 'vue'

// Cross-window signal: bumped whenever something changes the task list (e.g.
// the chat AI creates/completes a task) so the Tasks window can refresh live.
export const useTasksStore = defineStore('tasks', () => {
  const changeToken = ref(0)
  function notifyChanged() {
    changeToken.value++
  }
  return { changeToken, notifyChanged }
})
