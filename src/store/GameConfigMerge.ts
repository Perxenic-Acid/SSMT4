export function cloneConfigValue<T>(value: T): T {
  if (value === undefined) return value
  return JSON.parse(JSON.stringify(value)) as T
}

export function mergeChangedConfig<T extends object>(
  baseline: T,
  current: T,
  latest: T,
): { config: T; changed: boolean } {
  const baselineRecord = baseline as Record<string, unknown>
  const currentRecord = current as Record<string, unknown>
  const mergedRecord = cloneConfigValue(latest) as Record<string, unknown>
  const keys = new Set([...Object.keys(baselineRecord), ...Object.keys(currentRecord)])
  let changed = false

  for (const key of keys) {
    const currentValue = currentRecord[key]
    const baselineValue = baselineRecord[key]
    if (JSON.stringify(currentValue ?? null) === JSON.stringify(baselineValue ?? null)) continue
    mergedRecord[key] = cloneConfigValue(currentValue)
    changed = true
  }

  return { config: mergedRecord as T, changed }
}
