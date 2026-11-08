# 芝士猫通用第三方供应商支持设计

## 目标

让芝士猫不再只依赖“恰好兼容 OpenAI Chat Completions 或 Anthropic Messages”的简易请求，
而是像 DeepSeek Harness 一样按供应商配置选择真实协议、认证头、兼容开关和模型能力。
同时把 CC Switch 的供应商预设完整导入 SSMT4，并保证旧供应商配置可继续加载。

## 当前问题

- 芝士猫只实现了简易 OpenAI Chat Completions 与 Anthropic Messages 两条请求链。
- 测试连接只验证模型列表，不能代表真实推理请求可用。
- 供应商没有独立的协议、请求头、兼容参数、模型输入能力和推理能力模型。
- 第三方网关常见的 `developer` role、`max_tokens`、thinking 格式、工具消息差异没有配置入口。
- CC Switch 的最新供应商目录尚未接入。

## 调研结论

DSH 的 `llm-pi-ai` 使用 `@earendil-works/pi-ai` 统一承载：

- `openai-completions`
- `openai-responses`
- `anthropic-messages`

每家供应商由配置提供 API、Base URL、凭据、自定义 headers、compat、模型目录和能力元数据。
模型发现与真实推理是两条独立链路，不能用 `GET /models` 成功代替对话验证。

CC Switch 当前 `piProviderPresets` 包含：

- 117 个无重名供应商
- 421 个模型条目
- 68 个 OpenAI Chat Completions
- 41 个 Anthropic Messages
- 7 个 OpenAI Responses
- 1 个 Bedrock

按 CC Switch 各应用预设名称去重后共有 159 个供应商。额外的 42 个主要是官方、
OAuth、云平台和别名，需要保留来源信息并明确其鉴权要求。

## 方案选择

### 方案 A：继续手写协议适配

分别手写 OpenAI Responses、Gemini、各网关的 SSE 和兼容逻辑。

优点是依赖少；缺点是 421 个模型的请求差异无法长期维护，容易再次出现“能拉模型不能聊天”。

### 方案 B：pi-ai + 芝士猫窄适配层

锁定 `@earendil-works/pi-ai@1.3.0`，芝士猫只维护：

- 从现有会话历史到 pi-ai Context 的转换
- 从 pi-ai 流事件到现有 `onChunk` 和工具循环的转换
- Tauri HTTP、取消、会话 ID 和自定义请求头注入

供应商配置仍由 SSMT4 自己持久化，不把 DSH 的 Cordis、凭据或设置系统搬入 SSMT4。

这是推荐方案。

### 方案 C：只导入 CC Switch 预设静态数据

实现成本最低，但只能解决列表数量，不能解决协议和请求兼容，不能满足“正常对话”目标，因此不采用。

## 设计

### 供应商配置

在现有 `XianZunProvider` 上向后兼容地增加：

- `api`: `openai-completions | openai-responses | anthropic-messages`
- `headers`: 自定义 HTTP 请求头
- `compat`: 协议兼容参数
- `modelProfiles`: 每个模型的显示名、上下文、最大输出、输入类型、推理能力和 compat
- `authMode` 或现有限定字段用于区分 Bearer、Anthropic key 和特殊鉴权

读取优先级为：

1. 供应商预设默认值
2. 用户保存的供应商配置
3. 模型级覆盖
4. pi-ai 自身兼容检测

旧配置没有 `api` 时根据原 `protocol` 推导，不能丢失 ID、密钥、URL、模型和会话。

### 协议传输

新增独立的 `src/services/xianzun/` 传输层，使用 `@earendil-works/pi-ai@1.3.0` 的懒加载 API：

- OpenAI Chat Completions
- OpenAI Responses
- Anthropic Messages

统一输出现有芝士猫需要的文本、推理、工具调用增量、usage、完成和错误。
Tauri 的 HTTP 请求能力通过 pi-ai 的 `fetch` 选项注入；`AbortSignal` 必须一直传到最底层。

OpenCode 的 `x-opencode-session` 使用当前对话 ID，普通轮次、工具续轮和审批请求保持一致。

### 供应商目录

新增生成脚本，从固定 CC Switch 提交导入：

- 完整保留 117 个 Pi 供应商和 421 个模型
- 合并其余 42 个去重供应商
- 保留名称、来源、分类、协议、Base URL、模型、headers 和 compat
- 同名不同端点保留为不同的“版本”，不粗暴去重
- 不写入任何 API Key、Token 或用户配置

生成结果写入固定的 `src/data/xianzun/` 数据文件，并在文件头记录上游提交和 MIT 归属。

Bedrock、OAuth 和当前无法由 API Key 描述的云凭据供应商仍显示在目录中，
但标记为“当前需专用鉴权/未支持”，不能把不可用能力伪装成可对话。

### UI

- 添加供应商下拉框启用搜索，可按名称、别名和协议查找
- 选中预设后自动填入端点、默认模型、协议和高级参数
- 模型下拉显示供应商提供的完整模型目录，并支持单独搜索
- 高级设置提供协议选择、请求头 JSON、compat JSON 和模型能力查看
- “获取模型”只表示模型发现；“测试连接”应能区分列表可达与真实推理

### Agent 与工具

保留现有芝士猫工具循环和审批机制。传输层需要把 pi-ai 的工具调用增量映射到现有
`NativeToolCall`，并保证：

- tool call ID 在续轮中不丢失
- 工具结果以协议要求的形式回传
- reasoning、usage、abort 和错误信息不丢失
- 不原生支持 tools 的网关仍走现有文本工具调用降级协议

## 非目标

- 本期不在 SSMT4 中实现 Bedrock SigV4 签名
- 本期不实现 OAuth 登录与 token 刷新
- 本期不重写芝士猫的会话、审批和工具执行架构
- 本期不生成发行安装包

## 验收

- 旧 DeepSeek 配置无需手工迁移即可继续对话
- 现有 OpenCode Go 配置继续发送稳定会话头
- OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 的模拟流测试通过
- 117 个 CC Switch Pi 预设全部存在，421 个模型条目完整
- 159 个去重供应商均有稳定 ID，重复名以版本形式区分
- 无凭据泄漏
- `bun run build` 和 `tauri build --no-bundle` 通过
- 替换后的本地实例可启动，并实际完成至少三条不同协议的短对话或明确记录外部凭据阻塞
