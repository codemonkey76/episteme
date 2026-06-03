<script setup lang="ts">
import AuthGate from './components/AuthGate.vue'
import Workspace from './components/Workspace.vue'
</script>

<template>
  <AuthGate>
    <Workspace />
  </AuthGate>
</template>
