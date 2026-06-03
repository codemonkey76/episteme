import { defineStore } from 'pinia'
import { ref } from 'vue'

// Cross-window signal: bumped whenever something changes the calendar (e.g. the
// chat AI creates/deletes an event) so the Calendar window can refresh live.
export const useCalendarStore = defineStore('calendar', () => {
  const changeToken = ref(0)
  function notifyChanged() {
    changeToken.value++
  }
  return { changeToken, notifyChanged }
})
