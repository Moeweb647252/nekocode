<script setup lang="ts">
import Sidebar from "./components/Sidebar.vue";
import ThreadPanel from "./components/ThreadPanel.vue";
const selectedThread = ref();
provide("selectedThread", selectedThread);
</script>
<template>
  <Splitter style="height: 100%">
    <SplitterPanel :size="20">
      <Sidebar />
    </SplitterPanel>
    <SplitterPanel :size="80">
      <div class="h-full flex items-center justify-center" v-if="!selectedThread">
        <h1 class="text-3xl text-gray-500">Select or create a thread to get started</h1>
      </div>
      <div v-else>
        <ThreadPanel :key="selectedThread.id" />
      </div>
    </SplitterPanel>
  </Splitter>
</template>
