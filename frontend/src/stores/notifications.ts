import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api'

// Holds the unread notification count for the sidebar badge and a change token
// the Notifications window watches to refresh live. `refresh()` is polled from
// the sidebar and called after the window mutates notifications.
export const useNotificationsStore = defineStore('notifications', () => {
  const unread = ref(0)
  const changeToken = ref(0)

  async function refresh() {
    try {
      const res = await api.notifications.list(1)
      unread.value = res.unread
    } catch { /* offline / unauthenticated — leave the last count */ }
  }

  // Bumped by the window after it marks read / clears, so the badge updates and
  // other listeners can react.
  function notifyChanged() {
    changeToken.value++
    refresh()
  }

  return { unread, changeToken, refresh, notifyChanged }
})
