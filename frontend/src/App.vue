<script setup lang="ts">
import AuthGate from './components/AuthGate.vue'
import Workspace from './components/Workspace.vue'
import { useAuthStore } from './stores/auth'

const auth = useAuthStore()
</script>

<template>
  <AuthGate>
    <!-- Impersonation banner: a loud reminder that actions run as the member. -->
    <div
      v-if="auth.impersonator"
      class="flex items-center justify-center gap-3 bg-[var(--c-3a2a10)] border-b border-[var(--c-5a4520)] text-[var(--c-e0b060)] text-[0.8rem] py-1.5 px-4"
    >
      <span>
        You ({{ auth.impersonator }}) are acting as <b>{{ auth.username }}</b> — everything you do happens in their account.
      </span>
      <button
        class="bg-[var(--c-2a2418)] text-[var(--c-e0b060)] border border-[var(--c-5a4520)] rounded px-2.5 py-[0.2rem] cursor-pointer text-[0.75rem] font-[inherit] hover:bg-[var(--c-3a3020)]"
        @click="auth.stopImpersonating()"
      >
        Return to my account
      </button>
    </div>
    <Workspace />
  </AuthGate>
</template>
