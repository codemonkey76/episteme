<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  url: string
  name: string
  contentType: string
}>()

const kind = computed<'image' | 'pdf' | 'embed'>(() => {
  const t = props.contentType.toLowerCase()
  if (t.startsWith('image/')) return 'image'
  if (t === 'application/pdf') return 'pdf'
  // text/html/json etc. render fine in an iframe; binary types fall back to the
  // browser's own handling (usually a download), which is the sensible default.
  return 'embed'
})
</script>

<template>
  <div class="flex flex-col h-full bg-[var(--c-1a1a1a)]">
    <!-- Toolbar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--c-2a2a2a)] shrink-0 bg-[var(--c-202020)]">
      <span class="text-[0.78rem] text-[var(--c-c0c0c0)] truncate flex-1 min-w-0" :title="name">{{ name }}</span>
      <span class="text-[0.68rem] text-[var(--c-606060)] shrink-0">{{ contentType }}</span>
      <a
        :href="url"
        :download="name"
        class="flex items-center gap-[0.35rem] bg-[var(--c-1e3a6e)] text-[var(--c-7ab0ff)] border border-[var(--c-2a4a8a)] rounded px-2.5 py-1 text-[0.75rem] no-underline cursor-pointer transition-colors duration-100 hover:bg-[var(--c-254880)] shrink-0"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>
        </svg>
        Download
      </a>
    </div>

    <!-- Content -->
    <div class="flex-1 min-h-0 overflow-auto flex items-center justify-center">
      <img v-if="kind === 'image'" :src="url" :alt="name" class="max-w-full max-h-full object-contain" />
      <iframe
        v-else
        :src="url"
        class="w-full h-full border-none bg-white"
        :title="name"
      />
    </div>
  </div>
</template>
