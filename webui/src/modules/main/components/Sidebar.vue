<script setup lang="ts">
import { createThread, getDirs, listThreads } from "@/api";
import type { Thread } from "@/api/types";
import { useDialog } from "primevue";
import PickFolder from "./PickFolder.vue";

const dialog = useDialog();

const threads: Ref<Thread[]> = ref([]);
const homeDir = ref();
const selectedThread = inject<Ref<Thread>>("selectedThread");

onMounted(async () => {
  try {
    threads.value = await listThreads();
  } catch (e) {
    console.error("Failed to list threads:", e);
  }
});

const newThread = async () => {
  try {
    if (!homeDir.value) {
      homeDir.value = (await getDirs()).homeDir;
    }
    dialog.open(PickFolder, {
      props: {
        header: "Select a working directory",
      },
      data: {
        path: homeDir.value,
      },
      onClose: async (data: unknown) => {
        if (!data) return;
        // The dialog returns the chosen path as a plain string.
        const path = typeof data === "string" ? data : (data as { data?: string }).data;
        if (!path) return;
        try {
          await createThread(path);
          threads.value = await listThreads();
        } catch (e) {
          console.error("Failed to create thread:", e);
        }
      },
    });
  } catch (e) {
    console.error("Failed to open new-thread dialog:", e);
  }
};
</script>
<template>
  <div class="h-full sidebar">
    <div class="grid grid-rows-[auto_1fr] h-full overflow-hidden">
      <div class="">
        <Button label="New Thread" variant="text" size="small" @click="newThread()" />
      </div>
      <div class="overflow-hidden">
        <Listbox
          :options="threads"
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
