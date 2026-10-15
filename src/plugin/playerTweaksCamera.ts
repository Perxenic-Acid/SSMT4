export interface PlayerTweaksCameraConfig {
  customFov: boolean
  fov: number
  preserveAimingFov: boolean
  preserveCutsceneFov: boolean
  restoreInUi: boolean
  transitionSpeed: number
  disableInputSmoothing: boolean
  disableTransitionBlend: boolean
  disableCharacterFade: boolean
  disableEventCameraMovement: boolean
  cameraZoom: boolean
}

export const defaultPlayerTweaksCamera = (): PlayerTweaksCameraConfig => ({
  customFov: false,
  fov: 60,
  preserveAimingFov: true,
  preserveCutsceneFov: true,
  restoreInUi: true,
  transitionSpeed: 8,
  disableInputSmoothing: false,
  disableTransitionBlend: false,
  disableCharacterFade: true,
  disableEventCameraMovement: false,
  cameraZoom: false,
})

export const normalizePlayerTweaksCamera = (value: unknown): PlayerTweaksCameraConfig => {
  const defaults = defaultPlayerTweaksCamera()
  const source = value && typeof value === 'object' ? value as Record<string, unknown> : {}
  const bounded = (key: 'fov' | 'transitionSpeed', min: number, max: number) =>
    typeof source[key] === 'number' && Number.isFinite(source[key]) && source[key] >= min && source[key] <= max
      ? source[key] : defaults[key]
  return {
    customFov: typeof source.customFov === 'boolean' ? source.customFov : defaults.customFov,
    fov: bounded('fov', 30, 120),
    preserveAimingFov: typeof source.preserveAimingFov === 'boolean' ? source.preserveAimingFov : defaults.preserveAimingFov,
    preserveCutsceneFov: typeof source.preserveCutsceneFov === 'boolean' ? source.preserveCutsceneFov : defaults.preserveCutsceneFov,
    restoreInUi: typeof source.restoreInUi === 'boolean' ? source.restoreInUi : defaults.restoreInUi,
    transitionSpeed: bounded('transitionSpeed', 0, 30),
    disableInputSmoothing: typeof source.disableInputSmoothing === 'boolean' ? source.disableInputSmoothing : defaults.disableInputSmoothing,
    disableTransitionBlend: typeof source.disableTransitionBlend === 'boolean' ? source.disableTransitionBlend : defaults.disableTransitionBlend,
    disableCharacterFade: typeof source.disableCharacterFade === 'boolean' ? source.disableCharacterFade : defaults.disableCharacterFade,
    disableEventCameraMovement: typeof source.disableEventCameraMovement === 'boolean' ? source.disableEventCameraMovement : defaults.disableEventCameraMovement,
    cameraZoom: typeof source.cameraZoom === 'boolean' ? source.cameraZoom : defaults.cameraZoom,
  }
}

export const serializePlayerTweaksCamera = (value: unknown): string => {
  const camera = normalizePlayerTweaksCamera(value)
  const bool = (enabled: boolean) => enabled ? 1 : 0
  return `[Camera]\nCustomFov=${bool(camera.customFov)}\nFov=${camera.fov}\nPreserveAimingFov=${bool(camera.preserveAimingFov)}\nPreserveCutsceneFov=${bool(camera.preserveCutsceneFov)}\nRestoreInUi=${bool(camera.restoreInUi)}\nTransitionSpeed=${camera.transitionSpeed}\nDisableInputSmoothing=${bool(camera.disableInputSmoothing)}\nDisableTransitionBlend=${bool(camera.disableTransitionBlend)}\nDisableCharacterFade=${bool(camera.disableCharacterFade)}\nDisableEventCameraMovement=${bool(camera.disableEventCameraMovement)}\nCameraZoom=${bool(camera.cameraZoom)}\n`
}
