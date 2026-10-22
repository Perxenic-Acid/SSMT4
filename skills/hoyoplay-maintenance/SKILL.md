---
name: hoyoplay-maintenance
description: 开发或排查 SSMT 的米家客户端安装、预下载、更新及原神渠道转换，涵盖 Sophon 分块与文件恢复。不用于 DLL 注入或渲染诊断。
---

先阅读 [实现与验收说明](../../docs/hoyoplay-maintenance.md)，再按问题定位
`src-tauri/src/hoyoplay/` 或 `src/features/gameMaintenance/`。主页只负责入口与状态，
不要把分块下载或游戏差异处理搬到 Home.vue。

几个已确认的约束：

- 以 `getGameBranches` 和 Sophon `getBuild` 为当前版本依据。旧 `getGamePackages`
  曾返回明显落后的完整包，不能据它降级或覆盖新客户端。
- B服有独立 game ID，但官方 `biz` 仍为 `_cn`；不能虚构 `_bilibili`。
  崩铁 / 绝区零的国服和国际服共用 exe 名称，`channel=1` 不足以判断渠道。
- 清单、压缩块、解压块、完整文件分别校验官方摘要与大小。不能把 chunk ID
  猜成压缩 MD5。绝区零有大量非语音类别，不能只取 `matching_field=game`。
- 预下载只写 `.ssmt-maintenance`，从任务开始到结束都不能拦截普通启动。
  更新、换服成功前不写客户端版本、渠道和 SSMT 目标路径。
- 改动文件通过 journal / backup 提交；恢复前检查全部目标与备份，遇到后续人工
  修改应停下并保留记录。不要删除整个 Plugins / Mods / 数据目录。
- 下载、清单解压、文件哈希与进程扫描使用后台任务。异步状态必须防止跨游戏迟到
  响应覆盖，查询失败不能把未知版本变成强制更新。
- 配置中的程序路径不可信：检查真实文件，不能用 `exists` 把目录当成程序，也不能
  只因文件名未知而拒绝有效路径。主页与米家普通启动传 `quickOnly: true`，不能
  隐式触发全盘扫描；入口和文件检查须有等待上限，超时进入安装且不应用迟到结果。

改动后运行文档中的离线测试。`live_` 测试仅请求官方元数据或清单，默认忽略，
需要时单独运行；它们不证明完整安装或实机启动通过。未经本轮任务授权，不启动游戏、
不修改真实游戏目录，不发布构建或运行主仓库发行包脚本。
