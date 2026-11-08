# 芝士猫 OpenCode Go 会话头修复

## 根因

- OpenCode Go 的聊天接口要求稳定的 `x-opencode-session` 请求头。
- 芝士猫聊天请求原先只发送 `Authorization`，因此返回 `400 MissingSessionID`。
- “测试连接”只请求 `GET /models`，无法验证 `/chat/completions` 的真实推理链路。

## 修复

1. OpenCode 域名请求注入 `x-opencode-session` 与专用 `User-Agent`。
2. 会话 ID 使用持久化的当前对话 ID，并在代理轮次开始时固定。
3. 普通 DeepSeek 与其他 OpenAI 兼容供应商不发送 OpenCode 专属头。
4. 在芝士猫的内置供应商列表中加入 OpenCode Go，预置官方端点与当前模型。
5. 添加请求头和供应商预设回归测试。

## 验证

1. 运行新增的请求头测试。
2. 使用仓库自带的 `bun.lock` 安装依赖，并执行前端构建与 TypeScript 检查。
3. 已用真实最小请求确认：无会话头返回 400，有时返回 200。

## 范围说明

- 本次只修复当前复现的 OpenAI Chat Completions 路径。
- `GET /models` 仍只用于检查模型列表可访问性，不等同于推理测试。
- 若以后要完整支持 OpenCode Go 的 Anthropic Messages 分支，需要在该分支同步注入相同请求头。

## 本地替换

1. 使用 `tauri build --debug --no-bundle` 做首次运行验证，再用 `tauri build --no-bundle` 生成最终本地可执行文件；均不生成发行安装包。
2. 构建时同时把 portable Bun 和 Cargo 目录加入 PATH，确保 Tauri 子进程能找到工具链。
3. 备份当前 `D:\game\ssmt4\ssmt4.exe`。
4. 只替换 EXE，保留安装目录现有 `resources`、配置和数据结构。
5. 重新启动 SSMT4，并检查芝士猫的 OpenCode Go 预设。
