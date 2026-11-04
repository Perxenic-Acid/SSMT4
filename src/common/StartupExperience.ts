export const STARTUP_VISIBLE_EVENT = 'ssmt:startup-visible'

export function announceStartupVisible() {
  window.dispatchEvent(new Event(STARTUP_VISIBLE_EVENT))
}
