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
  <div class="sessions">
    <div class="header">
      <h2>Sessions</h2>
      <button @click="newSession">New</button>
    </div>
    <ul>
      <li v-for="s in store.sessions" :key="s.id">
        <button class="title" @click="open(s.id)">{{ s.title }}</button>
        <span class="date">{{ new Date(s.updated_at).toLocaleDateString() }}</span>
        <button class="del" @click="remove(s.id)">✕</button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.sessions { padding: 1.25rem; max-width: 40rem; }
.header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem; }
h2 { font-size: 1rem; }
ul { list-style: none; display: flex; flex-direction: column; gap: 0.5rem; }
li { display: flex; align-items: center; gap: 0.75rem; background: #1a1a1a; border-radius: 0.375rem; padding: 0.6rem 0.8rem; }
.title { flex: 1; background: none; border: none; color: inherit; text-align: left; cursor: pointer; font-size: 0.9rem; }
.date { font-size: 0.75rem; color: #606060; }
.del { background: none; border: none; color: #606060; cursor: pointer; font-size: 0.75rem; }
button { background: #2a4a7a; color: #e0e0e0; border: none; border-radius: 0.375rem; padding: 0.4rem 0.8rem; cursor: pointer; font-size: 0.85rem; }
</style>
