<script setup lang="ts">
import { DynamicDialog } from 'primevue'
import AppHeader from '@/components/AppHeader.vue'
</script>

<template>
  <DynamicDialog />
  <div class="h-full flex flex-col">
    <AppHeader />
    <main class="flex-1 min-h-0 overflow-hidden">
      <RouterView />
    </main>
  </div>
</template>

<style scoped></style>
