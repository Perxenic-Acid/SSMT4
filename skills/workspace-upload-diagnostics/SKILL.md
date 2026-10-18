---
name: workspace-upload-diagnostics
description: 排查 SSMT 工作空间预检、打包、R2 分片上传及公共目录发布失败，尤其是进度结束后失败或列表缺少条目。
---

# 工作空间上传诊断

- 调用链：`WorkPage.vue` → `workspace_access/commands.rs` → `archive.rs` / `upload.rs` →
  `services/workspace-access-api/src/index.js` → R2 校验 / GitHub 发布 / 索引生成。
- 先读缓存 `Logs/*-workspace-publish-*.log`。100% 只表示字节上传完成；完成接口仍需
  校验和发布。区分 `archive`、`initialize`、`upload`、`finalize`，保留错误码与路径。
- 没有服务端凭据时，可读取本地工作空间做预检，并用本地文件模拟 R2 的精确 Range
  复现校验；不要为排查自动向公共库发布测试条目。
- ZIP64 的 `0xffffffff` 是长度标记，首次解析与扩展 Range 均须读取 extra 中的真实长度。
  小清单测试不能覆盖跨越首次 64 KiB 的清单；测试应模拟准确的 Range 截断。
- 初始化返回 `published` 是复用对象成功，不能当作错误。正常新上传仍须走分片及完成接口。
- 当前 UI 使用的服务可能带 `dev` 名称。修改远程资源前核对实际地址与绑定；读取与
  发布是不同操作，按服务的部署约定执行。不要输出或提交本地凭据、签名 URL、真实包。
- 具体复现证据和验证命令见 [工作空间上传诊断](../../docs/workspace-upload-diagnostics.md)。
