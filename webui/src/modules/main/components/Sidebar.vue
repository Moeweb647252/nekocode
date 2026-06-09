<script setup lang="ts">
import { createThread, getDirs, listThreads } from "@/api";
import type { Thread } from "@/api/types";
import { useDialog } from "primevue";
import PickFloder from "./PickFolder.vue";

const dialog = useDialog();

const threads: Ref<Thread[]> = ref([]);
const homeDir = ref();
const selectedThread = inject<Ref<Thread>>("selectedThread");

onMounted(async () => {
  threads.value = await listThreads();
});

const newThread = async () => {
  if (!homeDir.value) {
    homeDir.value = (await getDirs()).homeDir;
  }
  dialog.open(PickFloder, {
    props: {
      header: "Select a working directory",
    },
    data: {
      path: homeDir.value,
    },
    onClose: async (data) => {
      if (!data) return;
      await createThread(data.data);
      threads.value = await listThreads();
    },
  });
};
</script>
<template>
  <div class="h-full sidebar">
    <div class="grid grid-rows-[auto_1fr] h-full overflow-hidden">
      <div class="">
        <li>
          <ul>
            <Button label="New Thread" variant="text" size="small" @click="newThread()" />
          </ul>
        </li>
      </div>
      <div class="overflow-hidden">
        <Listbox
          :options="threads"
          option-label="name"
          v-model="selectedThread"
          class="overflow-hidden border-none!"
          style="background: none"
        >
          <template #option="{ option }">
            <span class="select-none text-nowrap text-ellipsis overflow-hidden w-full"
              ><div v-if="option.title">{{ option.title }}</div>
              <div v-else>{{ option.workingDirectory }}</div></span
            >
          </template>
        </Listbox>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sidebar {
  background-color: var(--p-primary-contrast-color-100);
}
</style>
