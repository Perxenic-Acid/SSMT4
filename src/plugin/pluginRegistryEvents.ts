export const PLUGIN_REGISTRY_CHANGED_EVENT = 'ssmt:plugin-registry-changed'

export const notifyPluginRegistryChanged = (): void => {
  window.dispatchEvent(new CustomEvent(PLUGIN_REGISTRY_CHANGED_EVENT))
}
