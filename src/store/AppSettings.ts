import {
	migrateLegacyXianZunProvider,
	normalizeXianZunProvider,
	type XianZunProvider,
} from './XianZunProviders'
import { normalizeAppThemeMode, type AppThemeMode } from './AppTheme'

export enum BGType {
	Image = 'Image',
	Video = 'Video',
}

export enum SSMTLocale {
	de = 'de',
	en = 'en',
	es = 'es',
	fr = 'fr',
	it = 'it',
	ja = 'ja',
	ko = 'ko',
	ru = 'ru',
	zhs = 'zhs',
	zht = 'zht',
}

export const SSMT_LOCALE_OPTIONS = [
	{ value: SSMTLocale.de, label: 'Deutsch', badge: 'DE' },
	{ value: SSMTLocale.en, label: 'English', badge: 'EN' },
	{ value: SSMTLocale.es, label: 'Español', badge: 'ES' },
	{ value: SSMTLocale.fr, label: 'Français', badge: 'FR' },
	{ value: SSMTLocale.it, label: 'Italiano', badge: 'IT' },
	{ value: SSMTLocale.ja, label: '日本語', badge: '日' },
	{ value: SSMTLocale.ko, label: '한국어', badge: '한' },
	{ value: SSMTLocale.ru, label: 'Русский', badge: 'RU' },
	{ value: SSMTLocale.zhs, label: '简体中文', badge: '中' },
	{ value: SSMTLocale.zht, label: '繁體中文', badge: '繁' },
] as const

const normalizeLocale = (value: unknown): SSMTLocale => {
	const locale = SSMT_LOCALE_OPTIONS.find(option => option.value === value)?.value
	if (locale) return locale
	return SSMTLocale.en
}

export type TextureMarkStylePreference = 'Hash' | 'Slot' | 'SharedSlot'
export type PostProcessPreviewLightingMode = 'half-lambert' | 'unlit' | 'pbr'
export type ImageBlurMode = 'all' | 'nsfw' | 'none'
export type GameLaunchMode = 'always-pure' | 'ctrl-pure' | 'always-normal'
export type ModelExtractionLogLanguage = 'zh-CN' | 'en'
export type GameBananaTranslationProvider = 'openai' | 'compatible' | 'claude' | 'deepseek' | 'gemini' | 'google'
export type GameBananaTranslationFontStyle = 'regular' | 'italic' | 'bold' | 'bold-italic'
export type GameBananaTranslationFailureMode = 'retry' | 'message' | 'silent'
export type OptionalPageId = 'work' | 'markTexture' | 'textureModMaker' | 'mods' | 'gameBanana' | 'nexusMods' | 'xianzun' | 'uiBuilder'
export type PageVisibilitySettings = Record<OptionalPageId, boolean>

export const DEFAULT_PAGE_VISIBILITY: PageVisibilitySettings = {
	work: true,
	markTexture: true,
	textureModMaker: true,
	mods: true,
	gameBanana: true,
	nexusMods: true,
	xianzun: true,
	uiBuilder: true,
}

export const APP_UI_SCALE_MIN = 0.7
export const APP_UI_SCALE_MAX = 1.2
export const APP_UI_SCALE_STEP = 0.05

export const normalizeAppUiScale = (value: unknown): number => {
	if (value === null || value === undefined || value === '') {
		return 1
	}

	const numericValue = typeof value === 'number' ? value : Number(value)
	if (!Number.isFinite(numericValue)) {
		return 1
	}

	const clampedValue = Math.min(APP_UI_SCALE_MAX, Math.max(APP_UI_SCALE_MIN, numericValue))
	return Math.round(clampedValue / APP_UI_SCALE_STEP) * APP_UI_SCALE_STEP
}

const normalizeWorkspaceAccessProxyPort = (value: unknown): number => {
	const numericValue = typeof value === 'number' ? value : Number(value)
	if (!Number.isInteger(numericValue) || numericValue < 1 || numericValue > 65535) {
		return 0
	}
	return numericValue
}

const normalizeWorkspaceAccessAttribution = (value: unknown): string =>
	typeof value === 'string' ? value.trim().slice(0, 128) : ''

const normalizeTextureMarkStylePreference = (
	value: unknown
): TextureMarkStylePreference => {
	if (value === 'Slot') return 'Slot'
	if (value === 'SharedSlot') return 'SharedSlot'
	return 'Hash'
}

const normalizeImageBlurMode = (value: unknown, legacyBlurNsfw?: unknown): ImageBlurMode => {
	if (value === 'all' || value === 'nsfw' || value === 'none') return value
	if (value === 'show') return 'none'
	if (value === 'blur' || value === 'hide') return 'nsfw'
	if (typeof legacyBlurNsfw === 'boolean') return legacyBlurNsfw ? 'nsfw' : 'none'
	return 'nsfw'
}

const normalizeGameBananaTranslationProvider = (value: unknown): GameBananaTranslationProvider => {
	return value === 'compatible' || value === 'claude' || value === 'deepseek' || value === 'gemini' || value === 'google'
		? value
		: 'openai'
}

const REASONING_EFFORTS = ['auto', 'off', 'low', 'medium', 'high', 'max'] as const

export type XianZunReasoningEffort = (typeof REASONING_EFFORTS)[number]

export const REASONING_EFFORT_OPTIONS = [...REASONING_EFFORTS]

const XIANZUN_APPROVAL_MODES = ['manual', 'auto', 'none'] as const

export type XianZunApprovalMode = (typeof XIANZUN_APPROVAL_MODES)[number]

export const XIANZUN_APPROVAL_MODE_OPTIONS = [...XIANZUN_APPROVAL_MODES]

const normalizeReasoningEffort = (value: unknown): XianZunReasoningEffort =>
	typeof value === 'string' && (REASONING_EFFORTS as readonly string[]).includes(value)
		? (value as XianZunReasoningEffort)
		: 'auto'

const normalizeXianZunApprovalMode = (value: unknown): XianZunApprovalMode =>
	typeof value === 'string' && (XIANZUN_APPROVAL_MODES as readonly string[]).includes(value)
		? (value as XianZunApprovalMode)
		: 'manual'

const XIANZUN_API_URL_HISTORY_MAX = 12

const normalizeXianzunApiUrlHistory = (value: unknown): string[] => {
	if (!Array.isArray(value)) return []
	const seen = new Set<string>()
	const normalized: string[] = []
	for (const raw of value) {
		if (typeof raw !== 'string') continue
		const url = raw.trim()
		if (!url || !/^https?:\/\//i.test(url)) continue
		const key = url.replace(/\/+$/, '')
		if (seen.has(key)) continue
		seen.add(key)
		normalized.push(key)
		if (normalized.length >= XIANZUN_API_URL_HISTORY_MAX) break
	}
	return normalized
}

export const pushXianzunApiUrlHistory = (history: string[], url: string): string[] => {
	const trimmed = url.trim().replace(/\/+$/, '')
	if (!trimmed || !/^https?:\/\//i.test(trimmed)) return history
	const next = [trimmed, ...history.filter((item) => item !== trimmed)]
	return next.slice(0, XIANZUN_API_URL_HISTORY_MAX)
}

const normalizeXianzunMaxToolRounds = (value: unknown): number => {
	const n = typeof value === 'number' ? value : Number(value)
	if (!Number.isFinite(n)) return 20
	return Math.min(200, Math.max(1, Math.trunc(n)))
}

const normalizeSidebarGameOrder = (value: unknown): string[] => {
	if (!Array.isArray(value)) {
		return []
	}

	const seen = new Set<string>()
	const normalized: string[] = []

	for (const item of value) {
		if (typeof item !== 'string') {
			continue
		}

		const name = item.trim()
		if (!name || seen.has(name)) {
			continue
		}

		seen.add(name)
		normalized.push(name)
	}

	return normalized
}

const normalizeWorkspaceByGame = (value: unknown): Record<string, string> => {
	if (!value || typeof value !== 'object' || Array.isArray(value)) {
		return {}
	}

	const normalized: Record<string, string> = {}

	for (const [rawKey, rawValue] of Object.entries(value as Record<string, unknown>)) {
		if (typeof rawKey !== 'string' || typeof rawValue !== 'string') {
			continue
		}

		const key = rawKey.trim()
		const workspaceName = rawValue.trim()
		if (!key || !workspaceName) {
			continue
		}

		normalized[key] = workspaceName
	}

	return normalized
}

const parseVersionStringToNumber = (version: string): number => {
	const [majorRaw = '0', minorRaw = '0', patchRaw = '0'] = version
		.split('.')
		.map(part => part.match(/\d+/)?.[0] ?? '0')

	const major = Number.parseInt(majorRaw, 10)
	const minor = Number.parseInt(minorRaw, 10)
	const patch = Number.parseInt(patchRaw, 10)

	if (Number.isNaN(major) || Number.isNaN(minor) || Number.isNaN(patch)) {
		return 0
	}

	return major * 1000 + minor * 100 + patch
}

export class AppSettings {
	private static currentVersionNumber = 0
	VersionNumber:number = AppSettings.CURRENT_VERSION

	private static get CURRENT_VERSION(): number {
		return AppSettings.currentVersionNumber
	}

	static setCurrentVersionNumber(versionNumber: number): void {
		if (!Number.isFinite(versionNumber) || versionNumber <= 0) {
			return
		}
		AppSettings.currentVersionNumber = Math.trunc(versionNumber)
	}

	static getCurrentVersionNumber(): number {
		return AppSettings.currentVersionNumber
	}

	static parseVersionStringToNumber(version: string): number {
		return parseVersionStringToNumber(version)
	}

	bgType: BGType = BGType.Image
	bgImage: string = ''
	bgVideo: string = ''
	bgTheme: string = ''
	appTheme: AppThemeMode = 'dark'
	contentOpacity: number = 0.5
	globalDimMaskStrength: number = 2.5
	uiScale: number = 1
	DBMTWorkFolder: string = ''
	workspaceAccessProxyPort: number = 0
	workspaceAccessAttribution: string = ''
	CurrentGameName: string = 'Default'
	githubToken: string = ''
	includePrereleaseUpdates: boolean = true
	coreVersion: string = ''
	coreReleaseDescription: string = ''
	coreVersionDev: string = ''
	coreReleaseDescriptionDev: string = ''
	coreVersionIdentityVDev: string = ''
	coreReleaseDescriptionIdentityVDev: string = ''
	coreVersionPlay: string = ''
	coreReleaseDescriptionPlay: string = ''
	coreVersionSsiceA: string = ''
	coreReleaseDescriptionSsiceA: string = ''
	ReverseOutputFolder: string = ''
	ReversedWorkSpaceName: string = 'Default'
	DRMSingleIniPath: string = ''
	DRMResSPath: string = ''
	DRMAclFolderPath: string = ''
	DRMTargetFolderPath: string = ''
	textureMarkStylePreference: TextureMarkStylePreference = 'Hash'
	postProcessPreviewLightingMode: PostProcessPreviewLightingMode = 'half-lambert'
	locale: SSMTLocale = SSMTLocale.en
	windowWidth?: number = 1280
	windowHeight?: number = 720
	CurrentWorkSpace: string = 'Default'
	CurrentWorkSpaceByGame: Record<string, string> = {}
	sidebarGameOrder: string[] = []
	/** @deprecated Read only while migrating older settings. */
	modsManagementBlurNsfw?: boolean
	/** @deprecated Read only while migrating older settings. */
	gamebananaNsfwMode?: 'show' | 'blur' | 'hide'
	/** @deprecated Read only while migrating older settings. */
	gamebananaNsfwBlur?: boolean
	/** @deprecated Read only while migrating older settings. */
	gamebananaHideNsfw?: boolean
	modsManagementBlurMode: ImageBlurMode = 'nsfw'
	gamebananaBlurMode: ImageBlurMode = 'nsfw'
	gamebananaShowNsfw: boolean = true
	gamebananaRestoreNsfwAfterHide: boolean = false
	revealBlurredImagesOnHover: boolean = true
	titlebarCompactView: boolean = false
	gamebananaTranslationEnabled: boolean = true
	gamebananaTranslationRichText: boolean = false
	gamebananaTranslationProvider: GameBananaTranslationProvider = 'openai'
	gamebananaTranslationApiUrl: string = 'https://api.openai.com/v1'
	gamebananaTranslationApiKey: string = ''
	gamebananaTranslationModel: string = 'gpt-4o-mini'
	gamebananaTranslationTargetLanguage: string = '简体中文'
	gamebananaTranslationShortcut: string = 'Ctrl'
	gamebananaTranslationUseContext: boolean = true
	gamebananaTranslationFontFamily: string = ''
	gamebananaTranslationFontSize: string = ''
	gamebananaTranslationColor: string = ''
	gamebananaTranslationFontStyle: GameBananaTranslationFontStyle = 'regular'
	gamebananaTranslationFailureMode: GameBananaTranslationFailureMode = 'message'
	showWindowShortcutEnabled: boolean = true
	showHomeLauncherContent: boolean = true
	showHomeGameNews: boolean = true
	gameLaunchMode: GameLaunchMode = 'ctrl-pure'
	modelExtractionLogLanguage: ModelExtractionLogLanguage = 'zh-CN'
	pageVisibility: PageVisibilitySettings = { ...DEFAULT_PAGE_VISIBILITY }
	// Nexus Mods uses a per-user API key and a game URL-domain (for example,
	// "skyrimspecialedition"), rather than GameBanana's numeric game ID.
	nexusModsApiKey: string = ''
	nexusModsGameDomain: string = ''
	// CheeseCat (芝士猫) — AI chat agent. OpenAI-compatible endpoint, so it
	// works with DeepSeek out of the box and any compatible provider.
	xianzunApiKey: string = ''
	xianzunApiUrl: string = 'https://api.deepseek.com/v1'
	xianzunModel: string = 'deepseek-v4-flash'
	xianzunApiUrlHistory: string[] = []
	xianzunProviders: XianZunProvider[] = []
	xianzunActiveProviderId: string = ''
	xianzunSystemPrompt: string = ''
	xianzunReasoningEffort: XianZunReasoningEffort = 'auto'
	xianzunApprovalMode: XianZunApprovalMode = 'manual'
	xianzunNsfwBlur: boolean = true
	xianzunMaxToolRounds: number = 20

	constructor(init?: Partial<AppSettings>) {
		if (init) {
			Object.assign(this, init)
		}

		// Ensure defaults for fields that may be undefined/null after assignment
		this.bgType = init?.bgType ?? this.bgType
		this.bgImage = init?.bgImage ?? this.bgImage
		this.bgVideo = init?.bgVideo ?? this.bgVideo
		this.bgTheme = init?.bgTheme ?? this.bgTheme
		this.appTheme = normalizeAppThemeMode(init?.appTheme)
		this.contentOpacity = init?.contentOpacity ?? this.contentOpacity

		this.globalDimMaskStrength = init?.globalDimMaskStrength ?? this.globalDimMaskStrength
		this.uiScale = normalizeAppUiScale(init?.uiScale)
		this.DBMTWorkFolder = init?.DBMTWorkFolder ?? this.DBMTWorkFolder
		this.workspaceAccessProxyPort = normalizeWorkspaceAccessProxyPort(init?.workspaceAccessProxyPort)
		this.workspaceAccessAttribution = normalizeWorkspaceAccessAttribution(init?.workspaceAccessAttribution)
		this.CurrentGameName = init?.CurrentGameName ?? this.CurrentGameName
		this.githubToken = init?.githubToken ?? this.githubToken
		this.includePrereleaseUpdates = init?.includePrereleaseUpdates ?? this.includePrereleaseUpdates
		this.coreVersion = init?.coreVersion ?? this.coreVersion
		this.coreReleaseDescription = init?.coreReleaseDescription ?? this.coreReleaseDescription
		this.coreVersionDev = init?.coreVersionDev ?? this.coreVersion
		this.coreReleaseDescriptionDev = init?.coreReleaseDescriptionDev ?? this.coreReleaseDescription
		this.coreVersionIdentityVDev = init?.coreVersionIdentityVDev ?? this.coreVersionIdentityVDev
		this.coreReleaseDescriptionIdentityVDev = init?.coreReleaseDescriptionIdentityVDev ?? this.coreReleaseDescriptionIdentityVDev
		this.coreVersionPlay = init?.coreVersionPlay ?? this.coreVersionPlay
		this.coreReleaseDescriptionPlay = init?.coreReleaseDescriptionPlay ?? this.coreReleaseDescriptionPlay
		this.coreVersionSsiceA = init?.coreVersionSsiceA ?? this.coreVersionSsiceA
		this.coreReleaseDescriptionSsiceA = init?.coreReleaseDescriptionSsiceA ?? this.coreReleaseDescriptionSsiceA
		this.ReverseOutputFolder = init?.ReverseOutputFolder ?? this.ReverseOutputFolder
		this.ReversedWorkSpaceName = init?.ReversedWorkSpaceName ?? this.ReversedWorkSpaceName
		this.DRMSingleIniPath = init?.DRMSingleIniPath ?? this.DRMSingleIniPath
		this.DRMResSPath = init?.DRMResSPath ?? this.DRMResSPath
		this.DRMAclFolderPath = init?.DRMAclFolderPath ?? this.DRMAclFolderPath
		this.DRMTargetFolderPath = init?.DRMTargetFolderPath ?? this.DRMTargetFolderPath
		this.textureMarkStylePreference = normalizeTextureMarkStylePreference(
			init?.textureMarkStylePreference
		)
		this.postProcessPreviewLightingMode = ['half-lambert', 'unlit', 'pbr'].includes(init?.postProcessPreviewLightingMode || '')
			? init!.postProcessPreviewLightingMode!
			: 'half-lambert'
		this.locale = normalizeLocale(init?.locale)
		this.windowWidth = init?.windowWidth ?? this.windowWidth
		this.windowHeight = init?.windowHeight ?? this.windowHeight
		this.CurrentWorkSpace = init?.CurrentWorkSpace ?? this.CurrentWorkSpace
		this.CurrentWorkSpaceByGame = normalizeWorkspaceByGame(init?.CurrentWorkSpaceByGame)
		this.sidebarGameOrder = normalizeSidebarGameOrder(init?.sidebarGameOrder)
		this.modsManagementBlurMode = normalizeImageBlurMode(init?.modsManagementBlurMode, init?.modsManagementBlurNsfw)
		this.gamebananaBlurMode = normalizeImageBlurMode(init?.gamebananaBlurMode ?? init?.gamebananaNsfwMode, init?.gamebananaNsfwBlur)
		this.gamebananaShowNsfw = init?.gamebananaShowNsfw ?? !(init?.gamebananaHideNsfw ?? init?.gamebananaNsfwMode === 'hide')
		this.gamebananaRestoreNsfwAfterHide = init?.gamebananaRestoreNsfwAfterHide ?? false
		if (!this.gamebananaShowNsfw && this.gamebananaBlurMode === 'nsfw') {
			this.gamebananaBlurMode = 'none'
			this.gamebananaRestoreNsfwAfterHide = true
		}
		this.revealBlurredImagesOnHover = init?.revealBlurredImagesOnHover ?? this.revealBlurredImagesOnHover
		this.titlebarCompactView = init?.titlebarCompactView ?? this.titlebarCompactView
		this.gamebananaTranslationEnabled = init?.gamebananaTranslationEnabled ?? this.gamebananaTranslationEnabled
		this.gamebananaTranslationRichText = init?.gamebananaTranslationRichText ?? this.gamebananaTranslationRichText
		this.gamebananaTranslationProvider = normalizeGameBananaTranslationProvider(init?.gamebananaTranslationProvider)
		this.gamebananaTranslationApiUrl = init?.gamebananaTranslationApiUrl ?? this.gamebananaTranslationApiUrl
		this.gamebananaTranslationApiKey = init?.gamebananaTranslationApiKey ?? this.gamebananaTranslationApiKey
		this.gamebananaTranslationModel = init?.gamebananaTranslationModel ?? this.gamebananaTranslationModel
		this.gamebananaTranslationTargetLanguage = init?.gamebananaTranslationTargetLanguage ?? this.gamebananaTranslationTargetLanguage
		this.gamebananaTranslationUseContext = init?.gamebananaTranslationUseContext ?? this.gamebananaTranslationUseContext
		this.gamebananaTranslationFontFamily = init?.gamebananaTranslationFontFamily ?? this.gamebananaTranslationFontFamily
		this.gamebananaTranslationFontSize = init?.gamebananaTranslationFontSize ?? this.gamebananaTranslationFontSize
		this.gamebananaTranslationColor = init?.gamebananaTranslationColor ?? this.gamebananaTranslationColor
		this.gamebananaTranslationFontStyle = ['regular', 'italic', 'bold', 'bold-italic'].includes(init?.gamebananaTranslationFontStyle || '') ? init!.gamebananaTranslationFontStyle! : 'regular'
		this.gamebananaTranslationFailureMode = ['retry', 'message', 'silent'].includes(init?.gamebananaTranslationFailureMode || '') ? init!.gamebananaTranslationFailureMode! : 'message'
		const savedTranslationShortcut = init?.gamebananaTranslationShortcut?.trim()
		this.gamebananaTranslationShortcut = !savedTranslationShortcut || savedTranslationShortcut.toLowerCase() === 'ctrl+shift+t'
			? 'Ctrl'
			: savedTranslationShortcut
		this.showWindowShortcutEnabled = init?.showWindowShortcutEnabled ?? this.showWindowShortcutEnabled
		this.showHomeLauncherContent = init?.showHomeLauncherContent ?? this.showHomeLauncherContent
		this.showHomeGameNews = init?.showHomeGameNews ?? this.showHomeGameNews
		this.gameLaunchMode = ['always-pure', 'ctrl-pure', 'always-normal'].includes(init?.gameLaunchMode || '')
			? init!.gameLaunchMode!
			: this.gameLaunchMode
		this.modelExtractionLogLanguage = init?.modelExtractionLogLanguage === 'en' ? 'en' : 'zh-CN'
		this.pageVisibility = { ...DEFAULT_PAGE_VISIBILITY, ...(init?.pageVisibility || {}) }
		this.nexusModsApiKey = init?.nexusModsApiKey ?? this.nexusModsApiKey
		this.nexusModsGameDomain = init?.nexusModsGameDomain?.trim().toLowerCase() ?? this.nexusModsGameDomain
		this.xianzunApiKey = init?.xianzunApiKey ?? this.xianzunApiKey
		this.xianzunApiUrl = init?.xianzunApiUrl?.trim() || this.xianzunApiUrl
		// Normalize legacy model names (deepseek-flash / deepseek-pro) to the
		// current v4 naming so persisted settings keep working.
		const savedXianzunModel = init?.xianzunModel?.trim()
		this.xianzunModel =
			savedXianzunModel === 'deepseek-flash' || savedXianzunModel === 'deepseek-pro'
				? savedXianzunModel === 'deepseek-pro'
					? 'deepseek-v4-pro'
					: 'deepseek-v4-flash'
				: savedXianzunModel || this.xianzunModel
		this.xianzunProviders = Array.isArray(init?.xianzunProviders)
			? init.xianzunProviders
				.map(normalizeXianZunProvider)
				.filter((provider): provider is XianZunProvider => provider !== null)
			: []
		if (this.xianzunProviders.length === 0) {
			this.xianzunProviders = [
				migrateLegacyXianZunProvider(this.xianzunApiUrl, this.xianzunApiKey, this.xianzunModel),
			]
		}
		// 用户手动使用过的 API 地址历史（最近使用在前），用于设置页快速回填。
		this.xianzunApiUrlHistory = normalizeXianzunApiUrlHistory(init?.xianzunApiUrlHistory)
		this.xianzunActiveProviderId =
			this.xianzunProviders.some((provider) => provider.id === init?.xianzunActiveProviderId)
				? init!.xianzunActiveProviderId!
				: this.xianzunProviders[0].id
		const activeXianZunProvider = this.xianzunProviders.find(
			(provider) => provider.id === this.xianzunActiveProviderId
		)!
		this.xianzunApiKey = activeXianZunProvider.apiKey
		this.xianzunApiUrl = activeXianZunProvider.baseUrl
		this.xianzunModel = activeXianZunProvider.model
		this.xianzunSystemPrompt = init?.xianzunSystemPrompt ?? this.xianzunSystemPrompt
		this.xianzunReasoningEffort = normalizeReasoningEffort(init?.xianzunReasoningEffort)
		this.xianzunApprovalMode = normalizeXianZunApprovalMode(init?.xianzunApprovalMode)
		this.xianzunNsfwBlur = init?.xianzunNsfwBlur ?? this.xianzunNsfwBlur
		this.xianzunMaxToolRounds = normalizeXianzunMaxToolRounds(init?.xianzunMaxToolRounds)
		// VersionNumber is always controlled by current app code version,
		// not by persisted settings.json.
		this.VersionNumber = AppSettings.CURRENT_VERSION
	}

	static fromJSON(raw?: Partial<AppSettings>): AppSettings {
		const source = raw || {}
		const mapped: Partial<AppSettings> = {
			...source,
		}

		return new AppSettings(mapped)
	}

	toJSON(): Record<string, unknown> {
		return {
			VersionNumber: this.VersionNumber,
			bgType: this.bgType,
			bgImage: this.bgImage,
			bgVideo: this.bgVideo,
			bgTheme: this.bgTheme,
			appTheme: this.appTheme,
			contentOpacity: this.contentOpacity,
			globalDimMaskStrength: this.globalDimMaskStrength,
			uiScale: this.uiScale,
			DBMTWorkFolder: this.DBMTWorkFolder,
			workspaceAccessProxyPort: normalizeWorkspaceAccessProxyPort(this.workspaceAccessProxyPort),
			workspaceAccessAttribution: normalizeWorkspaceAccessAttribution(this.workspaceAccessAttribution),
			CurrentGameName: this.CurrentGameName,
			githubToken: this.githubToken,
			includePrereleaseUpdates: this.includePrereleaseUpdates,
			coreVersion: this.coreVersion,
			coreReleaseDescription: this.coreReleaseDescription,
			coreVersionDev: this.coreVersionDev,
			coreReleaseDescriptionDev: this.coreReleaseDescriptionDev,
			coreVersionIdentityVDev: this.coreVersionIdentityVDev,
			coreReleaseDescriptionIdentityVDev: this.coreReleaseDescriptionIdentityVDev,
			coreVersionPlay: this.coreVersionPlay,
			coreReleaseDescriptionPlay: this.coreReleaseDescriptionPlay,
			coreVersionSsiceA: this.coreVersionSsiceA,
			coreReleaseDescriptionSsiceA: this.coreReleaseDescriptionSsiceA,
			ReverseOutputFolder: this.ReverseOutputFolder,
			ReversedWorkSpaceName: this.ReversedWorkSpaceName,
			DRMSingleIniPath: this.DRMSingleIniPath,
			DRMResSPath: this.DRMResSPath,
			DRMAclFolderPath: this.DRMAclFolderPath,
			DRMTargetFolderPath: this.DRMTargetFolderPath,
			textureMarkStylePreference: this.textureMarkStylePreference,
			postProcessPreviewLightingMode: this.postProcessPreviewLightingMode,
			locale: this.locale,
			windowWidth: this.windowWidth,
			windowHeight: this.windowHeight,
			CurrentWorkSpace: this.CurrentWorkSpace,
			CurrentWorkSpaceByGame: this.CurrentWorkSpaceByGame,
			sidebarGameOrder: this.sidebarGameOrder,
			modsManagementBlurMode: this.modsManagementBlurMode,
			gamebananaBlurMode: this.gamebananaBlurMode,
			gamebananaShowNsfw: this.gamebananaShowNsfw,
			gamebananaRestoreNsfwAfterHide: this.gamebananaRestoreNsfwAfterHide,
			revealBlurredImagesOnHover: this.revealBlurredImagesOnHover,
			titlebarCompactView: this.titlebarCompactView,
			gamebananaTranslationEnabled: this.gamebananaTranslationEnabled,
			gamebananaTranslationRichText: this.gamebananaTranslationRichText,
			gamebananaTranslationProvider: this.gamebananaTranslationProvider,
			gamebananaTranslationApiUrl: this.gamebananaTranslationApiUrl,
			gamebananaTranslationApiKey: this.gamebananaTranslationApiKey,
			gamebananaTranslationModel: this.gamebananaTranslationModel,
			gamebananaTranslationTargetLanguage: this.gamebananaTranslationTargetLanguage,
			gamebananaTranslationShortcut: this.gamebananaTranslationShortcut,
			gamebananaTranslationUseContext: this.gamebananaTranslationUseContext,
			gamebananaTranslationFontFamily: this.gamebananaTranslationFontFamily,
			gamebananaTranslationFontSize: this.gamebananaTranslationFontSize,
			gamebananaTranslationColor: this.gamebananaTranslationColor,
			gamebananaTranslationFontStyle: this.gamebananaTranslationFontStyle,
			gamebananaTranslationFailureMode: this.gamebananaTranslationFailureMode,
			showWindowShortcutEnabled: this.showWindowShortcutEnabled,
			showHomeLauncherContent: this.showHomeLauncherContent,
			showHomeGameNews: this.showHomeGameNews,
			gameLaunchMode: this.gameLaunchMode,
			modelExtractionLogLanguage: this.modelExtractionLogLanguage,
			pageVisibility: { ...this.pageVisibility },
			nexusModsApiKey: this.nexusModsApiKey,
			nexusModsGameDomain: this.nexusModsGameDomain,
			xianzunApiKey: this.xianzunApiKey,
			xianzunApiUrl: this.xianzunApiUrl,
			xianzunModel: this.xianzunModel,
			xianzunApiUrlHistory: this.xianzunApiUrlHistory,
			xianzunProviders: this.xianzunProviders,
			xianzunActiveProviderId: this.xianzunActiveProviderId,
			xianzunSystemPrompt: this.xianzunSystemPrompt,
			xianzunReasoningEffort: this.xianzunReasoningEffort,
			xianzunApprovalMode: this.xianzunApprovalMode,
			xianzunNsfwBlur: this.xianzunNsfwBlur,
			xianzunMaxToolRounds: this.xianzunMaxToolRounds,
		}
	}
}
