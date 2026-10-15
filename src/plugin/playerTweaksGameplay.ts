export interface PlayerTweaksGameplayConfig {
  fpsUnlock: boolean
  targetFps: number
  fastTeamPage: boolean
}

export const defaultPlayerTweaksGameplay = (): PlayerTweaksGameplayConfig => ({
  fpsUnlock: true,
  targetFps: 120,
  fastTeamPage: true,
})

export const normalizePlayerTweaksGameplay = (value: unknown): PlayerTweaksGameplayConfig => {
  const defaults = defaultPlayerTweaksGameplay()
  const source = value && typeof value === 'object' ? value as Record<string, unknown> : {}
  return {
    fpsUnlock: typeof source.fpsUnlock === 'boolean' ? source.fpsUnlock : defaults.fpsUnlock,
    targetFps: typeof source.targetFps === 'number' && Number.isInteger(source.targetFps) &&
      source.targetFps >= 30 && source.targetFps <= 240 ? source.targetFps : defaults.targetFps,
    fastTeamPage: typeof source.fastTeamPage === 'boolean' ? source.fastTeamPage : defaults.fastTeamPage,
  }
}

export const serializePlayerTweaksGameplay = (value: unknown): string => {
  const gameplay = normalizePlayerTweaksGameplay(value)
  return `\n[Gameplay]\nFpsUnlock=${gameplay.fpsUnlock ? 1 : 0}\nTargetFps=${gameplay.targetFps}\nFastTeamPage=${gameplay.fastTeamPage ? 1 : 0}\n`
}
