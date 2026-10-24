---
name: model-extract-layout
description: 排查 SSMT 模型提取的 GameType 误匹配、实例数据误读、buffer 越界及 Blender 导入角色标记缺失；不用于客户端更新或注入故障。
---

先阅读 [帧分析解析说明](../../docs/frame-analysis-dump-resolution.md)，重点核对
`src-tauri/src/extract_new/srmi.rs` 的类型发现与分类导出调用链。

若问题是 Blender 未识别面部、脖颈或眼部，先按
[角色标记诊断](../../docs/blender-submesh-role-diagnostics.md) 对照页面记忆与实际导入
JSON 的 `SubMeshRole`。别名、贴图标记和 UI 勾选不能证明角色字段已写入。
角色恢复以模型 JSON 为准，仅用旧记忆补齐缺失值；`null` 是明确清除记录，不能用旧记忆覆盖。
涉及该流程时运行文档中的页面回归和 Rust 重新提取保留角色测试。

- 从日志找到实际解析到的 deduped TXT 和 BUF；不要仅凭 hash 猜数据来源。
- stride 和 TXT 的 vertex count 不足以证明逐顶点布局。CPU/GPU trianglelist
  都要核对 InputSlot、语义及索引、Format、AlignedByteOffset、InputSlotClass。
  per-instance 不能用作逐顶点分类，即使其它字段完全相同。
- 布局校验放在类型发现阶段；不要在导出候选选择阶段重新强制解释活动布局，
  以免破坏已有的可替代 GameType 导出。Pointlist 预蒙皮资源不直接套用最终 IA 布局。
- 无 element[] 的旧 TXT 保留兼容路径。越界保护仍须保留，不能截短数据掩盖类型错误。
- 使用真实布局字段构造离线回归，覆盖正确类型、同 stride 错误类型、实例属性和旧 TXT。
  运行 `cargo test --manifest-path src-tauri/Cargo.toml --lib extract_new::srmi::tests`。
