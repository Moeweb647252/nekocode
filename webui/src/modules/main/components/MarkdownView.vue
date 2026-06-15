<script setup lang="ts">
import { renderMarkdown } from './markdown'

const props = defineProps<{ content: string }>()

// Render once per content change. Output is safe: renderMarkdown escapes all
// input and only emits a whitelisted set of tags/attributes.
const html = computed(() => renderMarkdown(props.content ?? ''))

// Event-delegate the copy buttons (rendered into v-html) so each code block
// gets a working copy action without per-button listeners.
const root = ref<HTMLElement | null>(null)

async function onClick(e: MouseEvent) {
  const target = e.target as HTMLElement
  if (!target?.classList.contains('md-copy')) return
  const id = target.getAttribute('data-copy-from')
  if (!id) return
  const code = root.value?.querySelector<HTMLPreElement>(`#${id} code, #${id}`)
  const text = code?.textContent ?? ''
  try {
    await navigator.clipboard.writeText(text)
    const orig = target.textContent
    target.textContent = 'copied'
    setTimeout(() => {
      target.textContent = orig
    }, 1200)
  } catch {
    /* clipboard may be unavailable; ignore */
  }
}

onMounted(() => root.value?.addEventListener('click', onClick))
onBeforeUnmount(() => root.value?.removeEventListener('click', onClick))
</script>

<template>
  <div ref="root" class="markdown" v-html="html"></div>
</template>

<style scoped>
.markdown {
  line-height: 1.6;
  word-break: break-word;
}
.markdown :deep(.md-p) {
  margin: 0 0 0.7em 0;
}
.markdown :deep(.md-p:last-child) {
  margin-bottom: 0;
}
.markdown :deep(.md-h) {
  margin: 0.9em 0 0.4em;
  font-weight: 600;
  line-height: 1.3;
}
.markdown :deep(.md-h1) {
  font-size: 1.3em;
}
.markdown :deep(.md-h2) {
  font-size: 1.18em;
}
.markdown :deep(.md-h3) {
  font-size: 1.08em;
}
.markdown :deep(.md-h4),
.markdown :deep(.md-h5),
.markdown :deep(.md-h6) {
  font-size: 1em;
}

.markdown :deep(.md-inline-code) {
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.86em;
  padding: 0.1em 0.38em;
  border-radius: 4px;
  background: var(--p-surface-200);
  color: var(--p-primary-700);
}
.app-dark .markdown :deep(.md-inline-code) {
  background: var(--p-surface-800);
  color: var(--p-primary-300);
}

.markdown :deep(.md-pre) {
  margin: 0.6em 0;
  padding: 0;
  border-radius: 8px;
  background: #0f172a;
  border: 1px solid var(--p-surface-800);
  overflow: hidden;
}
.markdown :deep(.md-code-head) {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.3em 0.5em 0.3em 0.85em;
  background: rgba(255, 255, 255, 0.04);
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}
.markdown :deep(.md-code-lang) {
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.7em;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #94a3b8;
}
.markdown :deep(.md-copy) {
  appearance: none;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: transparent;
  color: #94a3b8;
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.68em;
  padding: 0.1em 0.5em;
  border-radius: 4px;
  cursor: pointer;
  text-transform: lowercase;
}
.markdown :deep(.md-copy:hover) {
  background: rgba(255, 255, 255, 0.06);
  color: #e2e8f0;
}
.markdown :deep(.md-pre code) {
  display: block;
  padding: 0.75em 0.9em;
  overflow-x: auto;
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.8em;
  line-height: 1.55;
  color: #e2e8f0;
  white-space: pre;
}

.markdown :deep(.md-quote) {
  margin: 0.6em 0;
  padding: 0.1em 0.9em;
  border-left: 3px solid var(--p-primary-400);
  color: var(--app-text-muted);
}

.markdown :deep(.md-ul) {
  margin: 0.4em 0;
  padding-left: 1.3em;
}
.markdown :deep(.md-ul li) {
  margin: 0.15em 0;
}

.markdown :deep(.md-hr) {
  border: none;
  border-top: 1px solid var(--app-border);
  margin: 1em 0;
}

.markdown :deep(.md-link) {
  color: var(--p-primary-600);
  text-decoration: none;
}
.markdown :deep(.md-link:hover) {
  text-decoration: underline;
}
.app-dark .markdown :deep(.md-link) {
  color: var(--p-primary-300);
}
</style>
