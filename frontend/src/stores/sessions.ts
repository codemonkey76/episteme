import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api'

// Shared session *list* only. Per-conversation state (active session, messages,
// streaming) lives in each Chat window instance so several chat windows can run
// independent conversations side by side.
export const useSessionsStore = defineStore('sessions', () => {
  const sessions = ref<api.Session[]>([])

  async function fetchSessions() {
    const res = await api.sessions.list()
    sessions.value = res.sessions
  }

  async function createSession(title?: string) {
    const res = await api.sessions.create(title)
    sessions.value.unshift(res.session)
    return res.session
  }

  // Apply an auto-generated (or renamed) title to the list entry.
  function setSessionTitle(id: string, title: string) {
    const s = sessions.value.find(x => x.id === id)
    if (s) s.title = title
  }

  return { sessions, fetchSessions, createSession, setSessionTitle }
})
