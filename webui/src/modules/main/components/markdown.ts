// A tiny, dependency-free markdown → HTML renderer covering the subset that
// shows up in chat: fenced code blocks (with copy-friendly escaping), inline
// code, bold/italic, headings, blockquotes, lists, links, and paragraphs.
// It is NOT a full CommonMark parser — it deliberately escapes all input and
// only interprets a safe whitelist of syntax, so no DOMPurify is needed.
//
// Rationale: pulling in `marked` + `dompurify` would add ~40KB to keep nice
// code blocks. This keeps the bundle at zero new deps while covering the
// 95% case for assistant replies.

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

// Inline formatting: links, code, bold, italic. Operates on already-escaped
// text, so the produced HTML stays safe.
function renderInline(escaped: string): string {
  let out = escaped
  // Inline code first so its contents aren't re-processed.
  out = out.replace(/`([^`]+)`/g, (_, c: string) => `<code class="md-inline-code">${c}</code>`)
  // Links [text](url) — url must be http(s)/mailto to be safe.
  out = out.replace(
    /\[([^\]]+)\]\((https?:\/\/[^\s)]+|mailto:[^\s)]+)\)/g,
    (_, t: string, u: string) =>
      `<a href="${u}" target="_blank" rel="noopener noreferrer" class="md-link">${t}</a>`,
  )
  // Bold **x** / __x__
  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  out = out.replace(/__([^_]+)__/g, '<strong>$1</strong>')
  // Italic *x* / _x_ (avoid clashing with bold by requiring non-* borders).
  out = out.replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, '$1<em>$2</em>')
  out = out.replace(/(^|[^_])_([^_\n]+)_(?!_)/g, '$1<em>$2</em>')
  return out
}

export function renderMarkdown(src: string): string {
  if (!src) return ''
  const lines = src.replace(/\r\n/g, '\n').split('\n')
  const html: string[] = []
  let i = 0
  let inList = false

  // `noUncheckedIndexedAccess` makes `lines[i]` potentially `undefined`;
  // this helper asserts the non-undefined line at `i` (the loop guard
  // guarantees `i < lines.length`, so it is always defined).
  const at = (n: number): string => {
    const l = lines[n]
    if (l === undefined) throw new Error('markdown: out of bounds')
    return l
  }

  const closeList = () => {
    if (inList) {
      html.push('</ul>')
      inList = false
    }
  }

  while (i < lines.length) {
    const line = at(i)

    // Fenced code block ```lang ... ```
    const fence = line.match(/^```(\w+)?\s*$/)
    if (fence) {
      closeList()
      const lang = fence[1] ?? ''
      const buf: string[] = []
      i++
      while (i < lines.length && !/^```\s*$/.test(at(i))) {
        buf.push(at(i))
        i++
      }
      i++ // consume closing fence (if present)
      const code = escapeHtml(buf.join('\n'))
      // Stable id so the copy button can locate the <code> element.
      const codeId = `md-code-${i}-${Math.random().toString(36).slice(2, 8)}`
      html.push(
        `<pre class="md-pre"><div class="md-code-head"><span class="md-code-lang">${escapeHtml(
          lang || 'code',
        )}</span><button type="button" class="md-copy" data-copy-from="${codeId}" title="Copy">copy</button></div><code id="${codeId}">${code}</code></pre>`,
      )
      continue
    }

    // Heading
    const h = line.match(/^(#{1,6})\s+(.*)$/)
    if (h) {
      const hashes = h[1] ?? '#'
      const level = hashes.length
      const text = h[2] ?? ''
      closeList()
      html.push(`<h${level} class="md-h md-h${level}">${renderInline(escapeHtml(text))}</h${level}>`)
      i++
      continue
    }

    // Blockquote
    if (/^>\s?/.test(line)) {
      closeList()
      const q = escapeHtml(line.replace(/^>\s?/, ''))
      html.push(`<blockquote class="md-quote">${renderInline(q)}</blockquote>`)
      i++
      continue
    }

    // Unordered list item
    if (/^\s*[-*+]\s+/.test(line)) {
      if (!inList) {
        html.push('<ul class="md-ul">')
        inList = true
      }
      const item = escapeHtml(line.replace(/^\s*[-*+]\s+/, ''))
      html.push(`<li>${renderInline(item)}</li>`)
      i++
      continue
    }

    // Horizontal rule
    if (/^(\s*[-*_]){3,}\s*$/.test(line) && line.trim().length >= 3) {
      closeList()
      html.push('<hr class="md-hr" />')
      i++
      continue
    }

    // Blank line → paragraph break
    if (line.trim() === '') {
      closeList()
      i++
      continue
    }

    // Paragraph (merge consecutive plain lines)
    closeList()
    const para: string[] = [line]
    i++
    while (i < lines.length) {
      const nxt = at(i)
      if (
        nxt.trim() === '' ||
        nxt.startsWith('```') ||
        /^(#{1,6})\s+/.test(nxt) ||
        /^>\s?/.test(nxt) ||
        /^\s*[-*+]\s+/.test(nxt)
      ) {
        break
      }
      para.push(nxt)
      i++
    }
    html.push(`<p class="md-p">${renderInline(escapeHtml(para.join('\n'))).replace(/\n/g, '<br/>')}</p>`)
  }
  closeList()
  return html.join('\n')
}
