<script setup lang="ts">
import { listDir, type ListDirEntry } from '@/api'
import type { DynamicDialogInstance } from 'primevue/dynamicdialogoptions'

const dialogRef = inject<Ref<DynamicDialogInstance>>('dialogRef')
const currentPath = ref('')
const selected = ref()
const entries: Ref<ListDirEntry[]> = ref([])
const isOkDisabled = computed(() => {
  return !selected.value || !selected.value.isDir
})

// Join path segments, collapsing duplicate slashes so navigating from "/"
// doesn't produce "//segment".
function joinPath(base: string, name: string): string {
  if (base.endsWith('/')) return base + name
  return base + '/' + name
}

async function load(path: string) {
  try {
    entries.value = await listDir(path)
  } catch (e) {
    console.error('Failed to list directory:', e)
    entries.value = []
  }
}

onMounted(async () => {
  const data = dialogRef?.value?.data
  currentPath.value = (data && (data as { path?: string }).path) || '/'
  await load(currentPath.value)
})

const enterDir = async (entry: ListDirEntry) => {
  if (!entry.isDir) return
  currentPath.value = joinPath(currentPath.value, entry.name)
  await load(currentPath.value)
}

const closeDialog = () => {
  const path = selected.value
    ? joinPath(currentPath.value, selected.value.name)
    : currentPath.value
  dialogRef?.value?.close(path)
}

const goUp = async () => {
  if (currentPath.value === '/') return
  const parts = currentPath.value.split('/').filter(Boolean)
  parts.pop()
  currentPath.value = '/' + parts.join('/')
  await load(currentPath.value)
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <!-- Path breadcrumb -->
    <div
      class="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-mono break-all"
      style="
        background: var(--p-surface-100);
        color: var(--app-text-muted);
      "
    >
      <i class="pi pi-folder-open"></i>
      <span>{{ currentPath || '/' }}</span>
    </div>

    <Listbox
      :options="entries"
      v-model="selected"
      class="folder-list"
      :pt="{
        list: { style: 'max-height: 340px' },
      }"
      @option-dblclick="(e) => enterDir(e.value)"
    >
      <template #option="{ option }">
        <div class="flex items-center gap-2.5">
          <i
            class="pi"
            :class="option.isDir ? 'pi-folder' : 'pi-file'"
            :style="option.isDir ? 'color: var(--p-primary-500)' : 'color: var(--app-text-muted)'"
          ></i>
          <span class="select-none text-nowrap">{{ option.name }}</span>
        </div>
      </template>
      <template #empty>
        <div class="px-2 py-6 text-center text-sm" style="color: var(--app-text-muted)">
          Empty directory.
        </div>
      </template>
    </Listbox>

    <div class="flex items-center justify-between">
      <Button
        icon="pi pi-arrow-up"
        variant="text"
        severity="secondary"
        size="small"
        title="Go up"
        :disabled="currentPath === '/'"
        @click="goUp"
      />
      <div class="flex gap-2">
        <Button
          label="Cancel"
          variant="text"
          severity="secondary"
          size="small"
          @click="dialogRef?.close()"
        />
        <Button label="OK" size="small" :disabled="isOkDisabled" @click="closeDialog" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.folder-list {
  border-radius: 10px;
}
:deep(.p-listbox-option) {
  padding: 10px 12px;
  min-height: 40px;
  border-radius: 8px;
  margin: 1px 2px;
}
:deep(.p-listbox-option:hover) {
  background: var(--p-surface-100);
}
.app-dark :deep(.p-listbox-option:hover) {
  background: var(--p-surface-800);
}
:deep(.p-listbox-option.p-listbox-option-selected) {
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
}
</style>
