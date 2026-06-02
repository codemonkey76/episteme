import { createRouter, createWebHistory } from 'vue-router'
import Chats from './views/Chats.vue'
import Calendar from './views/Calendar.vue'
import Notes from './views/Notes.vue'
import Tasks from './views/Tasks.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/chats' },
    { path: '/chats', component: Chats },
    { path: '/calendar', component: Calendar },
    { path: '/notes', component: Notes },
    { path: '/tasks', component: Tasks },
  ],
})
