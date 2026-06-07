// Web Push service worker: shows incoming pushes as notifications and
// focuses (or opens) the app when one is clicked. Payloads are the backend's
// {title, body} JSON.
self.addEventListener('push', (event) => {
  let data = {}
  try {
    data = event.data ? event.data.json() : {}
  } catch {
    data = { body: event.data ? event.data.text() : '' }
  }
  event.waitUntil(
    self.registration.showNotification(data.title || 'Episteme', {
      body: data.body || '',
      icon: '/apple-touch-icon.png',
      badge: '/favicon-32.png',
    }),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  event.waitUntil(
    clients.matchAll({ type: 'window', includeUncontrolled: true }).then((wins) => {
      for (const win of wins) {
        if ('focus' in win) return win.focus()
      }
      return clients.openWindow('/')
    }),
  )
})
