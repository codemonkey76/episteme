import { defineStore } from 'pinia'
import * as api from '../api'
import { syncThemeFromServer } from '../theme'

interface State {
  loaded: boolean
  setupRequired: boolean
  authenticated: boolean
  username: string | null
  role: 'admin' | 'member' | null
  impersonator: string | null
}

export const useAuthStore = defineStore('auth', {
  state: (): State => ({
    loaded: false,
    setupRequired: false,
    authenticated: false,
    username: null,
    role: null,
    impersonator: null,
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
      this.role = s.role
      this.impersonator = s.impersonator
      this.loaded = true
      // Theme follows the account (covers login, new devices, impersonation).
      if (this.authenticated) void syncThemeFromServer()
    },
    /** 'totp_required' = password accepted but the account has 2FA — call
     *  again with the code from the authenticator app (or a recovery code). */
    async login(username: string, password: string, code?: string): Promise<'ok' | 'totp_required'> {
      const r = await api.auth.login(username, password, code)
      if (r.totp_required) return 'totp_required'
      this.authenticated = true
      this.username = r.username ?? null
      this.setupRequired = false
      await this.refresh() // pick up the role
      return 'ok'
    },
    async setup(username: string, password: string) {
      const r = await api.auth.setup(username, password)
      this.authenticated = true
      this.username = r.username
      this.setupRequired = false
      await this.refresh()
    },
    async registerWithInvite(code: string, username: string, password: string) {
      const r = await api.auth.register(code, username, password)
      this.authenticated = true
      this.username = r.username
      await this.refresh()
    },
    async logout() {
      await api.auth.logout()
      this.authenticated = false
      this.username = null
      this.role = null
      this.impersonator = null
    },
    async impersonate(id: string) {
      await api.users.impersonate(id)
      await this.refresh()
      // Fresh identity → reload so every store refetches as the new user.
      window.location.reload()
    },
    async stopImpersonating() {
      await api.auth.stopImpersonating()
      await this.refresh()
      window.location.reload()
    },
  },
})
