<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useSessionsStore } from '../stores/sessions'
import * as api from '../api'

const store = useSessionsStore()
const router = useRouter()

onMounted(() => store.fetchSessions())

async function newSession() {
  const s = await store.createSession()
  await store.loadSession(s.id)
  router.push('/')
}

async function open(id: string) {
  await store.loadSession(id)
  router.push('/')
}

async function remove(id: string) {
  await api.sessions.delete(id)
  store.sessions.splice(store.sessions.findIndex((s) => s.id === id), 1)
}
</script>

<template>
  <div class="p-5 max-w-[40rem]">
    <div class="flex justify-between items-center mb-4">
      <h2 class="text-base">Sessions</h2>
      <button @click="newSession" class="bg-[#2a4a7a] text-fg border-none rounded-md py-1.5 px-3 cursor-pointer text-[0.85rem]">New</button>
    </div>
    <ul class="list-none flex flex-col gap-2">
      <li v-for="s in store.sessions" :key="s.id" class="flex items-center gap-3 bg-surface rounded-md py-[0.6rem] px-[0.8rem]">
        <button class="flex-1 bg-none border-none text-inherit text-left cursor-pointer text-[0.9rem]" @click="open(s.id)">{{ s.title }}</button>
        <span class="text-xs text-[#606060]">{{ new Date(s.updated_at).toLocaleDateString() }}</span>
        <button class="bg-none border-none text-[#606060] cursor-pointer text-xs" @click="remove(s.id)">✕</button>
      </li>
    </ul>
  </div>
</template>
