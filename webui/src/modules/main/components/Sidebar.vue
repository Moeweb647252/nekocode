<script setup lang="ts">
import { createThread, getDirs, listThreads } from '@/api'
import type { Thread } from '@/api/types'
import { useDialog } from 'primevue'
import PickFolder from './PickFolder.vue'

interface Workspace {
  dir: string
  name: string
  threads: Thread[]
  count: number
  lastUpdated: number
}

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

// Group threads by working directory (the "workspace"). Multiple threads can
// share one directory, so each workspace is a collapsible group whose children
// are the threads rooted there. Workspaces sort by most-recent activity; threads
// within a workspace sort newest-first.
const workspaces = computed<Workspace[]>(() => {
  const map = new Map<string, Thread[]>()
  for (const t of threads.value) {
    const arr = map.get(t.workingDirectory)
    if (arr) arr.push(t)
    else map.set(t.workingDirectory, [t])
  }
  const list: Workspace[] = []
  for (const [dir, items] of map) {
    const sorted = [...items].sort((a, b) => ts(b.updatedAt) - ts(a.updatedAt))
    list.push({
      dir,
      name: dirBasename(dir),
      threads: sorted,
      count: sorted.length,
      lastUpdated: sorted.length ? ts(sorted[0]!.updatedAt) : 0,
    })
  }
  list.sort((a, b) => b.lastUpdated - a.lastUpdated)
  return list
})

// id of the currently selected thread (or undefined) for row highlighting.
const selectedId = computed(() => selectedThread?.value?.id)

// Collapsed state keyed by working directory. Absent = expanded (default).
const collapsed = reactive<Record<string, boolean>>({})
function toggleWorkspace(dir: string) {
  collapsed[dir] = !collapsed[dir]
}
function isExpanded(dir: string) {
  return !collapsed[dir]
}
function selectThread(t: Thread) {
  if (!selectedThread) return
  selectedThread.value = t
  // Keep the selected thread visible: never leave its workspace collapsed.
  collapsed[t.workingDirectory] = false
}

function ts(iso: string): number {
  const n = new Date(iso).getTime()
  return Number.isNaN(n) ? 0 : n
}

// Last path segment of a working directory — the workspace label (coding
// agents are organized around projects, not chat titles).
function dirBasename(path: string): string {
  const parts = (path ?? '').replace(/\\/g, '/').split('/').filter(Boolean)
  return parts[parts.length - 1] || path || 'untitled'
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
          Workspaces
        </span>
        <span class="text-xs" style="color: var(--app-text-muted)">{{ workspaces.length }}</span>
      </div>
    </div>

    <!-- Workspace list -->
    <div class="overflow-auto pb-2">
      <div v-for="ws in workspaces" :key="ws.dir" class="workspace">
        <button type="button" class="ws-header" :title="ws.dir" @click="toggleWorkspace(ws.dir)">
          <i
            class="pi ws-chevron"
            :class="isExpanded(ws.dir) ? 'pi-chevron-down' : 'pi-chevron-right'"
          />
          <i class="pi pi-folder ws-icon" />
          <span class="ws-name">{{ ws.name }}</span>
          <span class="ws-count">{{ ws.count }}</span>
        </button>

        <div v-show="isExpanded(ws.dir)" class="ws-threads">
          <button
            v-for="t in ws.threads"
            :key="t.id"
            type="button"
            class="thread-row"
            :class="{ active: selectedId === t.id }"
            :title="t.title || 'Untitled'"
            @click="selectThread(t)"
          >
            <span class="thread-title">{{ t.title || 'Untitled' }}</span>
            <span v-if="timeAgo(t.updatedAt)" class="thread-time">{{ timeAgo(t.updatedAt) }}</span>
          </button>
        </div>
      </div>

      <div v-if="!workspaces.length" class="px-2 py-6 text-center text-sm" style="color: var(--app-text-muted)">
        No threads yet.
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Workspace header: a clickable row that toggles its thread sub-list. */
.ws-header {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin: 2px 4px;
  padding: 7px 10px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--app-text);
  font-size: 0.9rem;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.12s ease;
}
.ws-header:hover {
  background: var(--p-surface-100);
}
.app-dark .ws-header:hover {
  background: var(--p-surface-800);
}
.ws-chevron {
  font-size: 0.7rem;
  color: var(--app-text-muted);
  width: 12px;
  text-align: center;
}
.ws-icon {
  color: var(--p-primary-500);
  opacity: 0.85;
}
.ws-name {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
.ws-count {
  flex-shrink: 0;
  font-size: 0.72rem;
  color: var(--app-text-muted);
}

/* Thread sub-rows, indented under their workspace header. */
.ws-threads {
  display: flex;
  flex-direction: column;
}
.thread-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin: 1px 4px;
  padding: 6px 10px 6px 30px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--app-text);
  font-size: 0.85rem;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.12s ease;
}
.thread-row:hover {
  background: var(--p-surface-100);
}
.app-dark .thread-row:hover {
  background: var(--p-surface-800);
}
.thread-row.active {
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
  border-color: color-mix(in srgb, var(--p-primary-500) 30%, transparent);
}
.thread-row.active:hover {
  background: color-mix(in srgb, var(--p-primary-500) 20%, transparent);
}
.thread-title {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.thread-time {
  flex-shrink: 0;
  font-size: 0.72rem;
  color: var(--app-text-muted);
}
</style>
