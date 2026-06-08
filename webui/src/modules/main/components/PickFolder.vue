<script setup lang="ts">
import { listDir, type ListDirEntry } from "@/api";
import type { DynamicDialogInstance } from "primevue/dynamicdialogoptions";

const currentPath = ref();

const dialogRef = inject<Ref<DynamicDialogInstance>>("dialogRef");
const selected = ref();
const entries: Ref<ListDirEntry[]> = ref([]);
const isOkDisabled = computed(() => {
  return !selected.value || !selected.value.isDir;
});

onMounted(async () => {
  currentPath.value = dialogRef?.value.data.path;
  entries.value = await listDir(currentPath.value + "/");
});

const enterDir = async (entry: ListDirEntry) => {
  if (!entry.isDir) return;
  currentPath.value = `${currentPath.value}/${entry.name}`;
  entries.value = await listDir(currentPath.value + "/");
};

const closeDialog = () => {
  dialogRef?.value.close(`${currentPath.value}/${selected.value.name}`);
};

const goUp = async () => {
  if (currentPath.value === "/") return;
  const parts = currentPath.value.split("/");
  parts.pop();
  currentPath.value = parts.join("/") || "/";
  entries.value = await listDir(currentPath.value + "/");
};
</script>
<template>
  <Listbox :options="entries" v-model="selected">
    <template #option="{ option }">
      <div class="flex items-center gap-2" @dblclick="enterDir(option)">
        <i :class="option.isDir ? 'pi pi-folder' : 'pi pi-file'"></i>
        <span class="select-none text-nowrap">{{ option.name }}</span>
      </div>
    </template>
  </Listbox>
  <div class="flex justify-end">
    <Button label=".." @click="goUp" class="mt-2 mr-2" />
    <Button label="OK" @click="closeDialog" class="mt-2" :disabled="isOkDisabled" />
  </div>
</template>
