<script setup lang="ts">
import { onMounted, ref, computed } from 'vue'
import { useAuthStore } from '../stores/auth'

const auth = useAuthStore()

const username = ref('')
const password = ref('')
const confirm = ref('')
const error = ref('')
const busy = ref(false)

const mode = computed(() => (auth.setupRequired ? 'setup' : 'login'))

onMounted(async () => {
  auth.register()
  try {
    await auth.refresh()
  } catch {
    // Network/backend down — show the login form rather than a blank page.
    auth.loaded = true
  }
})

async function submit() {
  error.value = ''
  if (mode.value === 'setup') {
    if (password.value.length < 8) {
      error.value = 'Password must be at least 8 characters.'
      return
    }
    if (password.value !== confirm.value) {
      error.value = 'Passwords do not match.'
      return
    }
  }
  busy.value = true
  try {
    if (mode.value === 'setup') {
      await auth.setup(username.value.trim(), password.value)
    } else {
      await auth.login(username.value.trim(), password.value)
    }
    password.value = ''
    confirm.value = ''
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : 'Something went wrong.'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <!-- Authenticated: render the app. -->
  <slot v-if="auth.loaded && auth.authenticated" />

  <!-- Initial status probe. -->
  <div v-else-if="!auth.loaded" class="flex h-screen items-center justify-center bg-[#0c0c0c]">
    <div class="text-[0.8rem] text-[#585858]">Loading…</div>
  </div>

  <!-- Setup or login. -->
  <div v-else class="flex h-screen items-center justify-center bg-[#0c0c0c] p-6">
    <form
      class="flex w-full max-w-[340px] flex-col gap-4 rounded-xl border border-[#222] bg-[#111] p-7"
      @submit.prevent="submit"
    >
      <div class="flex flex-col gap-1">
        <h1 class="text-[1.05rem] font-semibold text-fg">
          {{ mode === 'setup' ? 'Create your account' : 'Sign in' }}
        </h1>
        <p class="text-[0.78rem] text-[#707070]">
          {{ mode === 'setup'
            ? 'Set the admin username and password for this Episteme instance.'
            : 'Enter your credentials to continue.' }}
        </p>
      </div>

      <label class="flex flex-col gap-1 text-[0.775rem] text-muted">
        Username
        <input
          v-model="username"
          type="text"
          autocomplete="username"
          required
          class="rounded border border-raised bg-surface px-2.5 py-2 text-[0.85rem] text-fg focus:border-[#3a6adf] focus:outline-none"
        />
      </label>

      <label class="flex flex-col gap-1 text-[0.775rem] text-muted">
        Password
        <input
          v-model="password"
          type="password"
          :autocomplete="mode === 'setup' ? 'new-password' : 'current-password'"
          required
          class="rounded border border-raised bg-surface px-2.5 py-2 text-[0.85rem] text-fg focus:border-[#3a6adf] focus:outline-none"
        />
      </label>

      <label v-if="mode === 'setup'" class="flex flex-col gap-1 text-[0.775rem] text-muted">
        Confirm password
        <input
          v-model="confirm"
          type="password"
          autocomplete="new-password"
          required
          class="rounded border border-raised bg-surface px-2.5 py-2 text-[0.85rem] text-fg focus:border-[#3a6adf] focus:outline-none"
        />
      </label>

      <p v-if="error" class="text-[0.775rem] text-[#ff7070]">{{ error }}</p>

      <button
        type="submit"
        :disabled="busy"
        class="mt-1 rounded-md border border-[#2a4a8a] bg-[#1e3a6e] px-3 py-2 text-[0.85rem] text-[#7ab0ff] transition-[background] duration-[120ms] hover:bg-[#254880] disabled:cursor-not-allowed disabled:opacity-60"
      >
        {{ busy ? 'Please wait…' : mode === 'setup' ? 'Create account' : 'Sign in' }}
      </button>
    </form>
  </div>
</template>
