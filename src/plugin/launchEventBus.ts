import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export const LAUNCH_EVENT_NAME = 'launch-event'

export interface LaunchEvent {
  event: string
  pid?: number
  module?: string
  stage?: string
  code?: string
  message?: string
  [key: string]: unknown
}

/** Subscribe to backend launch events without coupling consumers to Vue. */
export const subscribeLaunchEvents = (
  handler: (event: LaunchEvent) => void,
): Promise<UnlistenFn> => listen<LaunchEvent>(LAUNCH_EVENT_NAME, ({ payload }) => handler(payload))
