import { defineStore } from 'pinia'
import * as api from '../api'

interface State {
  loaded: boolean
  setupRequired: boolean
  authenticated: boolean
  username: string | null
}

export const useAuthStore = defineStore('auth', {
  state: (): State => ({
    loaded: false,
    setupRequired: false,
    authenticated: false,
    username: null,
  }),
  actions: {
    /// Drop to the login screen whenever any API call returns 401.
    register() {
      api.setUnauthorizedHandler(() => {
        this.authenticated = false
        this.username = null
      })
    },
    async refresh() {
      const s = await api.auth.status()
      this.setupRequired = s.setup_required
      this.authenticated = s.authenticated
      this.username = s.username
      this.loaded = true
    },
    async login(username: string, password: string) {
      const r = await api.auth.login(username, password)
      this.authenticated = true
      this.username = r.username
      this.setupRequired = false
    },
    async setup(username: string, password: string) {
      const r = await api.auth.setup(username, password)
      this.authenticated = true
      this.username = r.username
      this.setupRequired = false
    },
    async logout() {
      await api.auth.logout()
      this.authenticated = false
      this.username = null
    },
  },
})
