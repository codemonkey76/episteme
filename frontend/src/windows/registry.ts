import { markRaw } from 'vue'
import type { Component } from 'vue'
import type { DockAnchor } from '../stores/windows'

import Chat from '../views/Chat.vue'
import Chats from '../views/Chats.vue'
import Email from '../views/Email.vue'
import Calendar from '../views/Calendar.vue'
import Documents from '../views/Documents.vue'
import Jobs from '../views/Jobs.vue'
import Notes from '../views/Notes.vue'
import Shipments from '../views/Shipments.vue'
import Writer from '../views/Writer.vue'
import Notifications from '../views/Notifications.vue'
import Reports from '../views/Reports.vue'
import Logs from '../views/Logs.vue'
import Tasks from '../views/Tasks.vue'
import Memories from '../views/Memories.vue'
import Prompts from '../views/Prompts.vue'
import Terminal from '../views/Terminal.vue'
import SettingsPanel from '../components/SettingsPanel.vue'

export interface WindowDef {
  title: string
  component: Component
  width?: number
  height?: number
  props?: Record<string, unknown>
  initialDock?: DockAnchor
  /** Allow several independent instances (e.g. multiple chat conversations). */
  multi?: boolean
}

// Single source of truth for every openable window. Keyed by the stable `key`
// stored with each window — used both to open windows and to reconstruct them
// from a persisted layout (the component itself can't be serialized).
export const windowRegistry: Record<string, WindowDef> = {
  chat: { title: 'Chat', component: markRaw(Chat), width: 740, height: 560, multi: true },
  chats: { title: 'History', component: markRaw(Chats), width: 520, height: 480 },
  email: { title: 'Email', component: markRaw(Email), width: 1100, height: 660 },
  calendar: { title: 'Calendar', component: markRaw(Calendar), width: 800, height: 560 },
  notes: { title: 'Notes', component: markRaw(Notes), width: 680, height: 520 },
  shipments: { title: 'Shipments', component: markRaw(Shipments), width: 720, height: 560 },
  writer: { title: 'Writer', component: markRaw(Writer), width: 920, height: 640 },
  notifications: { title: 'Notifications', component: markRaw(Notifications), width: 520, height: 560 },
  documents: { title: 'Documents', component: markRaw(Documents), width: 680, height: 520 },
  jobs: { title: 'Jobs', component: markRaw(Jobs), width: 620, height: 560 },
  reports: { title: 'Reports', component: markRaw(Reports), width: 960, height: 660 },
  logs: { title: 'Logs', component: markRaw(Logs), width: 900, height: 520 },
  tasks: { title: 'Tasks', component: markRaw(Tasks), width: 620, height: 480 },
  memories: { title: 'Memories', component: markRaw(Memories), width: 680, height: 560 },
  prompts: { title: 'Prompts', component: markRaw(Prompts), width: 860, height: 600 },
  terminal: { title: 'Terminal', component: markRaw(Terminal), width: 1040, height: 560 },
  settings: { title: 'Settings', component: markRaw(SettingsPanel), width: 680, props: { initialTab: 'account' } },
}
