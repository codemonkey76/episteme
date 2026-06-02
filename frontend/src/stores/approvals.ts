import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api'

export const useApprovalsStore = defineStore('approvals', () => {
  const pending = ref<api.PendingAction[]>([])

  async function fetchPending(sessionId: string) {
    const res = await api.approvals.listPending(sessionId)
    pending.value = res.pending_actions
  }

  async function approve(actionId: string) {
    await api.approvals.approve(actionId)
    pending.value = pending.value.filter((a) => a.id !== actionId)
  }

  async function reject(actionId: string) {
    await api.approvals.reject(actionId)
    pending.value = pending.value.filter((a) => a.id !== actionId)
  }

  function addPending(action: api.PendingAction) {
    pending.value.push(action)
  }

  return { pending, fetchPending, approve, reject, addPending }
})
