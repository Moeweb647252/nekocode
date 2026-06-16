<script setup lang="ts">
import type { AgentEvent, ChatMessage, GetThreadResponse, Thread } from "@/api/types.ts";
import InputArea from "./InputArea.vue";
import MessageBox from "./MessageBox.vue";
import ThreadSettingsDialog from "./ThreadSettingsDialog.vue";
import { activateThread, getThread } from "@/api/thread.ts";
import { streamGenerate } from "@/api/generate.ts";
import { useDialog } from 'primevue';

const dialog = useDialog();

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

// Model badge for the thread sub-header. Only the Thread summary carries a
// model field; the loaded GetThreadResponse detail does not.
const displayModel = computed(() => selectedThread?.value?.model ?? '')

// Working directory is the project context — the headline of the header.
const displayWorkdir = computed(() => {
  const wd = thread.value?.workingDirectory ?? selectedThread?.value?.workingDirectory
  if (!wd) return 'untitled'
  // Shorten to the basename + a leading ellipsis when very long, but keep the
  // full path in the title attribute for hover.
  return wd
})

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
          // Closes the current assistant bubble only; the turn may continue
          // with another tool round. TurnEnd is what ends the whole turn.
          currentAssistantMsg = null;
          break;
        }
        case "turnEnd": {
          // The agent finished the whole turn (all tool rounds done).
          // Release the sending state; the trailing ws Stop frame will
          // arrive next as a backstop for the interrupted/error paths.
          currentAssistantMsg = null;
          sending.value = false;
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

const openSettings = () => {
  const id = thread.value?.id;
  if (id == null) return;
  dialog.open(ThreadSettingsDialog, {
    props: {
      header: 'Thread Settings',
      modal: true,
    },
    data: { threadId: id },
    onClose: (changed: unknown) => {
      if (changed === true) {
        // Reload thread to refresh title/model in the sub-header.
        getThread(id).then(t => { if (alive) thread.value = t; }).catch(console.error);
      }
    },
  });
}
</script>
<template>
  <div class="h-full grid grid-rows-[auto_1fr_auto] min-h-0">
    <!-- Thread sub-header: project context (working dir) is the headline -->
    <div
      class="flex items-center gap-2.5 px-4 py-2 border-b border-solid"
      style="border-color: var(--app-border); background: var(--app-surface)"
    >
      <i class="pi pi-folder text-sm" style="color: var(--p-primary-500)" />
      <span class="text-sm font-mono truncate" :title="displayWorkdir">{{
        displayWorkdir
      }}</span>
      <span
        v-if="thread?.title"
        class="text-xs truncate"
        style="color: var(--app-text-muted)"
        >— {{ thread.title }}</span
      >
      <span
        v-if="displayModel"
        class="ml-1 text-xs px-2 py-0.5 rounded font-mono"
        style="
          background: color-mix(in srgb, var(--p-primary-500) 12%, transparent);
          color: var(--p-primary-700);
        "
        >{{ displayModel }}</span
      >
      <span
        v-if="sending"
        class="ml-auto inline-flex items-center gap-1.5 text-xs"
        style="color: var(--app-text-muted)"
      >
        <span class="dot-pulse"></span>
        working
      </span>
    </div>

    <!-- Messages -->
    <div class="min-h-0">
      <MessageBox :messages="messages" :generating="sending"></MessageBox>
    </div>

    <!-- Input -->
    <InputArea
      v-model:value="userInput"
      :disabled="sending"
      @sendClicked="sendMessage"
      @settingsClicked="openSettings"
    ></InputArea>
  </div>
</template>

<style scoped>
/* Animated "generating" indicator. */
.dot-pulse {
  width: 7px;
  height: 7px;
  border-radius: 9999px;
  background: var(--p-primary-500);
  display: inline-block;
  animation: nekocode-pulse 1s ease-in-out infinite;
}
@keyframes nekocode-pulse {
  0%,
  100% {
    opacity: 0.35;
    transform: scale(0.85);
  }
  50% {
    opacity: 1;
    transform: scale(1.15);
  }
}
</style>
