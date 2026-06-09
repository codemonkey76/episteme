import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api'

export const useSessionsStore = defineStore('sessions', () => {
  const sessions = ref<api.Session[]>([])
  const activeSession = ref<api.Session | null>(null)
  const messages = ref<api.Message[]>([])

  async function fetchSessions() {
    const res = await api.sessions.list()
    sessions.value = res.sessions
  }

  async function createSession(title?: string) {
    const res = await api.sessions.create(title)
    sessions.value.unshift(res.session)
    return res.session
  }

  async function loadSession(id: string) {
    const [sessionRes, msgRes] = await Promise.all([
      api.sessions.get(id),
      api.sessions.messages(id),
    ])
    activeSession.value = sessionRes.session
    messages.value = msgRes.messages
  }

  function appendMessage(msg: api.Message) {
    messages.value.push(msg)
  }

  // Apply an auto-generated (or renamed) title to the active session and the list.
  function setSessionTitle(id: string, title: string) {
    if (activeSession.value?.id === id) activeSession.value.title = title
    const s = sessions.value.find(x => x.id === id)
    if (s) s.title = title
  }

  // Display-only indicator that the AI ran a tool (not persisted).
  function appendToolCall(name: string) {
    messages.value.push({
      id: crypto.randomUUID(),
      session_id: activeSession.value?.id ?? '',
      role: 'tool_call',
      content: name,
      created_at: new Date().toISOString(),
    })
  }

  function appendToken(text: string) {
    const last = messages.value[messages.value.length - 1]
    if (last?.role === 'assistant') {
      last.content += text
    } else {
      messages.value.push({
        id: crypto.randomUUID(),
        session_id: activeSession.value?.id ?? '',
        role: 'assistant',
        content: text,
        created_at: new Date().toISOString(),
      })
    }
  }

  return { sessions, activeSession, messages, fetchSessions, createSession, loadSession, appendMessage, setSessionTitle, appendToolCall, appendToken }
})
