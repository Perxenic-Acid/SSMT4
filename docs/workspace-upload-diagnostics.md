# 工作空间上传结束后失败

## 已复现的 ZIP64 大清单问题

SSMT 使用 ZIP64 本地文件头打包 `.ssmtws`。`manifest.json` 的 32 位长度字段可能是
`0xffffffff`，实际长度保存在 ZIP64 extra 中。Worker 首次读取包头的 64 KiB，
清单较大时再扩展 Range。

旧 `manifestRequiredLength()` 扩展 Range 时直接使用 32 位字段，把 ZIP64 标记误当成
约 4 GiB 的长度，触发清单大小上限，返回 `ARCHIVE_INVALID`。完成接口随后删除未通过
校验的 R2 对象、释放预留容量，条目不会发布到 GitHub，因此下载列表中也没有。

2026-10-18 使用 `Водяница` 的本地文件重新打包后离线复现：

- 归档大小：809,775,011 字节。
- payload：329 个文件，清单：67,690 字节。
- 旧逻辑：`ARCHIVE_MANIFEST_INVALID`；修复后同一归档通过校验。
- 验证使用本地文件模拟 R2 Range，没有重新上传或发布真实条目。

修复复用已有 ZIP64 extra 解析，仍保留 16 MiB 清单读取上限以及路径、中央目录、
文件数量、内容大小检查。回归覆盖普通 ZIP / ZIP64 的大清单及超限 ZIP64 长度。

此问题需要部署 **Cloudflare Worker** 才能修复线上上传。仅更新 SSMT 客户端无效。
本次未读取到线上 D1 失败记录，不能将离线复现当作已经完成线上验收。

## 客户端诊断

每次发布在当前缓存 `Logs/*-workspace-publish-*.log` 中记录：

- 本地检查与打包、上传初始化、分片进度、服务端校验与发布。
- submission ID、完成字节及分片数量。
- 最终 entry ID，或错误码与 HTTP 状态。

发布失败弹窗显示阶段、具体原因与日志路径。本地预检错误保留相关文件路径；
服务端校验错误可附带稳定的细分错误码。不记录签名 URL、请求正文或凭据。
日志无法创建不应阻止发布。

初始化返回 `published` 表示服务端已复用现有对象并完成发布，应直接成功，
不能再次请求分片或误报 `UPLOAD_INITIALIZATION_FAILED`。

## 验证与上线

```powershell
cd services/workspace-access-api
node --test test/*.test.js
```

```powershell
cd src-tauri
cargo test --offline --lib workspace_access:: -- --test-threads=1
```

```powershell
node --test scripts/tests/test-workspace-publish-error.mjs
bun run build
```

部署遵循服务目录的 `DEPLOYMENT.md`。核对客户端实际 API 地址和对应 Worker、D1、R2，
不能因资源名称包含 `dev` 就假定其不承载用户数据。先确认 Wrangler 登录可用；
没有部署授权或凭据时保留已验证修改，明确告知线上尚未更新。

上线后重新上传该工作空间，确认完成接口返回 `published`、公共 metadata/status
存在，并等待索引生成后核对下载列表。历史校验失败已删除的对象需要重新上传。
