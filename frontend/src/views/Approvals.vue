<script setup lang="ts">
import { onMounted } from 'vue'
import { useApprovalsStore } from '../stores/approvals'
import { useSessionsStore } from '../stores/sessions'
import ApprovalCard from '../components/ApprovalCard.vue'

const store = useApprovalsStore()
const sessions = useSessionsStore()

onMounted(async () => {
  if (sessions.activeSession) {
    await store.fetchPending(sessions.activeSession.id)
  }
})
</script>

<template>
  <div class="p-5 max-w-[40rem]">
    <h2 class="text-base mb-4">Pending approvals</h2>
    <p v-if="!sessions.activeSession" class="text-[var(--c-606060)] text-sm">No active session.</p>
    <p v-else-if="store.pending.length === 0" class="text-[var(--c-606060)] text-sm">No pending actions.</p>
    <ul v-else class="list-none flex flex-col gap-4">
      <li v-for="action in store.pending" :key="action.id">
        <ApprovalCard
          :tool-name="action.tool_name"
          :tool-args="action.tool_args"
          @approve="(edited) => store.approve(action.id, edited)"
          @reject="store.reject(action.id)"
        />
      </li>
    </ul>
  </div>
</template>
