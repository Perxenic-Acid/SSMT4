import assert from 'node:assert/strict'
import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

// 包发布与插件测试可以认识具体插件；应用源码不得据此分派业务。
const roots = ['src-tauri/src', 'src']
const files = ['src-tauri/Cargo.toml']
async function walk(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) await walk(path)
    else if (/\.(rs|ts|vue|json)$/.test(path)) files.push(path)
  }
}
for (const root of roots) await walk(root)
const forbidden = /dlss5|hoyoshade|reshade|player.?tweaks|ssmt\.poser|ssmt\.secondary-motion|graphicsProfiles|GraphicsProfileStrategy|ssmt-plugin-graphics/i
for (const path of files) assert.equal(forbidden.test(await readFile(path, 'utf8')), false, `${path}: 插件业务回流到本体`)
console.log(`插件隔离检查通过：${files.length} 个本体文件`)
