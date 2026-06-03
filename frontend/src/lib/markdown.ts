// Minimal, XSS-safe markdown rendering shared by the Chat and Notes windows.
// Output uses md-* classes; each consumer styles them (scoped, via :deep).

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function inlineMd(s: string): string {
  // Inline code spans are pulled out behind \u0001 sentinels so the other
  // replacements can't touch their contents, then restored at the end.
  const codes: string[] = []
  s = s.replace(/`([^`]+)`/g, (_m, c) => { codes.push(c); return `\u0001${codes.length - 1}\u0001` })
  s = s.replace(/\*\*([^*]+?)\*\*/g, '<strong>$1</strong>')
  s = s.replace(/__([^_]+?)__/g, '<strong>$1</strong>')
  s = s.replace(/(^|[^*\w])\*(?!\s)([^*\n]+?)\*/g, '$1<em>$2</em>')
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>')
  s = s.replace(/\u0001(\d+)\u0001/g, (_m, n) => `<code>${codes[+n]}</code>`)
  return s
}

export function renderMarkdown(src: string): string {
  const lines = escapeHtml(src).split('\n')
  const out: string[] = []
  let i = 0
  let list: 'ul' | 'ol' | null = null
  const closeList = () => { if (list) { out.push(`</${list}>`); list = null } }
  while (i < lines.length) {
    const line = lines[i]
    if (/^\s*```/.test(line)) {
      closeList(); i++
      const code: string[] = []
      while (i < lines.length && !/^\s*```/.test(lines[i])) { code.push(lines[i]); i++ }
      i++
      out.push(`<pre class="md-pre"><code>${code.join('\n')}</code></pre>`)
      continue
    }
    const h = /^(#{1,6})\s+(.*)$/.exec(line)
    if (h) { closeList(); out.push(`<div class="md-h md-h${Math.min(h[1].length, 3)}">${inlineMd(h[2])}</div>`); i++; continue }
    const ul = /^\s*[-*+]\s+(.*)$/.exec(line)
    if (ul) { if (list !== 'ul') { closeList(); out.push('<ul class="md-ul">'); list = 'ul' } out.push(`<li>${inlineMd(ul[1])}</li>`); i++; continue }
    const ol = /^\s*\d+\.\s+(.*)$/.exec(line)
    if (ol) { if (list !== 'ol') { closeList(); out.push('<ol class="md-ol">'); list = 'ol' } out.push(`<li>${inlineMd(ol[1])}</li>`); i++; continue }
    if (/^\s*$/.test(line)) { closeList(); out.push('<br>'); i++; continue }
    closeList(); out.push(`<div>${inlineMd(line)}</div>`); i++
  }
  closeList()
  return out.join('\n')
}
