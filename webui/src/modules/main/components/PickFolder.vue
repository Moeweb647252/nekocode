<script setup lang="ts">
import { listDir, type ListDirEntry } from "@/api";
import type { DynamicDialogInstance } from "primevue/dynamicdialogoptions";

const dialogRef = inject<Ref<DynamicDialogInstance>>("dialogRef");
const currentPath = ref("");
const selected = ref();
const entries: Ref<ListDirEntry[]> = ref([]);
const isOkDisabled = computed(() => {
  return !selected.value || !selected.value.isDir;
});

// Join path segments, collapsing duplicate slashes so navigating from "/"
// doesn't produce "//segment".
function joinPath(base: string, name: string): string {
  if (base.endsWith("/")) return base + name;
  return base + "/" + name;
}

async function load(path: string) {
  try {
    entries.value = await listDir(path);
  } catch (e) {
    console.error("Failed to list directory:", e);
    entries.value = [];
  }
}

onMounted(async () => {
  const data = dialogRef?.value?.data;
  currentPath.value = (data && (data as { path?: string }).path) || "/";
  await load(currentPath.value);
});

const enterDir = async (entry: ListDirEntry) => {
  if (!entry.isDir) return;
  currentPath.value = joinPath(currentPath.value, entry.name);
  await load(currentPath.value);
};

const closeDialog = () => {
  const path = selected.value
    ? joinPath(currentPath.value, selected.value.name)
    : currentPath.value;
  dialogRef?.value?.close(path);
};

const goUp = async () => {
  if (currentPath.value === "/") return;
  const parts = currentPath.value.split("/").filter(Boolean);
  parts.pop();
  currentPath.value = "/" + parts.join("/");
  await load(currentPath.value);
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
