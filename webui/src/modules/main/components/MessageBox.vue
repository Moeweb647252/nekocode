<script setup lang="ts">
import type { ChatMessage, ToolCallResultInner } from '@/api'
import MarkdownView from './MarkdownView.vue'

const props = defineProps({
  messages: {
    type: Array as () => ChatMessage[],
    required: true,
  },
  generating: {
    type: Boolean,
    default: false,
  },
})

// Index tool-call results by id so we can look up the result for each
// toolCall block. Built from the full message list (includes streamed results).
const toolResults = computed(() => {
  const results: Record<string, ToolCallResultInner> = {}
  for (const msg of props.messages) {
    if (msg.type === 'toolCallResult') {
      results[msg.data.id] = msg.data.result
    }
  }
  return results
})

// Render a tool call's argument summary: prefer a "command" field, otherwise
// show the JSON-serialized args. `args` is typed `unknown` so we narrow it.
function argsSummary(args: unknown): string {
  if (args && typeof args === 'object' && 'command' in args) {
    return String((args as { command: unknown }).command)
  }
  return JSON.stringify(args)
}

// Stringify a tool result's payload for display.
function resultText(result: ToolCallResultInner | undefined): string | null {
  if (!result) return null
  if (result.type === 'success') {
    return JSON.stringify(result)
  }
  return result.error
}

// Status of a tool call: 'success' | 'error' | 'running'
function toolStatus(result: ToolCallResultInner | undefined): 'success' | 'error' | 'running' {
  if (!result) return 'running'
  return result.type === 'success' ? 'success' : 'error'
}

// Auto-scroll: keep the view pinned to the bottom as messages stream in.
const scroller = ref<HTMLElement | null>(null)
const stickToBottom = ref(true)

function onScroll() {
  const el = scroller.value
  if (!el) return
  stickToBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 80
}

function maybeScroll() {
  const el = scroller.value
  if (!el || !stickToBottom.value) return
  el.scrollTop = el.scrollHeight
}

watch(
  () => props.messages.map((m) => JSON.stringify(m)).join('|'),
  () => nextTick(maybeScroll),
)
onMounted(() => nextTick(maybeScroll))
</script>

<template>
  <div ref="scroller" class="w-full h-full overflow-auto" @scroll="onScroll">
    <div class="mx-auto max-w-4xl px-5 py-4">
      <template v-for="(item, index) in messages" :key="index">
        <!-- User prompt -->
        <section v-if="item.type === 'user'" class="msg-block user-block">
          <div class="msg-role">user</div>
          <pre class="user-text">{{ item.data.content }}</pre>
        </section>

        <!-- Assistant -->
        <section v-else-if="item.type === 'assistant'" class="msg-block assistant-block">
          <template v-for="(block, bi) in item.data.blocks" :key="bi">
            <!-- Text block (may carry reasoning) -->
            <div v-if="block.type === 'text'">
              <!-- Reasoning (collapsible) -->
              <details v-if="block.reasoningContent" class="reasoning">
                <summary>
                  <i class="pi pi-bolt reasoning-icon"></i>
                  <span class="reasoning-label">thinking</span>
                </summary>
                <pre class="reasoning-text">{{ block.reasoningContent }}</pre>
              </details>
              <!-- Content -->
              <MarkdownView v-if="block.content" :content="block.content" class="assistant-text" />
            </div>

            <!-- Tool call — first-class citizen, terminal-style -->
            <div v-else-if="block.type === 'toolCall'" class="tool-block">
              <details>
                <summary class="tool-header">
                  <span class="tool-status" :data-status="toolStatus(toolResults[block.id])">
                    <i
                      class="pi"
                      :class="{
                        'pi-spin pi-spinner': toolStatus(toolResults[block.id]) === 'running',
                        'pi-check': toolStatus(toolResults[block.id]) === 'success',
                        'pi-times': toolStatus(toolResults[block.id]) === 'error',
                      }"
                    ></i>
                  </span>
                  <span class="tool-name">{{ block.name }}</span>
                  <code class="tool-args">{{ argsSummary(block.args) }}</code>
                </summary>
                <pre class="tool-result">{{
                  resultText(toolResults[block.id]) ?? 'running…'
                }}</pre>
              </details>
            </div>
          </template>

          <!-- Streaming caret -->
          <span v-if="generating && index === messages.length - 1" class="caret">▋</span>
        </section>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* ── Message blocks: flat, full-width, document-style ── */
.msg-block {
  padding: 10px 0;
}
.msg-block + .msg-block {
  border-top: 1px solid var(--app-border);
}

/* Role label */
.msg-role {
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--app-text-muted);
  margin-bottom: 4px;
}

/* ── User prompt: preformatted, prompt-style ── */
.user-text {
  margin: 0;
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.85rem;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--app-text);
  background: color-mix(in srgb, var(--p-primary-500) 6%, transparent);
  border-left: 2px solid color-mix(in srgb, var(--p-primary-500) 40%, transparent);
  padding: 8px 12px;
  border-radius: 0 6px 6px 0;
}

/* ── Assistant: document-style prose ── */
.assistant-block {
  padding-left: 0;
}
.assistant-text {
  font-size: 0.875rem;
  line-height: 1.6;
}
.assistant-text + .assistant-text {
  margin-top: 8px;
}

/* ── Reasoning ── */
.reasoning {
  margin-bottom: 8px;
  font-size: 0.8rem;
}
.reasoning summary {
  cursor: pointer;
  padding: 4px 0;
  color: var(--app-text-muted);
  list-style: none;
  user-select: none;
  display: flex;
  align-items: center;
  gap: 6px;
}
.reasoning summary::-webkit-details-marker {
  display: none;
}
.reasoning-icon {
  font-size: 0.75rem;
}
.reasoning-label {
  font-style: italic;
}
.reasoning-text {
  margin: 6px 0 0;
  padding: 8px 12px;
  border-left: 2px solid var(--app-border);
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.78rem;
  color: var(--app-text-muted);
  white-space: pre-wrap;
  word-break: break-word;
}

/* ── Tool call: terminal-style, first-class ── */
.tool-block {
  margin: 8px 0;
  border: 1px solid var(--app-border);
  border-radius: 6px;
  background: color-mix(in srgb, var(--p-surface-900) 3%, transparent);
  overflow: hidden;
  font-size: 0.82rem;
}
.app-dark .tool-block {
  background: color-mix(in srgb, var(--p-surface-0) 3%, transparent);
}
.tool-header {
  cursor: pointer;
  padding: 6px 10px;
  display: flex;
  align-items: center;
  gap: 8px;
  list-style: none;
  user-select: none;
  min-width: 0;
  background: color-mix(in srgb, var(--p-surface-900) 5%, transparent);
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
}
.app-dark .tool-header {
  background: color-mix(in srgb, var(--p-surface-0) 5%, transparent);
}
.tool-header::-webkit-details-marker {
  display: none;
}
.tool-name {
  font-weight: 600;
  color: var(--app-text);
  flex-shrink: 0;
}
.tool-args {
  color: var(--app-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
  font-family: inherit;
}
.tool-status {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  font-size: 0.75rem;
}
.tool-status[data-status='success'] {
  color: #22c55e;
}
.tool-status[data-status='error'] {
  color: #ef4444;
}
.tool-status[data-status='running'] {
  color: var(--p-primary-500);
}
.tool-result {
  margin: 0;
  padding: 8px 12px;
  border-top: 1px solid var(--app-border);
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.78rem;
  color: var(--app-text-muted);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 320px;
  overflow: auto;
  line-height: 1.5;
}

/* ── Streaming caret ── */
.caret {
  display: inline-block;
  color: var(--p-primary-500);
  animation: nekocode-blink 1s step-end infinite;
}
@keyframes nekocode-blink {
  0%,
  50% {
    opacity: 1;
  }
  51%,
  100% {
    opacity: 0;
  }
}
</style>
