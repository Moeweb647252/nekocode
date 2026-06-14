<script setup lang="ts">
import type { AgentEvent, ChatMessage, GetThreadResponse, Thread } from "@/api/types.ts";
import InputArea from "./InputArea.vue";
import MessageBox from "./MessageBox.vue";
import { activateThread, getThread } from "@/api/thread.ts";
import { streamGenerate } from "@/api/generate.ts";

const selectedThread = inject<Ref<Thread>>("selectedThread");
const thread = ref<GetThreadResponse | null>(null);
const streamingMessages = ref<ChatMessage[]>([]);
const messages = computed(() => {
  const dbMessages =
    thread.value?.turns.flatMap((t) => t.messages?.map((m) => m.content) ?? []) ?? [];
  return [...dbMessages, ...streamingMessages.value];
});
const userInput = ref("");
const sending = ref(false);

// Track whether this component instance is still mounted so async callbacks
// from getThread/activateThread don't write into stale state after the user
// switches threads (the parent remounts via :key).
let alive = true;
// Cleanup function for the in-flight WebSocket; invoked on unmount so the
// socket (and its onDelta closures) don't outlive this component.
let closeStream: (() => void) | null = null;

onMounted(async () => {
  const id = selectedThread?.value?.id;
  if (id == null) return;
  try {
    const got = await getThread(id, 10);
    if (!alive) return; // user switched threads while loading
    thread.value = got;
    if (!got.active) await activateThread(got.id);
  } catch (e) {
    console.error("Failed to load thread:", e);
  }
});

onBeforeUnmount(() => {
  alive = false;
  closeStream?.();
  closeStream = null;
});

const sendMessage = async () => {
  const id = thread.value?.id;
  if (id == null) return;
  const input = userInput.value.trim();
  if (!input || sending.value) return;
  sending.value = true;
  userInput.value = "";

  // Append the user message (don't wipe prior streaming state).
  streamingMessages.value.push({ type: "user", data: { type: "text", content: input } });

  // The assistant message currently being built (a direct reference into the
  // streaming array so deltas mutate the same reactive object).
  let currentAssistantMsg: ChatMessage | null = null;

  closeStream?.();
  closeStream = streamGenerate(id, input, {
    onDelta: (event: AgentEvent) => {
      if (!alive) return;
      const se = event.data;
      if (se.type !== "streamEvent") return;
      const d = se.data;

      switch (d.type) {
        case "messageStart": {
          const newMsg: ChatMessage = {
            type: "assistant",
            data: { blocks: reactive([]) },
          };
          streamingMessages.value.push(newMsg);
          currentAssistantMsg = newMsg;
          break;
        }
        case "content": {
          if (!currentAssistantMsg) return;
          if (currentAssistantMsg.type !== "assistant") return;
          const blocks = currentAssistantMsg.data.blocks;
          let lastBlock = blocks[blocks.length - 1];
          if (!lastBlock || lastBlock.type !== "text") {
            lastBlock = { type: "text", content: "", reasoningContent: null };
            blocks.push(lastBlock);
          }
          lastBlock.content += d.data;
          break;
        }
        case "reasoningContent": {
          if (!currentAssistantMsg) return;
          if (currentAssistantMsg.type !== "assistant") return;
          const blocks = currentAssistantMsg.data.blocks;
          let lastBlock = blocks[blocks.length - 1];
          if (!lastBlock || lastBlock.type !== "text") {
            lastBlock = { type: "text", content: "", reasoningContent: null };
            blocks.push(lastBlock);
          }
          lastBlock.reasoningContent = (lastBlock.reasoningContent ?? "") + d.data;
          break;
        }
        case "toolCall": {
          if (!currentAssistantMsg) return;
          if (currentAssistantMsg.type !== "assistant") return;
          currentAssistantMsg.data.blocks.push({
            type: "toolCall",
            ...d.data,
          });
          break;
        }
        case "toolCallResult": {
          streamingMessages.value.push({
            type: "toolCallResult",
            data: d.data,
          });
          break;
        }
        case "messageEnd": {
          currentAssistantMsg = null;
          break;
        }
      }
    },
    onStop: () => {
      sending.value = false;
    },
    onError: (err) => {
      sending.value = false;
      console.error(err);
    },
  });
};
</script>
<template>
  <Splitter layout="vertical" style="height: 100%">
    <SplitterPanel>
      <MessageBox :messages="messages"></MessageBox>
    </SplitterPanel>
    <SplitterPanel>
      <InputArea v-model:value="userInput" :disabled="sending" @sendClicked="sendMessage"></InputArea>
    </SplitterPanel>
  </Splitter>
</template>
