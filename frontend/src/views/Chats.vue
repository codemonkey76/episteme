<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useSessionsStore } from '../stores/sessions'
import * as api from '../api'

const store = useSessionsStore()
const router = useRouter()

onMounted(() => store.fetchSessions())

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
  <div class="chats">
    <div class="header">
      <h2>Chats</h2>
    </div>
    <ul v-if="store.sessions.length">
      <li v-for="s in store.sessions" :key="s.id">
        <button class="title" @click="open(s.id)">{{ s.title }}</button>
        <span class="date">{{ new Date(s.updated_at).toLocaleDateString() }}</span>
        <button class="del" @click="remove(s.id)">✕</button>
      </li>
    </ul>
    <p v-else class="empty">No chats yet.</p>
  </div>
</template>

<style scoped>
.chats { padding: 1.25rem; max-width: 40rem; }
.header { margin-bottom: 1rem; }
h2 { font-size: 1rem; font-weight: 600; }
ul { list-style: none; display: flex; flex-direction: column; gap: 0.375rem; }
li { display: flex; align-items: center; gap: 0.75rem; background: #1a1a1a; border-radius: 0.375rem; padding: 0.6rem 0.8rem; }
.title { flex: 1; background: none; border: none; color: #d0d0d0; text-align: left; cursor: pointer; font-size: 0.875rem; font-family: inherit; }
.title:hover { color: #fff; }
.date { font-size: 0.75rem; color: #505050; white-space: nowrap; }
.del { background: none; border: none; color: #505050; cursor: pointer; font-size: 0.75rem; padding: 0.2rem 0.4rem; border-radius: 0.2rem; }
.del:hover { color: #d08080; }
.empty { color: #505050; font-size: 0.875rem; }
</style>
