import { createRouter, createWebHistory } from 'vue-router'
import Chats from './views/Chats.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/chats' },
    { path: '/chats', component: Chats },
  ],
})
