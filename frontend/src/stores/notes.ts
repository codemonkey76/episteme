import { defineStore } from 'pinia'
import { ref } from 'vue'

// Cross-window signal: bumped whenever something changes the notes (e.g. the
// chat AI saves or edits one) so the Notes window can refresh live.
export const useNotesStore = defineStore('notes', () => {
  const changeToken = ref(0)
  function notifyChanged() {
    changeToken.value++
  }
  return { changeToken, notifyChanged }
})
