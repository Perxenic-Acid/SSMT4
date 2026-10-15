export const GAME_PRESET_VALUES = [
  'GIMI',
  'HIMI',
  'SRMI',
  'ZZMI',
  'ZZMIDX12',
  'WWMI',
  'EFMI',
  'NTEMI',
  'GF2',
  'IdentityV',
  'AILIMIT',
  'DOAV',
  'SnowBreak',
  'YYSLS',
  'APMI',
  'Naraka',
  'NarakaM',
  'CAMI',
] as const

export type GamePreset = (typeof GAME_PRESET_VALUES)[number]

/** English fallback only. Player-facing UI names live in i18n locale files. */
export const GAME_PRESET_DISPLAY_NAME_EN: Readonly<Record<GamePreset, string>> = {
  GIMI: 'Genshin Impact', HIMI: 'Honkai Impact 3rd', SRMI: 'Honkai: Star Rail',
  ZZMI: 'Zenless Zone Zero', ZZMIDX12: 'Zenless Zone Zero (DirectX 12)',
  WWMI: 'Wuthering Waves', EFMI: 'Arknights: Endfield', NTEMI: 'Neverness to Everness',
  GF2: 'Girls\' Frontline 2: Exilium', IdentityV: 'Identity V', AILIMIT: 'AI LIMIT',
  DOAV: 'Dead or Alive Xtreme: Venus Vacation', SnowBreak: 'Snowbreak: Containment Zone',
  YYSLS: 'Where Winds Meet', APMI: 'Azur Promilia', Naraka: 'NARAKA: BLADEPOINT',
  NarakaM: 'NARAKA: BLADEPOINT Mobile',
  CAMI: 'Calabiyau (Strinova)',
}

export type GamePresetTranslator = (key: string) => string

export function getGamePresetDisplayName(value: string | null | undefined, translate?: GamePresetTranslator): string {
  const normalized = (value || '').trim()
  if (!GAME_PRESET_SET.has(normalized)) return normalized
  const displayName = translate?.(`gameNames.${normalized}`) || GAME_PRESET_DISPLAY_NAME_EN[normalized as GamePreset]
  return `${displayName}(${normalized})`
}

const GAME_PRESET_SET: ReadonlySet<string> = new Set(GAME_PRESET_VALUES)

export const MIHOYO_GAME_PRESET_VALUES = [
  'GIMI',
  'HIMI',
  'SRMI',
  'ZZMI',
  'ZZMIDX12',
] as const

const MIHOYO_GAME_PRESET_SET: ReadonlySet<string> = new Set(MIHOYO_GAME_PRESET_VALUES)

export const getGamePresetOptions = (translate?: GamePresetTranslator): ReadonlyArray<{ label: string; value: GamePreset }> =>
  GAME_PRESET_VALUES.map(preset => ({
    label: getGamePresetDisplayName(preset, translate),
    value: preset,
  }))

/** @deprecated Use getGamePresetOptions(t) in player-facing UI. */
export const GAME_PRESET_OPTIONS = getGamePresetOptions()

export function isMihoyoGamePreset(gamePreset: unknown): boolean {
  const normalizedPreset = typeof gamePreset === 'string' ? gamePreset.trim().toUpperCase() : ''
  return MIHOYO_GAME_PRESET_SET.has(normalizedPreset)
}

export const AUTO_UPDATE_SUPPORTED_PRESETS = [
  'GIMI',
  'HIMI',
  'SRMI',
  'ZZMI',
  'ZZMIDX12',
  'NTEMI',
  'WWMI',
  'CAMI',
] as const

export const AUTO_UPDATE_SUPPORTED_PRESET_SET: ReadonlySet<string> =
  new Set(AUTO_UPDATE_SUPPORTED_PRESETS)

export const GAME_PRESET_GITHUB_REPO_MAP: Readonly<Record<GamePreset, string>> = {
  GIMI: 'SilentNightSound/GIMI-Package',
  HIMI: 'leotorrez/HIMI-Package',
  SRMI: 'SpectrumQT/SRMI-Package',
  ZZMI: 'leotorrez/ZZMI-Package',
  ZZMIDX12: 'StarBobis/MinBase-Package',
  WWMI: 'SpectrumQT/WWMI-Package',
  EFMI: 'SpectrumQT/EFMI-Package',
  
  //异环暂时用222221的包
  NTEMI: 'ssice-a/NTMI-PACKAGE',
  
  GF2: 'SilentNightSound/GIMI-Package',
  IdentityV: 'StarBobis/MinBase-Package',
  AILIMIT: 'StarBobis/MinBase-Package',
  DOAV: 'StarBobis/MinBase-Package',
  SnowBreak: 'StarBobis/MinBase-Package',
  YYSLS: 'StarBobis/MinBase-Package',
  APMI: 'StarBobis/MinBase-Package',
  Naraka: 'StarBobis/NBP-Package',
  NarakaM: 'StarBobis/NBPM-Package',
  CAMI: 'hello2565/CAMI-Package',
}

export function getGithubRepoByGamePreset(gamePreset: string): string | null {
  if (Object.prototype.hasOwnProperty.call(GAME_PRESET_GITHUB_REPO_MAP, gamePreset)) {
    return GAME_PRESET_GITHUB_REPO_MAP[gamePreset as GamePreset]
  }
  return null
}

export function resolveGamePresetByGameName(gameName: string): GamePreset | null {
  if (GAME_PRESET_SET.has(gameName)) {
    return gameName as GamePreset
  }
  return null
}

export function isValidGamePreset(gamePreset: string | null | undefined): boolean {
  const normalizedPreset = (gamePreset || '').trim()
  if (!normalizedPreset) {
    return false
  }
  return GAME_PRESET_SET.has(normalizedPreset)
}
