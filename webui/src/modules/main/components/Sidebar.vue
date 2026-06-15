<script setup lang="ts">
import { createThread, createWorkspace, getDirs, listWorkspaces } from '@/api'
import type { Thread, Workspace } from '@/api/types'
import { useDialog } from 'primevue'
import PickFolder from './PickFolder.vue'

const dialog = useDialog()

// Workspaces are a persisted, server-owned grouping (one per working
// directory), each carrying its threads. Loaded from /workspace/list.
const workspaces = ref<Workspace[]>([])
const homeDir = ref()
const selectedThread = inject<Ref<Thread>>('selectedThread')

async function loadWorkspaces() {
  try {
    workspaces.value = await listWorkspaces()
  } catch (e) {
    console.error('Failed to list workspaces:', e)
  }
}

onMounted(loadWorkspaces)

// Display order: workspaces by most-recent thread activity, threads within a
// workspace newest-first. The endpoint returns them unsorted.
const sortedWorkspaces = computed<Workspace[]>(() => {
  const list = workspaces.value.map((ws) => ({
    ...ws,
    threads: [...ws.threads].sort((a, b) => ts(b.updatedAt) - ts(a.updatedAt)),
  }))
  return list.sort((a, b) => lastActivity(b) - lastActivity(a))
})

function lastActivity(ws: Workspace): number {
  return ws.threads.reduce((max, t) => Math.max(max, ts(t.updatedAt)), 0)
}

// id of the currently selected thread (or undefined) for row highlighting.
const selectedId = computed(() => selectedThread?.value?.id)

// Collapsed state keyed by workspace id. Absent = expanded (default).
const collapsed = reactive<Record<number, boolean>>({})
function toggleWorkspace(id: number) {
  collapsed[id] = !collapsed[id]
}
function isExpanded(id: number) {
  return !collapsed[id]
}
function selectThread(t: Thread) {
  if (!selectedThread) return
  selectedThread.value = t
  // Keep the selected thread visible: never leave its workspace collapsed.
  if (t.workspaceId != null) collapsed[t.workspaceId] = false
}

function ts(iso: string): number {
  const n = new Date(iso).getTime()
  return Number.isNaN(n) ? 0 : n
}

// Last path segment of a working directory — the fallback workspace label when
// no explicit name is set (coding agents are organized around projects).
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

// Top-level action: pick a directory and create (find-or-create) a workspace
// for it. The workspace starts empty; threads are added per-workspace below.
const newWorkspace = async () => {
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
          await createWorkspace(path)
          await loadWorkspaces()
        } catch (e) {
          console.error('Failed to create workspace:', e)
        }
      },
    })
  } catch (e) {
    console.error('Failed to open new-workspace dialog:', e)
  }
}

// Per-workspace action: start a new thread inside an existing workspace. The
// backend find-or-creates the workspace for the directory (already present)
// and links the new thread to it — no folder picker needed.
const newThreadInWorkspace = async (ws: Workspace) => {
  try {
    await createThread(ws.workingDirectory)
    await loadWorkspaces()
  } catch (e) {
    console.error('Failed to create thread:', e)
  }
}
</script>

<template>
  <div class="h-full grid grid-rows-[auto_1fr] overflow-hidden">
    <!-- Header row -->
    <div class="px-3 py-3">
      <Button label="New Workspace" icon="pi pi-plus" class="w-full" @click="newWorkspace()" />
      <div class="mt-3 mb-1 flex items-center justify-between px-1">
        <span class="text-xs font-medium uppercase tracking-wide" style="color: var(--app-text-muted)">
          Workspaces
        </span>
        <span class="text-xs" style="color: var(--app-text-muted)">{{ sortedWorkspaces.length }}</span>
      </div>
    </div>

    <!-- Workspace list -->
    <div class="overflow-auto pb-2">
      <div v-for="ws in sortedWorkspaces" :key="ws.id" class="workspace">
        <div class="ws-row">
          <button
            type="button"
            class="ws-header"
            :title="ws.workingDirectory"
            @click="toggleWorkspace(ws.id)"
          >
            <i
              class="pi ws-chevron"
              :class="isExpanded(ws.id) ? 'pi-chevron-down' : 'pi-chevron-right'"
            />
            <i class="pi pi-folder ws-icon" />
            <span class="ws-name">{{ ws.name || dirBasename(ws.workingDirectory) }}</span>
            <span class="ws-count">{{ ws.threads.length }}</span>
          </button>
          <button
            type="button"
            class="ws-add"
            title="New thread in this workspace"
            @click="newThreadInWorkspace(ws)"
          >
            <i class="pi pi-plus" />
          </button>
        </div>

        <div v-show="isExpanded(ws.id)" class="ws-threads">
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

      <div v-if="!sortedWorkspaces.length" class="px-2 py-6 text-center text-sm" style="color: var(--app-text-muted)">
        No workspaces yet.
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Workspace row: header + inline add-thread button. */
.ws-row {
  display: flex;
  align-items: center;
  gap: 2px;
  margin: 2px 4px;
}
/* Workspace header: a clickable row that toggles its thread sub-list. */
.ws-header {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1 1 auto;
  min-width: 0;
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
/* Inline add-thread button — hidden until the row is hovered. */
.ws-add {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--app-text-muted);
  font-size: 0.8rem;
  cursor: pointer;
  opacity: 0;
  transition:
    opacity 0.12s ease,
    background-color 0.12s ease;
}
.ws-row:hover .ws-add {
  opacity: 1;
}
.ws-add:hover {
  background: var(--p-surface-100);
  color: var(--p-primary-500);
}
.app-dark .ws-add:hover {
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
