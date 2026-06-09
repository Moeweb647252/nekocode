<script setup lang="ts">
import type { ChatMessage } from "@/api";
const props = defineProps({
  messages: {
    type: Array as () => ChatMessage[],
    required: true,
  },
});
</script>
<template>
  <div class="w-full h-full overflow-scroll p-2">
    <div v-for="item in props.messages" :key="item.type">
      <div v-if="item.type === 'user'" class="flex justify-end">
        {{ item.data.content }}
      </div>
      <div v-else-if="item.type === 'assistant'">
        <div v-for="(part, index) in item.data.blocks" :key="index">
          <div v-if="part.type === 'text'">{{ part.content }}</div>
        </div>
      </div>
    </div>
  </div>
</template>
