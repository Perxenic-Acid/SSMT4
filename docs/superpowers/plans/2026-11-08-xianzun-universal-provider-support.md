# 芝士猫通用第三方供应商支持 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 用 pi-ai 协议层补齐全第三方 API 的添加、发现和真实对话能力，并导入 CC Switch 全部去重供应商预设。

**Architecture:** 芝士猫保留自己的配置、会话和工具循环；新增窄传输层把 pi-ai 的三种协议事件转换为现有流式接口。CC Switch 目录由可重复的生成脚本产出静态数据，旧配置在读取时迁移。

**Tech Stack:** Vue 3, TypeScript, Tauri 2, `@earendil-works/pi-ai@1.3.0`, Node/Bun tests

**Spec:** `docs/superpowers/specs/2026-11-08-xianzun-universal-provider-design.md`

## Global Constraints

- 不提交、推送或改写 Git 历史。
- 不覆盖用户已有配置和错误历史。
- 不把 API Key、Token 或用户数据写入生成目录。
- Bedrock、OAuth 和专用云鉴权不伪装成可用。
- 旧 DeepSeek、OpenCode 和自定义 Anthropic 配置必须保持可加载。
- 最终只替换本地 EXE，保留 `D:\game\ssmt4\resources` 和用户数据。

## Review Focus

- `GET /models` 成功但推理接口失败：测试必须分别验证两条链路。
- 工具调用流分片、工具结果续轮和审批请求：测试必须验证 native 与文本降级。
- 取消或切换会话时的旧流泄漏：测试必须验证 `AbortSignal` 到 pi-ai。
- 旧供应商缺少新字段：测试必须走真实旧形态样本加载。
- OAuth/Bedrock 预设：UI 必须标记限制，不得进入错误鉴权流程。

---

### Task 1: 基线与备份校验

**Files:**
- Verify only: `$env:SSMT4_SOURCE_BACKUP`
- Verify only: `src/store/XianZunProviders.ts`
- Verify only: `src/store/XianZunRequest.ts`

**Interfaces:**
- Produces: 已验证的备份哈希和当前 OpenCode 修复基线

- [ ] **Step 1: 校验备份**

Run:
```powershell
Get-FileHash -Algorithm SHA256 $env:SSMT4_SOURCE_BACKUP
```
Expected: `230B1CDC4936F07DAB27CB9EB620BCDE8D6FAC0A735DE98A55F793030AB3167F`

- [ ] **Step 2: 记录工作区差异**

Run:
```powershell
git status --short
git diff --stat
git diff -- src/store/XianZunProviders.ts src/store/XianZunRequest.ts src/views/XianZun/XianZun.vue
```
Expected: 只包含现有 OpenCode 修复和本任务计划文档。

### Task 2: pi-ai 传输适配层

**Files:**
- Create: `src/services/xianzun/types.ts`
- Create: `src/services/xianzun/piAiAdapter.ts`
- Test: `scripts/tests/test-xianzun-pi-ai-adapter.mjs`
- Modify: `package.json`
- Modify: `bun.lock`

**Interfaces:**
- Produces: `streamXianZunPiAi(options): Promise<XianZunStreamResult>`
- Produces: `buildPiAiModel(provider, modelId): Model<Api>`
- Produces: `toPiAiContext(messages, tools): Context`
- Consumes: 现有 `ApiMessage`、`AbortSignal`、Tauri fetch

- [ ] **Step 1: 写失败测试**

在 `scripts/tests/test-xianzun-pi-ai-adapter.mjs` 建本地 HTTP 模拟服务，断言：

- OpenAI Chat 路径为 `{baseURL}/chat/completions`
- Responses 路径为 `{baseURL}/responses`
- Anthropic 路径为 `{baseURL}/v1/messages`
- 自定义 `x-test-header` 实际到达服务端
- 文本、reasoning、工具参数增量、usage 和取消都被转换

- [ ] **Step 2: 跑测试确认失败**

Run:
```powershell
node --test scripts/tests/test-xianzun-pi-ai-adapter.mjs
```
Expected: FAIL，原因是适配层尚不存在。

- [ ] **Step 3: 安装 pi-ai**

Run:
```powershell
bun add @earendil-works/pi-ai@1.3.0
```
Expected: `package.json` 和 `bun.lock` 更新。

- [ ] **Step 4: 实现最小适配层**

使用三个懒加载 API；将 `fetch` 和 `AbortSignal` 原样传给 pi-ai。
将 pi-ai 事件映射到现有 `onChunk` 和 `XianZunStreamResult`。

- [ ] **Step 5: 跑测试确认通过**

Run:
```powershell
node --test scripts/tests/test-xianzun-pi-ai-adapter.mjs
```
Expected: PASS，0 failures。

### Task 3: 供应商类型与旧配置迁移

**Files:**
- Modify: `src/store/XianZunProviders.ts`
- Modify: `src/store/AppSettings.ts`
- Test: `scripts/tests/test-xianzun-provider-migration.mjs`

**Interfaces:**
- Produces: `XianZunApiFormat`
- Produces: `XianZunModelProfile`
- Produces: 扩展后的 `XianZunProvider`
- Produces: `resolveXianZunProviderModel(provider, modelId)`
- Consumes: Task 2 的 `buildPiAiModel`

- [ ] **Step 1: 写迁移测试**

覆盖旧 `protocol: openai`、旧 `protocol: anthropic`、现有 OpenCode 配置和新字段配置。
断言旧字段推导出正确 `api`，模型和密钥不丢失，二次序列化稳定。

- [ ] **Step 2: 运行测试确认失败**

Run:
```powershell
node --test scripts/tests/test-xianzun-provider-migration.mjs
```
Expected: FAIL，原因是类型和迁移函数尚未实现。

- [ ] **Step 3: 实现字段和迁移**

新增 `api`、`headers`、`compat`、`modelProfiles`，保留旧 `protocol` 兼容。

- [ ] **Step 4: 重新运行测试**

Run:
```powershell
node --test scripts/tests/test-xianzun-provider-migration.mjs
```
Expected: PASS。

### Task 4: CC Switch 预设导入

**Files:**
- Create: `scripts/import-cc-switch-presets.ts`
- Create: `src/data/xianzun/cc-switch-presets.json`
- Create: `docs/third-party/cc-switch.md`
- Test: `scripts/tests/test-xianzun-cc-switch-presets.mjs`
- Modify: `src/store/XianZunProviders.ts`

**Interfaces:**
- Produces: `CC_SWITCH_XIANZUN_PRESETS`
- Consumes: `..\cc-switch-source` 的固定提交

- [ ] **Step 1: 写目录测试**

断言 117 个 Pi 预设、421 个模型、159 个去重供应商、稳定 ID、无密钥字段，
并断言 Bedrock/OAuth 项带 `unsupportedReason`。

- [ ] **Step 2: 运行测试确认失败**

Run:
```powershell
node --test scripts/tests/test-xianzun-cc-switch-presets.mjs
```
Expected: FAIL，原因是生成文件不存在。

- [ ] **Step 3: 实现确定性生成脚本**

导入 CC Switch 各应用预设，以 Pi 目录为主，合并其余去重名称。
输出固定排序的 JSON，文件头文档记录上游提交和 MIT 归属。

- [ ] **Step 4: 生成并验证**

Run:
```powershell
bun run scripts/import-cc-switch-presets.ts
node --test scripts/tests/test-xianzun-cc-switch-presets.mjs
```
Expected: PASS。

### Task 5: 芝士猫 UI

**Files:**
- Modify: `src/views/XianZun/XianZun.vue`
- Test: `scripts/tests/test-xianzun-provider-ui-contracts.mjs`

**Interfaces:**
- Consumes: Task 3 的 provider profile
- Consumes: Task 4 的目录
- Produces: 可搜索的添加供应商、模型选择、协议和高级参数编辑

- [ ] **Step 1: 写 UI 合同测试**

断言供应商选择器 `filterable`，协议包含三项，高级设置存在 headers/compat，
模型选择继续使用完整目录，OAuth/Bedrock 显示限制说明。

- [ ] **Step 2: 运行测试确认失败**

Run:
```powershell
node --test scripts/tests/test-xianzun-provider-ui-contracts.mjs
```
Expected: FAIL。

- [ ] **Step 3: 最小实现 UI**

只修改芝士猫设置和添加供应商相关部分，不重构页面其他区域。

- [ ] **Step 4: 重新运行 UI 合同测试**

Run:
```powershell
node --test scripts/tests/test-xianzun-provider-ui-contracts.mjs
```
Expected: PASS。

### Task 6: 接入真实对话与工具循环

**Files:**
- Modify: `src/views/XianZun/XianZun.vue`
- Modify: `src/store/XianZunApi.ts`
- Modify: `src/store/XianZunAnthropic.ts`
- Test: `scripts/tests/test-xianzun-agent-transport.mjs`

**Interfaces:**
- Consumes: Task 2 的 `streamXianZunPiAi`
- Produces: 三种协议的统一芝士猫对话链

- [ ] **Step 1: 写请求形状测试**

覆盖 OpenAI Chat、Responses、Anthropic、OpenCode 会话头、工具增量、usage 和 abort。

- [ ] **Step 2: 运行测试确认失败**

Run:
```powershell
node --test scripts/tests/test-xianzun-agent-transport.mjs
```
Expected: FAIL。

- [ ] **Step 3: 切换请求入口**

新预设通过 pi-ai，旧未迁移自定义配置回退到现有请求实现；审批请求沿用相同会话 ID。

- [ ] **Step 4: 回归测试**

Run:
```powershell
node --test scripts/tests/test-xianzun-agent-transport.mjs scripts/tests/test-xianzun-opencode-session.mjs
```
Expected: PASS。

### Task 7: 全量验证与本地替换

**Files:**
- Build output: `src-tauri/target/release/ssmt4.exe`
- Replace: `D:\game\ssmt4\ssmt4.exe`

**Interfaces:**
- Consumes: 前六项全部结果

- [ ] **Step 1: 安装锁文件和类型检查**

Run:
```powershell
bun install --frozen-lockfile
bun run build
```
Expected: exit 0。

- [ ] **Step 2: 运行本次测试集合**

Run:
```powershell
node --test scripts/tests/test-xianzun-pi-ai-adapter.mjs scripts/tests/test-xianzun-provider-migration.mjs scripts/tests/test-xianzun-cc-switch-presets.mjs scripts/tests/test-xianzun-provider-ui-contracts.mjs scripts/tests/test-xianzun-agent-transport.mjs scripts/tests/test-xianzun-opencode-session.mjs
```
Expected: 全部 PASS。

- [ ] **Step 3: 构建生产 EXE**

Run:
```powershell
bun x tauri build --no-bundle
```
Expected: `src-tauri/target/release/ssmt4.exe` 生成。

- [ ] **Step 4: 备份并替换**

确认 `ssmt4.exe` 未运行，备份当前 EXE，将新 release EXE 原位替换。
保留 `resources`、配置和聊天数据。

- [ ] **Step 5: 启动与功能检查**

验证单实例启动、旧供应商加载、供应商搜索、三种协议预设显示。
用现有 DeepSeek 和 OpenCode Key 各完成一条短对话；
没有可用凭据的协议记录为“目录/模拟通过，真实账号未验证”，不得宣称通过。
