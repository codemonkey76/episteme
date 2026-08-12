import { defineStore } from 'pinia'
import { ref } from 'vue'

// Cross-window signal: bumped whenever something changes the shipments (the
// chat AI tracking a new parcel, say) so the Shipments window can refresh live.
export const useShipmentsStore = defineStore('shipments', () => {
  const changeToken = ref(0)
  function notifyChanged() {
    changeToken.value++
  }
  return { changeToken, notifyChanged }
})
