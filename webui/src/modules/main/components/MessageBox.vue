<script setup lang="ts">
import type { ChatMessage, ToolCallResultInner } from "@/api";
const props = defineProps({
  messages: {
    type: Array as () => ChatMessage[],
    required: true,
  },
});
// Index tool-call results by id so we can look up the result for each
// toolCall block. Built from the full message list (includes streamed results).
const toolResults = computed(() => {
  const results: Record<string, ToolCallResultInner> = {};
  for (const msg of props.messages) {
    if (msg.type === "toolCallResult") {
      results[msg.data.id] = msg.data.result;
    }
  }
  return results;
});

// Render a tool call's argument summary: prefer a "command" field, otherwise
// show the JSON-serialized args. `args` is typed `unknown` so we narrow it.
function argsSummary(args: unknown): string {
  if (args && typeof args === "object" && "command" in args) {
    return String((args as { command: unknown }).command);
  }
  return JSON.stringify(args);
}

// Stringify a tool result's payload for display.
function resultText(result: ToolCallResultInner | undefined): string | null {
  if (!result) return null;
  if (result.type === "success") {
    return typeof result.success === "string"
      ? result.success
      : JSON.stringify(result.success);
  }
  return `Error: ${result.error}`;
}
</script>
<template>
  <div class="w-full h-full overflow-scroll p-2">
    <div v-for="(item, index) in messages" :key="index">
      <div v-if="item.type === 'user'" class="flex justify-end mb-2 mr-2">
        {{ item.data.content }}
      </div>
      <!-- Assistant -->
      <div v-else-if="item.type === 'assistant'">
        <div v-for="(block, bi) in item.data.blocks" :key="bi">
          <div v-if="block.type === 'text'">
            <div v-if="block.reasoningContent" class="mb-2">
              <Panel header="Thinking" toggleable :collapsed="true">
                <p class="m-0 whitespace-pre-wrap">
                  {{ block.reasoningContent }}
                </p>
              </Panel>
            </div>
            <div v-if="block.content" class="mb-2 ml-2 whitespace-pre-wrap">
              {{ block.content }}
            </div>
          </div>
          <div v-else-if="block.type === 'toolCall'" class="mb-2">
            <Panel
              :header="`${block.name}(${argsSummary(block.args)})`"
              toggleable
              :collapsed="true"
            >
              <p class="m-0 whitespace-pre-wrap">
                {{ resultText(toolResults[block.id]) ?? "Loading..." }}
              </p>
            </Panel>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
