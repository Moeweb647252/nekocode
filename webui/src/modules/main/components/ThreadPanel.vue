<script setup lang="ts">
import type { GetThreadResponse, Thread } from "@/api/types.ts";
import InputArea from "./InputArea.vue";
import MessageBox from "./MessageBox.vue";
import { activateThread, getThread } from "@/api/thread.ts";
import { streamGenerate } from "@/api/generate.ts";

const selectedThread = inject<Ref<Thread>>("selectedThread");
const thread = ref<GetThreadResponse | null>(null);

onMounted(async () => {
  thread.value = await getThread(selectedThread?.value.id!);
  if (!thread.value.active) {
    await activateThread(thread.value.id);
  }
});

const sendMessage = async () => {
  streamGenerate(thread.value!.id, "你好", {
    onDelta: (message) => {
      console.log(message);
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
      <MessageBox></MessageBox>
    </SplitterPanel>
    <SplitterPanel>
      <InputArea></InputArea>
      <Button label="Send" @click="sendMessage" />
    </SplitterPanel>
  </Splitter>
</template>
