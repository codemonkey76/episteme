// Theme switching. Themes are CSS-variable remaps defined in style.css; this
// module sets `data-theme` on <html> and remembers the choice. The selection
// is stored per user on the server and follows the account across devices;
// localStorage is only a fast-start cache so the right theme paints before
// auth resolves. Swatches mirror each theme's generated palette for the
// Appearance picker.

import * as api from './api'

export interface ThemeDef {
  key: string
  label: string
  description: string
  swatch: { bg: string; surface: string; accent: string; text: string }
}

export const THEMES: ThemeDef[] = [
  {
    key: 'graphite',
    label: 'Graphite',
    description: 'The original neutral dark theme.',
    swatch: { bg: '#0f0f0f', surface: '#1a1a1a', accent: '#4a8aff', text: '#e0e0e0' },
  },
  {
    key: 'oled',
    label: 'OLED Black',
    description: 'Pure-black backgrounds for OLED screens.',
    swatch: { bg: '#020202', surface: '#0d0d0d', accent: '#4a8aff', text: '#e0e0e0' },
  },
  {
    key: 'midnight',
    label: 'Midnight',
    description: 'Cool blue-tinted dark.',
    swatch: { bg: '#0c0e12', surface: '#141820', accent: '#4a8aff', text: '#dfdfe1' },
  },
  {
    key: 'forest',
    label: 'Forest',
    description: 'Green-tinted dark with a mint accent.',
    swatch: { bg: '#0c120f', surface: '#14201a', accent: '#4affa8', text: '#dfe1e0' },
  },
  {
    key: 'amethyst',
    label: 'Amethyst',
    description: 'Violet-tinted dark with a purple accent.',
    swatch: { bg: '#0f0c12', surface: '#1a1420', accent: '#a44aff', text: '#e0dfe1' },
  },
  {
    key: 'daylight',
    label: 'Daylight',
    description: 'A light theme for bright rooms.',
    swatch: { bg: '#f0f0f0', surface: '#e5e5e5', accent: '#0040b5', text: '#1f1f1f' },
  },
]

const STORAGE_KEY = 'episteme-theme'
let previewing = false

export function currentTheme(): string {
  const saved = localStorage.getItem(STORAGE_KEY)
  return THEMES.some(t => t.key === saved) ? (saved as string) : 'graphite'
}

export function applyTheme(key: string) {
  if (!THEMES.some(t => t.key === key)) key = 'graphite'
  document.documentElement.setAttribute('data-theme', key)
  localStorage.setItem(STORAGE_KEY, key)
}

/** Apply + persist to the user's account (best-effort). */
export function saveTheme(key: string) {
  applyTheme(key)
  api.settings.setTheme(key).catch(() => {})
}

/** Pull the account's theme after auth resolves. First run (nothing stored
 *  server-side yet) pushes the local choice up instead. */
export async function syncThemeFromServer() {
  if (previewing) return // a ?theme= preview wins for this page load
  try {
    const { theme } = await api.settings.getTheme()
    if (theme && THEMES.some(t => t.key === theme)) {
      if (theme !== currentTheme()) applyTheme(theme)
    } else {
      await api.settings.setTheme(currentTheme())
    }
  } catch {
    /* offline / impersonation edge — keep the cached theme */
  }
}

/** Apply the cached theme before the app mounts (no flash of default).
 *  A `?theme=` URL param previews a theme without persisting it. */
export function initTheme() {
  const param = new URLSearchParams(window.location.search).get('theme')
  if (param && THEMES.some(t => t.key === param)) {
    previewing = true
    document.documentElement.setAttribute('data-theme', param)
    return
  }
  document.documentElement.setAttribute('data-theme', currentTheme())
}
