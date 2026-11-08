<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { requestPluginConfigForm } from '../pageRuntime'
import { decodeConfigForm, readConfigField, writeConfigField, type ConfigForm, type ConfigField } from '../configForm'
const props = defineProps<{ pluginId: string; pageId: string; gameName: string; enabled: boolean }>()
const schema = ref<ConfigForm | null>(null), values = ref<unknown>(null)
const loading = ref(false), saving = ref(false), error = ref(''), saved = ref(false)
let alive = true
onBeforeUnmount(() => { alive = false })
const request = (action: 'schema' | 'read' | 'save', payload: unknown = null) => requestPluginConfigForm(props.pluginId, props.pageId, props.gameName, action, payload)
function validate(form: ConfigForm, value: unknown) { for (const group of form.groups) for (const field of group.fields) readConfigField(value, field) }
async function load() {
  loading.value = true; error.value = ''; saved.value = false
  try {
    const form = decodeConfigForm(await request('schema'))
    const data = await request('read')
    validate(form, data)
    if (alive) { schema.value = form; values.value = data }
  } catch (reason) { if (alive) error.value = String(reason) }
  finally { if (alive) loading.value = false }
}
function update(field: ConfigField, value: boolean | number | undefined) {
  if (value === undefined) return
  writeConfigField(values.value, field, value); saved.value = false
}
async function save() {
  if (!schema.value || !props.enabled) return
  saving.value = true; error.value = ''; saved.value = false
  try {
    validate(schema.value, values.value)
    const data = await request('save', values.value)
    validate(schema.value, data)
    if (alive) { values.value = data; saved.value = true }
  } catch (reason) { if (alive) error.value = String(reason) }
  finally { if (alive) saving.value = false }
}
onMounted(load)
</script>
<template>
  <form class="config-form" @submit.prevent="save" v-loading="loading">
    <header v-if="schema">
      <h3>{{ schema.title }}</h3><p v-if="schema.description">{{ schema.description }}</p>
    </header>
    <p v-if="error" class="config-error" role="alert">{{ error }}</p>
    <template v-if="schema && values !== null">
      <section v-for="group in schema.groups" :key="group.id" class="config-group">
        <h4>{{ group.title }}</h4><p v-if="group.description">{{ group.description }}</p>
        <div v-for="field in group.fields" :key="field.path.join('.')" class="config-row">
          <label :id="`label-${group.id}-${field.path.join('-')}`"><span>{{ field.label }}</span><small v-if="field.description">{{ field.description }}</small></label>
          <el-switch v-if="field.kind === 'boolean'" :aria-labelledby="`label-${group.id}-${field.path.join('-')}`" :model-value="readConfigField(values, field) as boolean" :disabled="!enabled || saving || loading" @update:model-value="update(field, Boolean($event))" />
          <el-input-number v-else :aria-labelledby="`label-${group.id}-${field.path.join('-')}`" :model-value="readConfigField(values, field) as number" :min="field.min" :max="field.max" :step="field.step || 1" :disabled="!enabled || saving || loading" size="small" controls-position="right" @update:model-value="update(field, $event)" />
        </div>
      </section>
      <footer><el-button native-type="submit" type="primary" :disabled="!enabled || loading" :loading="saving">保存设置</el-button><span v-if="saved" role="status">设置已保存，下次启动时生效。</span></footer>
    </template>
    <el-button v-else-if="!loading" @click="load">重新读取</el-button>
  </form>
</template>
<style scoped>
.config-form { display:flex; flex-direction:column; gap:20px; color:var(--t-page-text); font-size:13px; }
h3,h4,p { margin:0; } h3 { font-size:15px; font-weight:600; } h4 { font-size:13px; font-weight:600; }
header p,.config-group > p { margin-top:6px; color:var(--t-page-text-muted); font-size:12px; line-height:1.7; }
.config-group { padding:16px; border:var(--t-border-subtle); border-radius:10px; background:var(--t-surface-subtle); }
.config-row { display:flex; align-items:center; justify-content:space-between; gap:20px; min-height:48px; padding:10px 0; border-bottom:var(--t-border-subtle); }
.config-row:last-child { border-bottom:0; padding-bottom:0; } .config-row label { display:flex; flex-direction:column; gap:4px; } small { color:var(--t-page-text-muted); font-size:11px; line-height:1.6; }
.config-row :deep(.el-switch) { --el-switch-on-color:var(--theme-success); --el-switch-off-color:var(--t-surface-hover); }
.config-row :deep(.el-input-number) { flex:none; width:120px; } footer { display:flex; flex-wrap:wrap; align-items:center; gap:12px; } footer span { color:var(--t-success-text); font-size:12px; } .config-error { color:var(--t-danger-text); overflow-wrap:anywhere; }
</style>
