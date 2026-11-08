export type WorkspaceTabMeta = {
  id: string
  name: string
}

export const getNextWorkspaceTabIndex = (
  tabs: readonly Pick<WorkspaceTabMeta, 'name'>[],
  prefix: string,
): number => {
  const normalizedPrefix = prefix.toLowerCase()
  let highestIndex = -1

  for (const tab of tabs) {
    const normalizedName = tab.name.trim().toLowerCase()
    if (!normalizedName.startsWith(normalizedPrefix)) continue

    const suffix = normalizedName.slice(normalizedPrefix.length).trim()
    if (!/^\d+$/.test(suffix)) continue
    highestIndex = Math.max(highestIndex, Number.parseInt(suffix, 10))
  }

  return highestIndex + 1
}

export const createWorkspaceTabsAfterClear = (
  prefix: string,
  createId: () => string,
): WorkspaceTabMeta[] => [{
  id: createId(),
  name: `${prefix}0`,
}]
