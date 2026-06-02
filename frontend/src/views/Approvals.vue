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
  <div class="p-5 max-w-[40rem]">
    <h2 class="text-base mb-4">Pending approvals</h2>
    <p v-if="!sessions.activeSession" class="text-[#606060] text-sm">No active session.</p>
    <p v-else-if="store.pending.length === 0" class="text-[#606060] text-sm">No pending actions.</p>
    <ul v-else class="list-none flex flex-col gap-4">
      <li v-for="action in store.pending" :key="action.id" class="bg-surface border border-raised rounded-lg p-4">
        <div class="font-semibold mb-2">{{ action.tool_name }}</div>
        <pre class="text-[0.8rem] bg-[#111] p-2 rounded overflow-x-auto mb-3">{{ JSON.stringify(JSON.parse(action.tool_args), null, 2) }}</pre>
        <div class="flex gap-2">
          <button class="border-none rounded-md py-[0.4rem] px-[0.9rem] cursor-pointer text-[0.85rem] bg-[#2a5a2a] text-[#c0e0c0]" @click="store.approve(action.id)">Approve</button>
          <button class="border-none rounded-md py-[0.4rem] px-[0.9rem] cursor-pointer text-[0.85rem] bg-[#5a2a2a] text-[#e0c0c0]" @click="store.reject(action.id)">Reject</button>
        </div>
      </li>
    </ul>
  </div>
</template>
