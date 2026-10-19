import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './shared/widget.css'

createApp(App).use(createPinia()).mount('#app')
