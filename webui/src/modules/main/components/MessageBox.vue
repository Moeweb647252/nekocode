<!-- eslint-disable @typescript-eslint/no-explicit-any -->
<script setup lang="ts">
import type { ChatMessage } from "@/api";
const props = defineProps({
  messages: {
    type: Array as () => ChatMessage[],
    required: true,
  },
});
const toolResults = computed(() => {
  const results: Record<string, any> = {};
  props.messages.forEach((msg) => {
    if (msg.type === "toolCallResult") {
      results[msg.data.id] = msg.data.result;
    }
  });
  return results;
});
const { messages } = toRefs(props);
</script>
<template>
  <div class="w-full h-full overflow-scroll p-2">
    <div v-for="(item, index) in messages" :key="index">
      <div v-if="item.type === 'user'" class="flex justify-end mb-2 mr-2">
        {{ item.data.content }}
      </div>
      <!-- Assistant -->
      <div v-else-if="item.type === 'assistant'">
        <div v-for="(block, bi) in (item.data as { blocks: any[] }).blocks" :key="bi">
          <div v-if="block.type === 'text'">
            <div v-if="block.reasoningContent" class="mb-2">
              <Panel header="Thinking" toggleable :collapsed="true">
                <p class="m-0">
                  {{ block.reasoningContent }}
                </p>
              </Panel>
            </div>
            <div v-if="block.content" class="mb-2 ml-2">
              {{ block.content }}
            </div>
          </div>
          <div v-else-if="block.type === 'toolCall'" class="mb-2">
            <div v-if="block.name == 'shell'">
              <Panel :header="`Shell( ${block.args.command} )`" toggleable :collapsed="true">
                <p class="m-0">
                  {{ toolResults[block.id] || "Loading..." }}
                </p>
              </Panel>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
