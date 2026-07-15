<script setup lang="ts">
import { createThread, createWorkspace, getDirs, listWorkspaces } from '@/api'
import type { Thread, WorkspaceResponse } from '@/api/types'
import { useDialog } from 'primevue'
import { useToast } from 'primevue/usetoast'
import PickFolder from './PickFolder.vue'
import ThreadTreeNode from './ThreadTreeNode.vue'

const dialog = useDialog()
const toast = useToast()

// Workspaces are a persisted, server-owned grouping (one per working
// directory), each carrying its threads. Loaded from /workspace/list.
const workspaces = ref<WorkspaceResponse[]>([])
const homeDir = ref()
const selectedThread = inject<Ref<Thread>>('selectedThread')

async function loadWorkspaces() {
  try {
    workspaces.value = await listWorkspaces()
  } catch (e) {
    console.error('Failed to list workspaces:', e)
    toast.add({ severity: 'error', summary: 'Error', detail: 'Failed to load workspaces', life: 5000 })
  }
}

onMounted(loadWorkspaces)

/**
 * Per-workspace tree view: top-level threads (ownById == null) plus an index
 * mapping each parent thread id to its subthreads. Built once per render from
 * the flat `ws.threads` array the backend returns. Newly created subthreads
 * reuse their parent's workspace, so no extra request is needed to render the
 * tree.
 */
interface WorkspaceTree {
  ws: WorkspaceResponse
  topLevelThreads: Thread[]
  childrenByParent: Map<number, Thread[]>
}

function buildTree(ws: WorkspaceResponse): WorkspaceTree {
  const topLevelThreads: Thread[] = []
  const childrenByParent = new Map<number, Thread[]>()
  for (const t of ws.threads) {
    if (t.ownById == null) {
      topLevelThreads.push(t)
    } else {
      const list = childrenByParent.get(t.ownById)
      if (list) list.push(t)
      else childrenByParent.set(t.ownById, [t])
    }
  }
  // Top-level threads: newest activity first (mirrors the old sort).
  topLevelThreads.sort((a, b) => ts(b.updatedAt) - ts(a.updatedAt))
  // Subthreads: spawn order (oldest first) so the tree reads top-to-bottom.
  for (const list of childrenByParent.values()) {
    list.sort((a, b) => ts(a.createdAt) - ts(b.createdAt))
  }
  return { ws, topLevelThreads, childrenByParent }
}

// Display order: workspaces by most-recent thread activity. The endpoint
// returns them unsorted.
const sortedWorkspaces = computed<WorkspaceTree[]>(() => {
  const trees = workspaces.value.map(buildTree)
  return trees.sort((a, b) => lastActivity(b.ws) - lastActivity(a.ws))
})

function lastActivity(ws: WorkspaceResponse): number {
  return ws.threads.reduce((max, t) => Math.max(max, ts(t.updatedAt)), 0)
}

// Collapsed state keyed by workspace id. Absent = expanded (default).
const collapsed = reactive<Record<number, boolean>>({})
// Per-thread collapse state (top-level threads AND subthreads). Absent =
// expanded, matching the workspace convention.
const threadCollapsed = reactive<Record<number, boolean>>({})

function toggleWorkspace(id: number) {
  collapsed[id] = !collapsed[id]
}
function isExpanded(id: number) {
  return !collapsed[id]
}
function toggleThread(id: number) {
  threadCollapsed[id] = !threadCollapsed[id]
}

/**
 * Thread lookup by id across all workspaces, used to walk the `ownById`
 * ancestor chain when expanding a selected subthread's parents.
 */
const threadById = computed<Map<number, Thread>>(() => {
  const map = new Map<number, Thread>()
  for (const ws of workspaces.value) {
    for (const t of ws.threads) map.set(t.id, t)
  }
  return map
})

function selectThread(t: Thread) {
  if (!selectedThread) return
  selectedThread.value = t
  // Keep the selected thread visible: never leave its workspace collapsed…
  if (t.workspaceId != null) collapsed[t.workspaceId] = false
  // …nor any of its ancestor threads. Walk ownById upward, expanding each.
  let cursor: Thread | undefined = t
  while (cursor && cursor.ownById != null) {
    threadCollapsed[cursor.ownById] = false
    cursor = threadById.value.get(cursor.ownById)
  }
}

// Expose the shared tree state to ThreadTreeNode via inject keys. The node
// component reads these instead of receiving them as props at every depth.
provide('threadCollapsed', threadCollapsed)
provide('selectThread', selectThread)
provide('toggleThread', toggleThread)

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
          toast.add({ severity: 'error', summary: 'Error', detail: 'Failed to create workspace', life: 5000 })
        }
      },
    })
  } catch (e) {
    console.error('Failed to open new-workspace dialog:', e)
    toast.add({ severity: 'error', summary: 'Error', detail: 'Could not open folder picker', life: 5000 })
  }
}

// Per-workspace action: start a new thread inside an existing workspace. The
// backend find-or-creates the workspace for the directory (already present)
// and links the new thread to it — no folder picker needed.
const newThreadInWorkspace = async (ws: WorkspaceResponse) => {
  try {
    await createThread(ws.workingDirectory)
    await loadWorkspaces()
  } catch (e) {
    console.error('Failed to create thread:', e)
    toast.add({ severity: 'error', summary: 'Error', detail: 'Failed to create thread', life: 5000 })
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
      <div v-for="tree in sortedWorkspaces" :key="tree.ws.id" class="workspace">
        <div class="ws-row">
          <button
            type="button"
            class="ws-header"
            :title="tree.ws.workingDirectory"
            @click="toggleWorkspace(tree.ws.id)"
          >
            <i
              class="pi ws-chevron"
              :class="isExpanded(tree.ws.id) ? 'pi-chevron-down' : 'pi-chevron-right'"
            />
            <i class="pi pi-folder ws-icon" />
            <span class="ws-name">{{ tree.ws.name || dirBasename(tree.ws.workingDirectory) }}</span>
            <span class="ws-count">{{ tree.ws.threads.length }}</span>
          </button>
          <button
            type="button"
            class="ws-add"
            title="New thread in this workspace"
            @click="newThreadInWorkspace(tree.ws)"
          >
            <i class="pi pi-plus" />
          </button>
        </div>

        <div v-show="isExpanded(tree.ws.id)" class="ws-threads">
          <ThreadTreeNode
            v-for="t in tree.topLevelThreads"
            :key="t.id"
            :thread="t"
            :children-by-parent="tree.childrenByParent"
            :depth="0"
          />
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

/* Thread sub-tree container, indented under the workspace header. */
.ws-threads {
  display: flex;
  flex-direction: column;
}
</style>
