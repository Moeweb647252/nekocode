<script setup lang="ts">
import type {
  AgentEvent,
  AssistantBlock,
  ChatMessage,
  GetThreadResponse,
  Thread,
  ToolCall,
} from "@/api/types.ts";
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

onMounted(async () => {
  thread.value = await getThread(selectedThread?.value.id!, 10);
  if (!thread.value.active) {
    await activateThread(thread.value.id);
  }
});

interface PendingAssistant {
  content: string;
  reasoning: string;
  toolCalls: ToolCall[];
}

function emptyPending(): PendingAssistant {
  return { content: "", reasoning: "", toolCalls: [] };
}

function finalizePending(pending: PendingAssistant): ChatMessage {
  const blocks: AssistantBlock[] = [];
  if (pending.content || pending.reasoning) {
    blocks.push({
      type: "text",
      content: pending.content,
      reasoningContent: pending.reasoning || null,
    });
  }
  for (const tc of pending.toolCalls) {
    blocks.push({ type: "toolCall", ...tc });
  }
  return { type: "assistant", data: { blocks } };
}

const sendMessage = async () => {
  const input = userInput.value.trim();
  if (!input) return;
  userInput.value = "";

  // Append the user message immediately.
  streamingMessages.value = [{ type: "user", data: { type: "text", content: input } }];

  let pending: PendingAssistant | null = null;

  streamGenerate(thread.value!.id, input, {
    onDelta: (event: AgentEvent) => {
      const se = event.data;
      if (se.type !== "streamEvent") return;
      const d = se.data;

      switch (d.type) {
        case "messageStart": {
          // Finalize any previous pending message before starting a new one.
          if (pending) {
            streamingMessages.value = [...streamingMessages.value, finalizePending(pending)];
          }
          pending = emptyPending();
          break;
        }
        case "content": {
          if (pending) pending.content += d.data;
          break;
        }
        case "reasoningContent": {
          if (pending) pending.reasoning += d.data;
          break;
        }
        case "toolCall": {
          if (pending) pending.toolCalls.push(d.data);
          break;
        }
        case "toolCallResult": {
          streamingMessages.value = [
            ...streamingMessages.value,
            { type: "toolCallResult", data: d.data },
          ];
          break;
        }
        case "messageEnd": {
          if (pending) {
            streamingMessages.value = [...streamingMessages.value, finalizePending(pending)];
            pending = null;
          }
          break;
        }
      }
    },
    onStop: () => {
      console.log("completed");
    },
    onError: (err) => {
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
      <InputArea v-model:value="userInput" @sendClicked="sendMessage"></InputArea>
    </SplitterPanel>
  </Splitter>
</template>
