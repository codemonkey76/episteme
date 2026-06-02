import { createRouter, createWebHistory } from 'vue-router'
import Chat from './views/Chat.vue'
import Chats from './views/Chats.vue'
import Email from './views/Email.vue'
import Calendar from './views/Calendar.vue'
import Notes from './views/Notes.vue'
import Tasks from './views/Tasks.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: Chat },
    { path: '/chats', component: Chats },
    { path: '/email', component: Email },
    { path: '/calendar', component: Calendar },
    { path: '/notes', component: Notes },
    { path: '/tasks', component: Tasks },
  ],
})
