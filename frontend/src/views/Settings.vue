<script setup lang="ts">
import { ref, onMounted } from 'vue'
import * as api from '../api'

const providers = ref<api.ProviderConfig[]>([])
const mcpServers = ref<api.McpServerConfig[]>([])

onMounted(async () => {
  const [pRes, mRes] = await Promise.all([
    api.settings.listProviders(),
    api.settings.listMcpServers(),
  ])
  providers.value = pRes.providers
  mcpServers.value = mRes.mcp_servers
})

// Provider form
const newProvider = ref<api.ProviderConfig>({
  name: '', provider: 'anthropic', model_id: '', base_url: '', api_key: '',
})

async function saveProvider() {
  await api.settings.upsertProvider(newProvider.value)
  providers.value = (await api.settings.listProviders()).providers
  newProvider.value = { name: '', provider: 'anthropic', model_id: '', base_url: '', api_key: '' }
}

async function deleteProvider(name: string) {
  await api.settings.deleteProvider(name)
  providers.value = providers.value.filter((p) => p.name !== name)
}
</script>

<template>
  <div class="settings">
    <section>
      <h2>Model providers</h2>
      <ul v-if="providers.length">
        <li v-for="p in providers" :key="p.name">
          <span>{{ p.name }} — {{ p.provider }} / {{ p.model_id }}</span>
          <button @click="deleteProvider(p.name)">Remove</button>
        </li>
      </ul>
      <p v-else class="empty">No providers configured.</p>

      <form @submit.prevent="saveProvider" class="form">
        <h3>Add / update provider</h3>
        <label>Name <input v-model="newProvider.name" required /></label>
        <label>Provider
          <select v-model="newProvider.provider">
            <option>anthropic</option>
            <option>openai</option>
            <option>ollama</option>
            <option>openai_compatible</option>
          </select>
        </label>
        <label>Model ID <input v-model="newProvider.model_id" required /></label>
        <label>Base URL <input v-model="newProvider.base_url" /></label>
        <label>API Key <input v-model="newProvider.api_key" type="password" /></label>
        <button type="submit">Save</button>
      </form>
    </section>

    <section>
      <h2>MCP servers</h2>
      <ul v-if="mcpServers.length">
        <li v-for="s in mcpServers" :key="s.name">
          <span>{{ s.name }}</span>
        </li>
      </ul>
      <p v-else class="empty">No MCP servers configured.</p>
    </section>
  </div>
</template>

<style scoped>
.settings { padding: 1.25rem; max-width: 40rem; display: flex; flex-direction: column; gap: 2rem; }
section { display: flex; flex-direction: column; gap: 0.75rem; }
h2 { font-size: 1rem; border-bottom: 1px solid #2a2a2a; padding-bottom: 0.4rem; }
h3 { font-size: 0.9rem; color: #a0a0a0; }
.empty { color: #606060; font-size: 0.875rem; }
ul { list-style: none; display: flex; flex-direction: column; gap: 0.4rem; }
li { display: flex; justify-content: space-between; align-items: center; background: #1a1a1a; border-radius: 0.375rem; padding: 0.5rem 0.75rem; font-size: 0.875rem; }
.form { display: flex; flex-direction: column; gap: 0.5rem; background: #1a1a1a; padding: 1rem; border-radius: 0.5rem; }
label { display: flex; flex-direction: column; gap: 0.2rem; font-size: 0.8rem; color: #a0a0a0; }
input, select { background: #111; color: #e0e0e0; border: 1px solid #2a2a2a; border-radius: 0.25rem; padding: 0.35rem 0.5rem; font-size: 0.875rem; }
button { background: #2a4a7a; color: #e0e0e0; border: none; border-radius: 0.375rem; padding: 0.4rem 0.8rem; cursor: pointer; font-size: 0.85rem; align-self: flex-start; }
</style>
