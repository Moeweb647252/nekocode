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

onMounted(async () => {
  thread.value = await getThread(selectedThread?.value.id!, 10);
  if (!thread.value.active) {
    await activateThread(thread.value.id);
  }
});

const sendMessage = async () => {
  const input = userInput.value.trim();
  if (!input) return;
  userInput.value = "";

  // 立即追加用户消息
  streamingMessages.value = [{ type: "user", data: { type: "text", content: input } }];

  // 当前正在构建的 assistant 消息（直接引用 streamingMessages 中的最后一条）
  let currentAssistantMsg: ChatMessage | null = null;

  streamGenerate(thread.value!.id, input, {
    onDelta: (event: AgentEvent) => {
      const se = event.data;
      if (se.type !== "streamEvent") return;
      const d = se.data;

      switch (d.type) {
        case "messageStart": {
          // 创建新的 assistant 消息并推入列表
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
          // 确保最后一个 block 是文本块（可追加）
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
          // 当前消息已完整，清空引用即可
          currentAssistantMsg = null;
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
