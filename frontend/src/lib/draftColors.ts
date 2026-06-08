// Make a draft's HTML body readable in the dark rich-text editor WITHOUT
// baking wrong colours into the outgoing mail.
//
// Quoted history pasted from Outlook/Gmail carries inline `color:#000` etc.
// (authored for a white background) — black-on-black in our dark editor. We
// can't re-colour it light: that would then be near-invisible in the
// recipient's light mail client. Instead we STRIP the clashing declaration so
// the text inherits a default at both ends — light in our dark editor, dark in
// the recipient's light client — i.e. it looks the same to them, readable to
// us. Light-background "islands" (intentional highlights) are left intact so
// their dark text stays readable.

/** Strip inline colours that would clash with the dark editor background. */
export function sanitizeDraftForDarkEditor(html: string): string {
  if (!html.includes('<')) return html
  let doc: Document
  try {
    doc = new DOMParser().parseFromString(html, 'text/html')
  } catch {
    return html
  }
  for (const el of Array.from(doc.body.querySelectorAll<HTMLElement>('*'))) {
    sanitizeElement(el)
  }
  return doc.body.innerHTML
}

function sanitizeElement(el: HTMLElement): void {
  const bgRaw =
    cssValue(el.style.cssText, 'background-color') ??
    cssValue(el.style.cssText, 'background') ??
    el.getAttribute('bgcolor') ??
    undefined
  const bg = parseColor(bgRaw)
  // A light background is a self-consistent island (dark text on it stays
  // readable) — leave it and its text alone.
  if (bg && luminance(bg) > 0.6) return

  // Dark/!set background that clashes with the editor: drop it so the element
  // sits on the editor surface.
  if (bg && luminance(bg) <= 0.6) {
    stripDecl(el, 'background')
    stripDecl(el, 'background-color')
    el.removeAttribute('bgcolor')
  }

  // Dark text on a dark surface would vanish — strip it so it inherits the
  // editor's light default (and the recipient's dark default when sent).
  const color = parseColor(cssValue(el.style.cssText, 'color') ?? el.getAttribute('color') ?? undefined)
  if (color && luminance(color) < 0.5) {
    stripDecl(el, 'color')
    el.removeAttribute('color')
  }
}

/** Remove a single inline declaration, leaving the rest of the style intact. */
function stripDecl(el: HTMLElement, prop: string): void {
  const next = el.style.cssText
    // `background` mustn't match `background-color` — require the bare prop name.
    .replace(new RegExp(`(?:^|;)\\s*${prop}\\s*:[^;]*`, 'gi'), '')
    .replace(/^[;\s]+|[;\s]+$/g, '')
  if (next) el.setAttribute('style', next)
  else el.removeAttribute('style')
}

/** Last declaration of `prop` in an inline style string, or undefined. */
function cssValue(style: string, prop: string): string | undefined {
  const re = new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`, 'gi')
  let m: RegExpExecArray | null
  let last: string | undefined
  while ((m = re.exec(style))) last = m[1].trim()
  return last
}

/** Perceived luminance 0–1. */
function luminance([r, g, b]: [number, number, number]): number {
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255
}

const NAMED: Record<string, [number, number, number]> = {
  black: [0, 0, 0],
  white: [255, 255, 255],
  gray: [128, 128, 128],
  grey: [128, 128, 128],
  silver: [192, 192, 192],
  navy: [0, 0, 128],
  red: [255, 0, 0],
  blue: [0, 0, 255],
}

/** Parse a CSS colour to RGB, or undefined when unparseable/transparent. */
function parseColor(raw?: string): [number, number, number] | undefined {
  if (!raw) return undefined
  const v = raw.replace('!important', '').trim().toLowerCase()
  if (!v || ['transparent', 'inherit', 'initial', 'unset', 'currentcolor', 'none'].includes(v)) {
    return undefined
  }
  if (NAMED[v]) return NAMED[v]
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/.exec(v)
  if (hex) {
    let h = hex[1]
    if (h.length === 3) h = h.split('').map(c => c + c).join('')
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]
  }
  const rgb = /^rgba?\(([^)]+)\)$/.exec(v)
  if (rgb) {
    const parts = rgb[1].split(/[,/\s]+/).filter(Boolean)
    if (parts.length >= 3) {
      const ch = (p: string) => (p.endsWith('%') ? (parseFloat(p) * 255) / 100 : parseFloat(p))
      const [r, g, b] = [ch(parts[0]), ch(parts[1]), ch(parts[2])]
      // Fully transparent → treat as unset.
      if (parts.length >= 4 && parseFloat(parts[3]) === 0) return undefined
      if ([r, g, b].every(n => Number.isFinite(n))) return [r, g, b]
    }
  }
  return undefined
}
