# F8 空转储与文件映射恢复

## 两个独立问题

2026-10-17 的原神记录中，普通 F8 目录只有 Present 命令日志，没有游戏 Draw；指定 IB 的目录则包含实际绘制和 Buffer。用户确认采集来自同一进程、正常渲染且窗口在前台。`analyse_options=0140073d` 的多个空目录并非没有启用转储选项。

Runtime 曾在 `24a94e7d` 修复图形组件复制 COM 虚表后的帧分析内部派发。该修复应保留；不能为了 F8 回退到在 COM 虚表末尾追加私有方法的实现。

另一个可复现的缺陷在帧边界：辅助交换链会委托游戏交换链执行帧动作，连续 Present 可以在下一次游戏 Draw 前结束刚启动的采集。WARP 夹具复现了旧 DLL 创建目录但不导出 VB。现在只有游戏 Context 发生 Draw、Dispatch 或 `ExecuteCommandList` 后才推进帧分析边界；不改变 Mod 的 Present 命令与输入轮询。完全没有游戏绘制时采集会等待，用户仍可通过采集绑定中止。

## 已有数据的解析修复

`FrameAnalysis-2026-10-17-193103` 的 `848c5980` 使用 Pointlist `000002`，Trianglelist 为 `000073` 和 `000138`。日志缺少 `000002` 的 VB0/VB1 映射，但原始文件是有效符号链接，实际数据存在：

| Category | 文件 | 大小 | 顶点数 |
| --- | --- | ---: | ---: |
| Position | `54c32104.buf` | 12640 | 316 × 40 |
| Blend | `61224bdf.buf` | 10112 | 316 × 32 |
| Texcoord | `9f15c59d.buf` | 3792 | 316 × 12 |

提取器原先只认日志索引，缺少索引即返回空路径，导致排除 GPU 类型。现按以下顺序解析：

1. 使用日志映射指向的现存 `deduped` 文件。
2. 使用原始转储文件；由文件系统跟随符号链接，硬链接和普通文件同样可读。
3. 两者均不可用时，区分文件名未找到、映射缺失、去重文件丢失或原始链接失效。

文件名接口也从真实链接目标恢复名称，避免贴图同步取得有效路径却没有文件名。不会扫描 Hash 同名文件猜测来源。只有 `.lnk` 快捷方式且日志映射也缺失时，当前不解析 Shell 快捷方式；归档丢失链接、目标文件缺失等情况仍会报告失败。

真实样本只读验证通过：两个 Trianglelist 都匹配 `GPU_P12_N12_TA16_C4_T8_BW16_BI16_`，不再退回 CPU 类型。没有修改样本、工作空间或 GameType，也未改动 Pointlist 选择算法。

上述帧边界缺陷不能单独证明历史映射为何丢失。Runtime 原实现会在 Buffer 导出前记录映射，因此不能把映射缺失简单解释成去重缓存命中；需用更新后的实际进程日志继续核对。

## 回归与验收

应用侧运行：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib common::frame_analysis
cargo test --manifest-path src-tauri/Cargo.toml --lib extract_new::
$env:SSMT_GIMI_MAPPING_SAMPLE = '实际的 FrameAnalysis-2026-10-17-193103 目录'
cargo test --manifest-path src-tauri/Cargo.toml --lib existing_848c5980_dump_matches_gpu_type -- --ignored
```

合成夹具使用零填充数据和真实文件链接，覆盖映射缺失但文件存在，以及 Hash 文件存在但原始链接丢失的反例。最后一项显式读取外部样本，不写入样本或工作空间。

Runtime 编译后，按 [Context 回归说明](../runtime/tests/context-interop/README.md) 执行 WARP 测试。不要运行主应用发布包构建脚本；可通过 `scripts/stage-ssmt-runtime.ps1 -Build` 构建并暂存本机 Runtime，再重新构建开发应用。

实机仍需重新启动游戏，确认实际加载的是本次 DLL；旧进程不会因替换磁盘文件更新。开启 Hunting，关闭指定 IB 转储，在正常画面按 F8，应同时出现游戏 Draw、VB/IB 等文件及其日志映射。再检查指定 IB 模式，确认定向转储仍有效。分别保留两次新日志；若仍只有 Present，继续检查实际 Context/交换链调用路径，不能把 WARP 回归当作完整实机结论。
