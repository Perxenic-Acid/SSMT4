// Official aliases: https://gamebanana.com/apiv11/Mod/ListFilterConfig
export const GAMEBANANA_SORTS = [
  'Generic_Newest', 'Generic_Oldest', 'Generic_LatestModified',
  'Generic_NewAndUpdated', 'Generic_LatestUpdated',
  'Generic_Alphabetically', 'Generic_ReverseAlphabetically',
  'Generic_MostLiked', 'Generic_MostViewed', 'Generic_MostCommented',
  'Generic_LatestComment', 'Generic_MostDownloaded',
] as const;
export type GameBananaSort = typeof GAMEBANANA_SORTS[number];
export const DEFAULT_GAMEBANANA_SORT: GameBananaSort = 'Generic_LatestUpdated';
const STORAGE_KEY = 'gamebanana:mod-sort:v1';

export function normalizeGameBananaSort(value: unknown): GameBananaSort {
  return GAMEBANANA_SORTS.includes(value as GameBananaSort) ? value as GameBananaSort : DEFAULT_GAMEBANANA_SORT;
}
export function readGameBananaSort(): GameBananaSort {
  try { return normalizeGameBananaSort(localStorage.getItem(STORAGE_KEY)); }
  catch { return DEFAULT_GAMEBANANA_SORT; }
}
export function saveGameBananaSort(value: GameBananaSort): void {
  try { localStorage.setItem(STORAGE_KEY, normalizeGameBananaSort(value)); }
  catch { /* Sorting still works when persistent storage is unavailable. */ }
}
