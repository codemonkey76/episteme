<script setup lang="ts">
// Rich-text (contenteditable) editor for email composition and signatures.
// - v-model carries HTML.
// - Images pasted or dropped from the clipboard land inline as data: URIs;
//   the consumer converts them to cid: attachments at send time.
// - Non-image files dropped on the editor are emitted via @files so the
//   parent can add them as regular attachments.
import { ref, watch, onMounted } from 'vue'

const props = withDefaults(
  defineProps<{
    modelValue: string
    placeholder?: string
    minHeight?: string
  }>(),
  { placeholder: '', minHeight: '120px' },
)

const emit = defineEmits<{
  (e: 'update:modelValue', html: string): void
  (e: 'files', files: File[]): void
}>()

const editor = ref<HTMLDivElement | null>(null)

// Only push external model changes into the DOM when they differ — writing
// innerHTML on every keystroke would reset the caret to the start.
watch(
  () => props.modelValue,
  (html) => {
    if (editor.value && editor.value.innerHTML !== html) {
      editor.value.innerHTML = html
    }
  },
)

onMounted(() => {
  if (editor.value) editor.value.innerHTML = props.modelValue
})

function onInput() {
  if (editor.value) emit('update:modelValue', editor.value.innerHTML)
}

// execCommand is deprecated but still the only dependency-free way to drive
// contenteditable formatting, and every browser keeps it working for exactly
// this use case.
function exec(command: string, value?: string) {
  editor.value?.focus()
  document.execCommand(command, false, value)
  onInput()
}

function addLink() {
  const url = window.prompt('Link URL')
  if (url) exec('createLink', url)
}

// Insert a file as an inline <img> at the caret (data: URI; converted to a
// cid: inline attachment when the email is sent).
function insertImageFile(file: File) {
  const reader = new FileReader()
  reader.onload = () => {
    const src = reader.result as string
    editor.value?.focus()
    document.execCommand('insertImage', false, src)
    onInput()
  }
  reader.readAsDataURL(file)
}

function onPaste(e: ClipboardEvent) {
  const items = Array.from(e.clipboardData?.items ?? [])
  const images = items.filter(i => i.kind === 'file' && i.type.startsWith('image/'))
  if (!images.length) return // let text/HTML paste through normally
  e.preventDefault()
  for (const item of images) {
    const file = item.getAsFile()
    if (file) insertImageFile(file)
  }
}

function onDrop(e: DragEvent) {
  const files = Array.from(e.dataTransfer?.files ?? [])
  if (!files.length) return
  e.preventDefault()
  const others: File[] = []
  for (const f of files) {
    if (f.type.startsWith('image/')) insertImageFile(f)
    else others.push(f)
  }
  if (others.length) emit('files', others)
}

function focus() {
  editor.value?.focus()
}

defineExpose({ focus })

const buttons: { label: string; title: string; cmd: () => void; cls?: string }[] = [
  { label: 'B', title: 'Bold', cmd: () => exec('bold'), cls: 'font-bold' },
  { label: 'I', title: 'Italic', cmd: () => exec('italic'), cls: 'italic' },
  { label: 'U', title: 'Underline', cmd: () => exec('underline'), cls: 'underline' },
  { label: '•', title: 'Bullet list', cmd: () => exec('insertUnorderedList') },
  { label: '1.', title: 'Numbered list', cmd: () => exec('insertOrderedList') },
  { label: '🔗', title: 'Insert link', cmd: addLink },
  { label: '⌫', title: 'Clear formatting', cmd: () => exec('removeFormat') },
]
</script>

<template>
  <div class="flex flex-col min-h-0">
    <div class="flex items-center gap-1 pb-1.5 border-b border-[var(--c-1e1e1e)]">
      <button
        v-for="b in buttons"
        :key="b.title"
        type="button"
        :title="b.title"
        class="min-w-[1.7rem] py-[0.15rem] px-1.5 bg-transparent text-[var(--c-808080)] border border-transparent rounded text-[0.775rem] font-[inherit] cursor-pointer transition-colors duration-100 hover:bg-[var(--c-1e1e1e)] hover:text-[var(--c-c0c0c0)]"
        :class="b.cls"
        @mousedown.prevent
        @click="b.cmd()"
      >{{ b.label }}</button>
    </div>
    <div
      ref="editor"
      contenteditable="true"
      class="rte-body flex-1 overflow-y-auto text-[var(--c-d0d0d0)] text-[0.8125rem] leading-[1.6] py-2 outline-none"
      :style="{ minHeight: props.minHeight }"
      :data-placeholder="props.placeholder"
      @input="onInput"
      @paste="onPaste"
      @drop="onDrop"
      @dragover.prevent
    />
  </div>
</template>

<style scoped>
/* Placeholder for an empty contenteditable. */
.rte-body:empty::before {
  content: attr(data-placeholder);
  color: var(--c-404040);
  pointer-events: none;
}
.rte-body :deep(a) { color: var(--c-7ab0ff); text-decoration: underline; }
.rte-body :deep(ul) { list-style: disc; padding-left: 1.4rem; margin: 0.25rem 0; }
.rte-body :deep(ol) { list-style: decimal; padding-left: 1.4rem; margin: 0.25rem 0; }
.rte-body :deep(img) { max-width: 100%; height: auto; }
.rte-body :deep(blockquote) {
  border-left: 2px solid var(--c-2a4a8a);
  margin: 0.25rem 0;
  padding-left: 0.75rem;
  color: var(--c-a0a0a0);
}
</style>
