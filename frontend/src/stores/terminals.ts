import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

// Per-terminal callbacks live outside Pinia state (functions, non-serializable).
// The store keeps only the reactive bits (open list + which one is active).
interface Handlers {
  shell: string
  /** Write text onto the prompt without a trailing newline (never runs it). */
  paste: (cmd: string) => void
  /** Recent buffer text, for giving the AI suggestion some context. */
  scrollback: () => string
}
const handlers = new Map<string, Handlers>()

export const useTerminalsStore = defineStore('terminals', () => {
  const terminals = ref<{ id: string; shell: string }[]>([])
  const activeId = ref<string | null>(null)

  function register(id: string, shell: string, h: Handlers) {
    handlers.set(id, h)
    terminals.value.push({ id, shell })
    activeId.value = id
  }

  function unregister(id: string) {
    handlers.delete(id)
    terminals.value = terminals.value.filter((t) => t.id !== id)
    if (activeId.value === id) {
      activeId.value = terminals.value.at(-1)?.id ?? null
    }
  }

  function setActive(id: string) {
    if (handlers.has(id)) activeId.value = id
  }

  function activeHandlers(): Handlers | null {
    return activeId.value ? handlers.get(activeId.value) ?? null : null
  }

  /** Paste a command onto the active terminal's prompt. Returns false if none. */
  function pasteToActive(cmd: string): boolean {
    const h = activeHandlers()
    if (!h) return false
    h.paste(cmd)
    return true
  }

  function activeScrollback(): string {
    return activeHandlers()?.scrollback() ?? ''
  }

  const activeShell = computed(() =>
    activeId.value ? handlers.get(activeId.value)?.shell ?? null : null,
  )

  return {
    terminals,
    activeId,
    activeShell,
    register,
    unregister,
    setActive,
    pasteToActive,
    activeScrollback,
  }
})
