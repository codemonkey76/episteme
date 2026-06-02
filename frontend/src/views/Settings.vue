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
  <div class="p-5 max-w-[40rem] flex flex-col gap-8">
    <section class="flex flex-col gap-3">
      <h2 class="text-base border-b border-raised pb-[0.4rem]">Model providers</h2>
      <ul v-if="providers.length" class="list-none flex flex-col gap-[0.4rem]">
        <li v-for="p in providers" :key="p.name" class="flex justify-between items-center bg-surface rounded-md px-3 py-2 text-sm">
          <span>{{ p.name }} — {{ p.provider }} / {{ p.model_id }}</span>
          <button @click="deleteProvider(p.name)" class="bg-[#2a4a7a] text-fg border-none rounded-md px-[0.8rem] py-[0.4rem] cursor-pointer text-[0.85rem] self-start">Remove</button>
        </li>
      </ul>
      <p v-else class="text-[#606060] text-sm">No providers configured.</p>

      <form @submit.prevent="saveProvider" class="flex flex-col gap-2 bg-surface p-4 rounded-lg">
        <h3 class="text-[0.9rem] text-[#a0a0a0]">Add / update provider</h3>
        <label class="flex flex-col gap-[0.2rem] text-[0.8rem] text-[#a0a0a0]">Name <input v-model="newProvider.name" required class="bg-[#111] text-fg border border-raised rounded px-2 py-[0.35rem] text-sm" /></label>
        <label class="flex flex-col gap-[0.2rem] text-[0.8rem] text-[#a0a0a0]">Provider
          <select v-model="newProvider.provider" class="bg-[#111] text-fg border border-raised rounded px-2 py-[0.35rem] text-sm">
            <option>anthropic</option>
            <option>openai</option>
            <option>ollama</option>
            <option>openai_compatible</option>
          </select>
        </label>
        <label class="flex flex-col gap-[0.2rem] text-[0.8rem] text-[#a0a0a0]">Model ID <input v-model="newProvider.model_id" required class="bg-[#111] text-fg border border-raised rounded px-2 py-[0.35rem] text-sm" /></label>
        <label class="flex flex-col gap-[0.2rem] text-[0.8rem] text-[#a0a0a0]">Base URL <input v-model="newProvider.base_url" class="bg-[#111] text-fg border border-raised rounded px-2 py-[0.35rem] text-sm" /></label>
        <label class="flex flex-col gap-[0.2rem] text-[0.8rem] text-[#a0a0a0]">API Key <input v-model="newProvider.api_key" type="password" class="bg-[#111] text-fg border border-raised rounded px-2 py-[0.35rem] text-sm" /></label>
        <button type="submit" class="bg-[#2a4a7a] text-fg border-none rounded-md px-[0.8rem] py-[0.4rem] cursor-pointer text-[0.85rem] self-start">Save</button>
      </form>
    </section>

    <section class="flex flex-col gap-3">
      <h2 class="text-base border-b border-raised pb-[0.4rem]">MCP servers</h2>
      <ul v-if="mcpServers.length" class="list-none flex flex-col gap-[0.4rem]">
        <li v-for="s in mcpServers" :key="s.name" class="flex justify-between items-center bg-surface rounded-md px-3 py-2 text-sm">
          <span>{{ s.name }}</span>
        </li>
      </ul>
      <p v-else class="text-[#606060] text-sm">No MCP servers configured.</p>
    </section>
  </div>
</template>
