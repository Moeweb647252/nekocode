<script setup lang="ts">
import Sidebar from './components/Sidebar.vue'
import ThreadPanel from './components/ThreadPanel.vue'
const selectedThread = ref()
provide('selectedThread', selectedThread)
</script>

<template>
  <Splitter style="height: 100%" :gutterSize="4" stateKey="nekocode.mainSplit" stateStorage="local">
    <SplitterPanel :size="22" :minSize="16" class="sidebar-panel">
      <Sidebar />
    </SplitterPanel>
    <SplitterPanel :size="78" :minSize="40">
      <!-- Empty state -->
      <div
        v-if="!selectedThread?.id"
        class="h-full flex flex-col items-center justify-center px-6 text-center"
      >
        <span
          class="mb-5 inline-flex items-center justify-center rounded-2xl text-white shadow-lg"
          style="background: var(--p-primary-500); width: 64px; height: 64px"
        >
          <svg width="34" height="34" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d="M4 6l2 3a8 8 0 0 1 12 0l2-3v6a6 6 0 0 1-6 6h-4a6 6 0 0 1-6-6V6z"
              fill="currentColor"
              opacity="0.95"
            />
            <circle cx="9.5" cy="11" r="1.1" fill="#0b1120" />
            <circle cx="14.5" cy="11" r="1.1" fill="#0b1120" />
          </svg>
        </span>
        <h1 class="text-2xl font-semibold tracking-tight mb-2">Select a project</h1>
        <p style="color: var(--app-text-muted)" class="max-w-sm">
          Pick a working directory to start a session, or create a new one from the sidebar.
        </p>
      </div>
      <div v-else class="h-full">
        <ThreadPanel :key="selectedThread.id" />
      </div>
    </SplitterPanel>
  </Splitter>
</template>

<style scoped>
.sidebar-panel {
  background: var(--app-surface);
  border-right: 1px solid var(--app-border);
}
/* Make the splitter gutter match the surface so it blends in. */
:deep(.p-splitter-gutter) {
  background: transparent;
}
:deep(.p-splitter-gutter-handle) {
  background: var(--app-border);
}
</style>
