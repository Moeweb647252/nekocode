<script setup lang="ts">
import type { Thread } from '@/api/types'

/**
 * Recursive node in the workspace > thread > subthread tree. Renders one
 * thread row and, when it has children (subthreads whose `ownById` points at
 * it), recursively renders them indented one level deeper.
 *
 * The shared tree state (selected thread, per-thread collapse flags, the
 * select/toggle callbacks) is injected from `Sidebar.vue` so this component
 * stays focused on rendering a single node + its subtree. The
 * `childrenByParent` index is built once per workspace in the sidebar and
 * passed down unchanged at every depth.
 */

const props = defineProps<{
  thread: Thread
  /** Parent-id → children index, shared across the whole workspace subtree. */
  childrenByParent: Map<number, Thread[]>
  /** 0 for a top-level thread, +1 per nesting level. Drives indentation. */
  depth: number
}>()

// Injected from Sidebar.vue. The non-null assertions are safe because the
// sidebar always provides them before mounting any node.
const selectedThread = inject<Ref<Thread>>('selectedThread')
const threadCollapsed = inject<Record<number, boolean>>('threadCollapsed')!
const selectThread = inject<(t: Thread) => void>('selectThread')!
const toggleThread = inject<(id: number) => void>('toggleThread')!

const selectedId = computed(() => selectedThread?.value?.id)

// Children of this node, looked up from the shared index. Subthreads keep
// their spawn order (createdAt ASC) so the tree reads top-to-bottom.
const children = computed(
  () => props.childrenByParent.get(props.thread.id) ?? [],
)
const hasChildren = computed(() => children.value.length > 0)
// Absent in the collapse map = expanded (mirrors the workspace convention).
const expanded = computed(() => !threadCollapsed[props.thread.id])

// Indentation grows with depth so the hierarchy is readable at a glance.
const rowStyle = computed(() => ({
  paddingLeft: `${14 + props.depth * 16}px`,
}))

// Distinguish top-level threads (chat bubbles) from nested subthreads
// (sitemap icon) so depth is legible even without tracking the indent.
const iconClass = computed(() => (props.depth === 0 ? 'pi-comment' : 'pi-sitemap'))

// Relative-ish timestamp (e.g. "2h ago"). Mirrors Sidebar's helper; duplicated
// here to keep the node self-contained rather than threading another inject.
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

function onRowClick() {
  selectThread(props.thread)
}
</script>

<template>
  <div class="node">
    <button
      type="button"
      class="thread-row"
      :class="{ active: selectedId === thread.id }"
      :style="rowStyle"
      :title="thread.title || 'Untitled'"
      @click="onRowClick"
    >
      <i
        v-if="hasChildren"
        class="pi chevron"
        :class="expanded ? 'pi-chevron-down' : 'pi-chevron-right'"
        @click.stop="toggleThread(thread.id)"
      />
      <i v-else class="pi chevron-placeholder" />
      <i class="pi node-icon" :class="iconClass" />
      <span class="thread-title">{{ thread.title || 'Untitled' }}</span>
      <span v-if="hasChildren" class="child-count">{{ children.length }}</span>
      <span v-else-if="timeAgo(thread.updatedAt)" class="thread-time">{{
        timeAgo(thread.updatedAt)
      }}</span>
    </button>

    <div v-if="hasChildren && expanded" class="children">
      <ThreadTreeNode
        v-for="child in children"
        :key="child.id"
        :thread="child"
        :children-by-parent="childrenByParent"
        :depth="depth + 1"
      />
    </div>
  </div>
</template>

<style scoped>
.node {
  display: flex;
  flex-direction: column;
}
.thread-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin: 1px 4px;
  padding: 6px 10px;
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
/* Chevron toggles the subtree; aligned with the workspace chevron. */
.chevron {
  font-size: 0.7rem;
  color: var(--app-text-muted);
  width: 12px;
  text-align: center;
  cursor: pointer;
  flex-shrink: 0;
}
/* Keep alignment for leaf rows that have no chevron. */
.chevron-placeholder {
  width: 12px;
  flex-shrink: 0;
}
.node-icon {
  font-size: 0.8rem;
  color: var(--app-text-muted);
  opacity: 0.85;
  flex-shrink: 0;
}
.thread-title {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.child-count {
  flex-shrink: 0;
  font-size: 0.72rem;
  color: var(--app-text-muted);
}
.thread-time {
  flex-shrink: 0;
  font-size: 0.72rem;
  color: var(--app-text-muted);
}
</style>
