import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './style.css'
import { initTheme } from './theme'

initTheme()

const app = createApp(App)
app.use(createPinia())
app.mount('#app')
