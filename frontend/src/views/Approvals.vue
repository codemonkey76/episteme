<script setup lang="ts">
import { onMounted } from 'vue'
import { useApprovalsStore } from '../stores/approvals'
import { useSessionsStore } from '../stores/sessions'

const store = useApprovalsStore()
const sessions = useSessionsStore()

onMounted(async () => {
  if (sessions.activeSession) {
    await store.fetchPending(sessions.activeSession.id)
  }
})
</script>

<template>
  <div class="approvals">
    <h2>Pending approvals</h2>
    <p v-if="!sessions.activeSession" class="empty">No active session.</p>
    <p v-else-if="store.pending.length === 0" class="empty">No pending actions.</p>
    <ul v-else>
      <li v-for="action in store.pending" :key="action.id">
        <div class="tool-name">{{ action.tool_name }}</div>
        <pre class="args">{{ JSON.stringify(JSON.parse(action.tool_args), null, 2) }}</pre>
        <div class="actions">
          <button class="approve" @click="store.approve(action.id)">Approve</button>
          <button class="reject" @click="store.reject(action.id)">Reject</button>
        </div>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.approvals { padding: 1.25rem; max-width: 40rem; }
h2 { font-size: 1rem; margin-bottom: 1rem; }
.empty { color: #606060; font-size: 0.875rem; }
ul { list-style: none; display: flex; flex-direction: column; gap: 1rem; }
li { background: #1a1a1a; border: 1px solid #2a2a2a; border-radius: 0.5rem; padding: 1rem; }
.tool-name { font-weight: 600; margin-bottom: 0.5rem; }
.args { font-size: 0.8rem; background: #111; padding: 0.5rem; border-radius: 0.25rem; overflow-x: auto; margin-bottom: 0.75rem; }
.actions { display: flex; gap: 0.5rem; }
button { border: none; border-radius: 0.375rem; padding: 0.4rem 0.9rem; cursor: pointer; font-size: 0.85rem; }
.approve { background: #2a5a2a; color: #c0e0c0; }
.reject { background: #5a2a2a; color: #e0c0c0; }
</style>
