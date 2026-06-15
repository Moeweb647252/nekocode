<script setup lang="ts">
import { createThread, getDirs, listThreads } from '@/api'
import type { Thread } from '@/api/types'
import { useDialog } from 'primevue'
import PickFolder from './PickFolder.vue'

const dialog = useDialog()

const threads: Ref<Thread[]> = ref([])
const homeDir = ref()
const selectedThread = inject<Ref<Thread>>('selectedThread')

onMounted(async () => {
  try {
    threads.value = await listThreads()
  } catch (e) {
    console.error('Failed to list threads:', e)
  }
})

// Working directory basename — the project context, shown as the primary
// label (coding agents are organized around projects, not chat titles).
function workdirBasename(t: Thread): string {
  const wd = t.workingDirectory ?? ''
  const parts = wd.replace(/\\/g, '/').split('/').filter(Boolean)
  return parts[parts.length - 1] || wd || 'untitled'
}

// Relative-ish timestamp (e.g. "2h ago"). Best-effort; falls back to date.
function timeAgo(iso: string): string {
  const then = new Date(iso).getTime()
  if (Number.isNaN(then)) return ''
  const diff = Date.now() - then
  const min = Math.round(diff / 60000)
  if (min < 1) return 'just now'
  if (min < 60) return `${min}m ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  if (day < 7) return `${day}d ago`
  return new Date(iso).toLocaleDateString()
}

const newThread = async () => {
  try {
    if (!homeDir.value) {
      homeDir.value = (await getDirs()).homeDir
    }
    dialog.open(PickFolder, {
      props: {
        header: 'Select a working directory',
        modal: true,
      },
      data: {
        path: homeDir.value,
      },
      onClose: async (data: unknown) => {
        if (!data) return
        // The dialog returns the chosen path as a plain string.
        const path = typeof data === 'string' ? data : (data as { data?: string }).data
        if (!path) return
        try {
          await createThread(path)
          threads.value = await listThreads()
        } catch (e) {
          console.error('Failed to create thread:', e)
        }
      },
    })
  } catch (e) {
    console.error('Failed to open new-thread dialog:', e)
  }
}
</script>

<template>
  <div class="h-full grid grid-rows-[auto_1fr] overflow-hidden">
    <!-- Header row -->
    <div class="px-3 py-3">
      <Button label="New Thread" icon="pi pi-plus" class="w-full" @click="newThread()" />
      <div class="mt-3 mb-1 flex items-center justify-between px-1">
        <span class="text-xs font-medium uppercase tracking-wide" style="color: var(--app-text-muted)">
          Threads
        </span>
        <span class="text-xs" style="color: var(--app-text-muted)">{{ threads.length }}</span>
      </div>
    </div>

    <!-- List -->
    <div class="overflow-hidden">
      <Listbox
        :options="threads"
        v-model="selectedThread"
        optionLabel="id"
        class="thread-list h-full overflow-auto border-none!"
        style="background: none"
        :pt="{
          list: { style: 'background: none' },
          option: { class: 'thread-option' },
        }"
      >
        <template #option="{ option }">
          <div class="flex items-start gap-2 w-full min-w-0 py-0.5">
            <i
              class="pi pi-folder mt-1 text-sm"
              style="color: var(--p-primary-500); opacity: 0.8"
            />
            <div class="min-w-0 flex-1">
              <div
                class="text-nowrap text-ellipsis overflow-hidden text-sm font-medium font-mono"
                :title="option.workingDirectory"
              >
                {{ workdirBasename(option) }}
              </div>
              <div
                v-if="option.title"
                class="text-nowrap text-ellipsis overflow-hidden text-xs mt-0.5"
                style="color: var(--app-text-muted)"
              >
                {{ option.title }}
              </div>
              <div
                v-if="timeAgo(option.updatedAt)"
                class="text-xs mt-0.5"
                style="color: var(--app-text-muted)"
              >
                {{ timeAgo(option.updatedAt) }}
              </div>
            </div>
          </div>
        </template>
        <template #empty>
          <div class="px-2 py-6 text-center text-sm" style="color: var(--app-text-muted)">
            No threads yet.
          </div>
        </template>
      </Listbox>
    </div>
  </div>
</template>

<style scoped>
:deep(.thread-list) {
  background: none;
}
:deep(.p-listbox-list-container) {
  background: none;
}
/* Rounded, airy option rows with a soft hover/selected state. */
:deep(.p-listbox-option) {
  margin: 2px 6px;
  border-radius: 10px;
  padding: 6px 10px;
  border: 1px solid transparent;
  transition: background-color 0.12s ease;
}
:deep(.p-listbox-option:hover) {
  background: var(--p-surface-100);
}
.app-dark :deep(.p-listbox-option:hover) {
  background: var(--p-surface-800);
}
:deep(.p-listbox-option.p-listbox-option-selected) {
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
  border-color: color-mix(in srgb, var(--p-primary-500) 30%, transparent);
}
:deep(.p-listbox-option.p-listbox-option-selected:hover) {
  background: color-mix(in srgb, var(--p-primary-500) 20%, transparent);
}
</style>
