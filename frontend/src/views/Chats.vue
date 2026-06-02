<script setup lang="ts">
import { onMounted } from 'vue'
import { useSessionsStore } from '../stores/sessions'
import { useWindowsStore } from '../stores/windows'
import Chat from './Chat.vue'
import * as api from '../api'

const store = useSessionsStore()
const winStore = useWindowsStore()

onMounted(() => store.fetchSessions())

async function open(id: string) {
  await store.loadSession(id)
  winStore.open({ key: 'chat', title: 'Chat', component: Chat, width: 740, height: 560, initialDock: 'fill' })
}

async function remove(id: string) {
  await api.sessions.delete(id)
  store.sessions.splice(store.sessions.findIndex((s) => s.id === id), 1)
}
</script>

<template>
  <div class="p-5 max-w-[40rem]">
    <div class="mb-4">
      <h2 class="text-base font-semibold">Chats</h2>
    </div>
    <ul v-if="store.sessions.length" class="list-none flex flex-col gap-1.5">
      <li v-for="s in store.sessions" :key="s.id" class="flex items-center gap-3 bg-surface rounded-md py-[0.6rem] px-[0.8rem]">
        <button class="flex-1 bg-none border-none text-[#d0d0d0] text-left cursor-pointer text-sm font-[inherit] hover:text-[#fff]" @click="open(s.id)">{{ s.title }}</button>
        <span class="text-xs text-[#505050] whitespace-nowrap">{{ new Date(s.updated_at).toLocaleDateString() }}</span>
        <button class="bg-none border-none text-[#505050] cursor-pointer text-xs py-[0.2rem] px-[0.4rem] rounded-[0.2rem] hover:text-[#d08080]" @click="remove(s.id)">✕</button>
      </li>
    </ul>
    <p v-else class="text-[#505050] text-sm">No chats yet.</p>
  </div>
</template>
