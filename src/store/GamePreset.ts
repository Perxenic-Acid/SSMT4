// ─────────────────────────────────────────────────────────────
// 游戏预设定义
// ─────────────────────────────────────────────────────────────
// 说明:
// - GAME_PRESET_ALL_VALUES 是全量预设表(历史遗留 + 当前保留),
//   仅作为底层数据保留,不直接决定"当前支持"的入口。以后需要恢复
//   某个游戏时,把它加回 GAME_PRESET_VALUES 即可。
// - GAME_PRESET_VALUES 是当前真正参与构建/暴露的支持列表,按优先级
//   排序,顺序决定下拉框与侧边栏的置顶顺序。
// - i18n 的 gameNames 字典保持完整,不随此列表裁剪。
// ─────────────────────────────────────────────────────────────

/** 全量预设表(历史遗留 + 当前保留),底层逻辑与数据保留,便于以后恢复。 */
const GAME_PRESET_ALL_VALUES = [
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

type GamePresetAll = (typeof GAME_PRESET_ALL_VALUES)[number]

/**
 * 当前保留并参与构建的游戏预设,按优先级排序。
 * - 绝对支持: GIMI
 * - 长期支持: WWMI / SRMI / ZZMI / EFMI
 * - 有限支持: NTEMI / HIMI / CAMI / ZZMIDX12
 * - 将会支持: APMI(蓝色星原:旅谣)
 *   (星布谷地 / 无限大 待接入后再加入)
 */
export const GAME_PRESET_VALUES = [
  // 绝对支持
  'GIMI',
  // 长期支持
  'WWMI',
  'SRMI',
  'ZZMI',
  'EFMI',
  // 有限支持
  'NTEMI',
  'HIMI',
  'CAMI',
  'ZZMIDX12',
  // 将会支持
  'APMI',
] as const

export type GamePreset = (typeof GAME_PRESET_VALUES)[number]

/** English fallback only. Player-facing UI names live in i18n locale files. */
export const GAME_PRESET_DISPLAY_NAME_EN: Readonly<Record<GamePresetAll, string>> = {
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
  if (!GAME_PRESET_ALL_SET.has(normalized)) return normalized
  const displayName = translate?.(`gameNames.${normalized}`) || GAME_PRESET_DISPLAY_NAME_EN[normalized as GamePresetAll]
  return `${displayName}(${normalized})`
}

const GAME_PRESET_ALL_SET: ReadonlySet<string> = new Set(GAME_PRESET_ALL_VALUES)
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

/**
 * 返回预设的置顶优先级(数值越小越靠前)。未知/未支持的预设排最后。
 */
export function getGamePresetPriority(gamePreset: string | null | undefined): number {
  const normalized = (gamePreset || '').trim().toUpperCase()
  const index = (GAME_PRESET_VALUES as readonly string[]).indexOf(normalized)
  return index < 0 ? Number.MAX_SAFE_INTEGER : index
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
  'EFMI',
] as const

export const AUTO_UPDATE_SUPPORTED_PRESET_SET: ReadonlySet<string> =
  new Set(AUTO_UPDATE_SUPPORTED_PRESETS)

export const GAME_PRESET_GITHUB_REPO_MAP: Readonly<Record<GamePresetAll, string>> = {
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
    return GAME_PRESET_GITHUB_REPO_MAP[gamePreset as GamePresetAll]
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
