import { createApp } from 'vue'
import './style.css'
import './mirror.css'
import './features/mirror/mirror-center.css'
import './file-manager.css'
import './features/settings/settings.css'
import './features/cloud/cloud-account.css'
import './resource-admin.css'
import './readability.css'

async function bootstrap() {
  const cloudMode = import.meta.env.MODE === 'cloud' || import.meta.env.VITE_APP_MODE === 'cloud'
  const websiteMode = import.meta.env.MODE === 'website' || import.meta.env.VITE_APP_MODE === 'website'
  const pathname = window.location.pathname.replace(/\/$/, '')
  const rootComponent = websiteMode
    ? (await import('./WebsiteApp.vue')).default
    : cloudMode
    ? (await import('./CloudApp.vue')).default
    : pathname === '/resource-admin'
      ? (await import('./ResourceAdminApp.vue')).default
      : (await import('./App.vue')).default

  createApp(rootComponent).mount('#app')
}

void bootstrap()
