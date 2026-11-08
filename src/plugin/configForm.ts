export interface ConfigField { path: string[]; label: string; description?: string; kind: 'boolean' | 'number'; min?: number; max?: number; step?: number }
export interface ConfigGroup { id: string; title: string; description?: string; fields: ConfigField[] }
export interface ConfigForm { encoding: 'ssmt-config/1'; title: string; description?: string; groups: ConfigGroup[] }
const forbidden = new Set(['__proto__', 'prototype', 'constructor'])
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error('配置描述必须为对象')
  return value as Record<string, unknown>
}
function text(value: unknown, optional = false): string {
  if (optional && (value === undefined || value === '')) return ''
  if (typeof value !== 'string' || !value.trim() || value.length > 2048) throw Error('配置文本无效')
  return value
}
function keys(value: Record<string, unknown>, allowed: string[]) {
  if (Object.keys(value).some(key => !allowed.includes(key))) throw Error('不支持的配置描述字段')
}
// 私有格式只描述控件和数据绑定，不接受脚本、HTML、CSS 或可执行表达式。
export function decodeConfigForm(value: unknown): ConfigForm {
  const form = object(value)
  keys(form, ['encoding', 'title', 'description', 'groups'])
  if (form.encoding !== 'ssmt-config/1') throw Error('不支持的配置编码版本')
  if (!Array.isArray(form.groups) || form.groups.length > 16) throw Error('配置分组数量无效')
  const paths: string[] = [], ids = new Set<string>()
  let count = 0
  const groups = form.groups.map(value => {
    const group = object(value)
    keys(group, ['id', 'title', 'description', 'fields'])
    const id = text(group.id)
    if (ids.has(id)) throw Error('配置分组重复')
    ids.add(id)
    if (!Array.isArray(group.fields)) throw Error('配置字段必须为数组')
    const fields = group.fields.map(value => {
      const field = object(value)
      keys(field, ['path', 'label', 'description', 'kind', 'min', 'max', 'step'])
      if (++count > 128) throw Error('配置字段过多')
      if (!Array.isArray(field.path) || !field.path.length || field.path.length > 8 || field.path.some(key => typeof key !== 'string' || key.length > 128 || !/^[a-zA-Z][\w-]*$/.test(key) || forbidden.has(key))) throw Error('配置绑定路径无效')
      const path = field.path as string[], key = path.join('.')
      if (paths.some(existing => key === existing || key.startsWith(existing + '.') || existing.startsWith(key + '.'))) throw Error('配置绑定路径冲突')
      paths.push(key)
      if (field.kind !== 'boolean' && field.kind !== 'number') throw Error('不支持的配置控件')
      for (const name of ['min', 'max', 'step']) if (field[name] !== undefined && (typeof field[name] !== 'number' || !Number.isFinite(field[name]))) throw Error('数值约束无效')
      if (field.min !== undefined && field.max !== undefined && Number(field.min) > Number(field.max)) throw Error('数值范围无效')
      if (field.step !== undefined && Number(field.step) <= 0) throw Error('数值步长无效')
      return { path, label: text(field.label), description: text(field.description, true), kind: field.kind, min: field.min, max: field.max, step: field.step } as ConfigField
    })
    return { id, title: text(group.title), description: text(group.description, true), fields }
  })
  return { encoding: 'ssmt-config/1', title: text(form.title), description: text(form.description, true), groups }
}
export function readConfigField(values: unknown, field: ConfigField): boolean | number {
  let current = values
  for (const key of field.path) {
    const record = object(current)
    if (!Object.hasOwn(record, key)) throw Error('配置值缺少字段')
    current = record[key]
  }
  if (field.kind === 'boolean' ? typeof current !== 'boolean' : typeof current !== 'number' || !Number.isFinite(current) || field.min !== undefined && current < field.min || field.max !== undefined && current > field.max) throw Error('配置值与描述不匹配')
  return current as boolean | number
}
export function writeConfigField(values: unknown, field: ConfigField, value: boolean | number) {
  let record = object(values)
  for (const key of field.path.slice(0, -1)) record = object(record[key])
  record[field.path.at(-1)!] = value
}
