export function cloneConfigValue<T>(value: T): T {
  if (value === undefined) return value
  return JSON.parse(JSON.stringify(value)) as T
}

export function configValuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left ?? null) === JSON.stringify(right ?? null)
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

function mergeValue(
  baseline: unknown,
  current: unknown,
  latest: unknown,
): { value: unknown; changed: boolean } {
  if (isPlainObject(baseline) && isPlainObject(current) && isPlainObject(latest)) {
    const merged = cloneConfigValue(latest)
    const keys = new Set([...Object.keys(baseline), ...Object.keys(current)])
    let changed = false
    for (const key of keys) {
      const result = mergeValue(baseline[key], current[key], merged[key])
      if (!result.changed) continue
      if (result.value === undefined) {
        delete merged[key]
      } else {
        merged[key] = result.value
      }
      changed = true
    }
    return { value: merged, changed }
  }

  if (configValuesEqual(baseline, current)) {
    return { value: latest, changed: false }
  }
  return { value: cloneConfigValue(current), changed: true }
}

export function mergeChangedConfig<T extends object>(
  baseline: T,
  current: T,
  latest: T,
): { config: T; changed: boolean } {
  const result = mergeValue(baseline, current, latest)
  return { config: result.value as T, changed: result.changed }
}
