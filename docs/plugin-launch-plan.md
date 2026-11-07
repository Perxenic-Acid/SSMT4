# 插件启动计划与文件事务

本文描述 P3 阶段落地的通用 LaunchPlan 契约。它解决插件参与启动时的四个问题：

- 插件只声明意图，不直接执行任意 Shell 或写任意绝对路径；
- 多个插件争抢同一个 DLL、配置或逻辑资源时，由宿主确定性仲裁；
- 文件修改在第一次落盘前写入恢复日志；
- 游戏退出或启动失败后，可以只恢复本插件拥有的变更，并保留用户后来写入的内容。

当前机制已经完成独立测试和陌生插件冲突夹具；官方插件迁移仍属于 P4，不会在本阶段
自动接管 DLSS5、HoYoShade 或 Player Tweaks 的真实启动路线。

## 1. Manifest

在 Schema 2 插件清单中增加 `contributions.launchHooks`：

```json
{
  "contributions": {
    "launchHooks": [
      {
        "id": "install-proxy",
        "priority": 100,
        "actions": [
          {
            "type": "file-copy",
            "source": "payload/dxgi.dll",
            "target": "dxgi.dll",
            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "resource": "proxy:dxgi"
          }
        ]
      }
    ]
  },
  "permissions": [
    "filesystem.read",
    "filesystem.write"
  ]
}
```

约束：

- `source` 必须是插件包内相对路径，且 `file-copy` 必须声明源文件 SHA-256；
- `target` 必须是游戏目录内相对路径，禁止绝对路径、`..`、盘符和宿主保留目录；
- `resource` 是可选逻辑资源键，例如 `proxy:dxgi`；目标路径还会自动形成
  `file:<relative-target>` 资源；
- 每个插件最多 128 个 hook，每个 hook 最多 64 个动作；hook ID 在插件内唯一；
- 同一 hook 不得对同一目标声明两个动作；
- 文件动作需要对应文件权限，启动参数动作需要 `game.launch`。

## 2. 动作

### `file-copy`

把包内源文件部署到游戏目录。

| 字段 | 必填 | 说明 |
|---|---|---|
| `source` | 是 | 插件包内源文件 |
| `target` | 是 | 游戏目录相对目标 |
| `sha256` | 是 | 源文件固定摘要 |
| `resource` | 否 | 跨文件逻辑资源键 |
| `expectedTargetSha256` | 否 | 部署前目标摘要，用于防止覆盖计划生成后发生的修改 |

### `file-remove`

删除计划拥有的游戏文件。

| 字段 | 必填 | 说明 |
|---|---|---|
| `target` | 是 | 游戏目录相对目标 |
| `resource` | 否 | 跨文件逻辑资源键 |
| `expectedTargetSha256` | 否 | 删除前目标摘要 |

### `argument-append`

按确定顺序向启动参数追加值：

```json
{
  "type": "argument-append",
  "values": ["--plugin-fixture"]
}
```

每个动作最多 32 个值，每个值最多 4096 字符。参数动作按优先级、插件 ID、hook ID
和动作 ID 排序，不依赖目录扫描或异步完成顺序。

## 3. 资源仲裁

所有启用的插件先按 Manifest 解析为候选动作，再按资源键分组。

规则：

1. 同一资源只有唯一最高优先级的插件时，该插件获得资源；
2. 多个插件处于相同最高优先级时，返回明确冲突，不按加载顺序随机选择；
3. 低优先级候选记入 `skippedActions`，用于诊断；
4. 任一未解决冲突会让整份计划不可执行，文件动作不会部分部署；
5. 已提交的持久部署继续拥有目标，后续计划在恢复前不能覆盖。

预览命令：

```text
plugin_launch_plan_preview(gameName)
```

返回：

```json
{
  "fileActions": [],
  "argumentAppends": [],
  "skippedActions": [],
  "conflicts": [
    {
      "resourceKey": "plugin:proxy:fixture",
      "reason": "multiple plugins claim this resource at the same priority",
      "candidates": [
        {
          "pluginId": "example.codex.launch-alpha",
          "pluginVersion": "1.0.0",
          "hookId": "install-proxy",
          "priority": 100,
          "target": "ssmt-fixture-proxy.dll"
        },
        {
          "pluginId": "example.codex.launch-beta",
          "pluginVersion": "1.0.0",
          "hookId": "install-proxy",
          "priority": 100,
          "target": "ssmt-fixture-proxy.dll"
        }
      ]
    }
  ]
}
```

插件详情页会用同一结果展示“启动资源冲突”，不包含插件专用 ID 分派。

## 4. 文件事务

文件事务目录位于游戏目录：

```text
.ssmt-launch-files.lock
.ssmt-launch-transactions/<transactionId>/
```

这些名称由宿主保留，插件不能声明为目标。

执行顺序：

```text
preview
  → validate target baseline
  → copy originals to transaction backup
  → stage new files
  → fsync journal
  → mutate targets
  → write committed marker
  → launch / hold deployment
  → restore on game exit
```

关键保证：

- Journal 在第一次目标修改前持久化；
- 所有源文件和备份都做流式 SHA-256 校验；
- 提交失败时按逆序恢复原文件并删除新文件；
- 中断后再次打开目录时自动处理没有 `committed` 标记的事务；
- 已提交部署保留备份和恢复日志，供游戏退出后恢复；
- 如果目标在部署后又被用户或其他程序修改，恢复会拒绝覆盖并保留恢复现场；
- 事务锁保证同一游戏目录不能被两个执行器同时修改。

## 5. 生命周期 API

Rust 核心入口：

- `launch_plan::resolve_hook_plan(gameRoot, plugins)`：只读解析、校验和仲裁；
- `ResolvedHookPlan::apply_file_actions(gameRoot)`：提交持久文件部署；
- `ResolvedHookPlan::restore_file_actions(gameRoot, transactionId)`：恢复并清理部署；
- `file_transaction::apply(...)`：持久事务执行器；
- `file_transaction::restore_applied(...)`：按事务 ID 恢复；
- `file_transaction::applied_targets(...)`：读取当前仍被部署占用的目标。

P4 将把这些通用入口接到实际游戏启动和目标退出观察中；P3 不改变现有正式启动链。

## 6. 模拟验收

构建两个陌生插件：

```powershell
.\scripts\dev\build-plugin-launch-plan-fixture.ps1
```

启动开发版 SSMT4 后：

```powershell
node scripts/dev/verify-plugin-launch-plan-fixture.mjs
```

验收内容：

- 两个陌生 ID 无需修改 Core 即可安装和启用；
- 两者以相同优先级声明同一目标和 `proxy:fixture` 资源；
- 预览返回冲突，不生成任何文件动作；
- 禁用其中一个后，计划只保留唯一所有者；
- 整个流程没有向游戏目录写入夹具 DLL。

Rust 单元测试另行覆盖：

- 失败后的多文件回滚；
- 已存在文件替换和新文件删除；
- 持久部署阻止第二次覆盖；
- 游戏退出恢复原始文件；
- 中断 Journal 恢复；
- 用户修改后拒绝覆盖；
- 源文件摘要不匹配、路径越界和权限不足。

## 7. 安全边界

- LaunchPlan 不是脚本引擎，不提供任意命令、任意路径或任意注入代理。
- 文件事务不是权限沙箱；它保证可追踪、可恢复，不改变 Windows 用户权限边界。
- 原生 DLL 的真实注入仍由 Native Runtime Contribution 和现有 ABI 负责。
- `priority` 只影响资源仲裁，不会让插件获得额外系统权限。
- 插件文件在安装时验证声明源摘要，执行前再次验证，避免安装后静默替换。
