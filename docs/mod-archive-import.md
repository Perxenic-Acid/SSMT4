# Mod 多选与加密压缩包导入

Mod 管理页的文件、目录导入按钮支持多选；拖入多个压缩包或目录走同一批量入口。单个对象保留安装确认页，多对象依次预览、解压并统计成功、失败和跳过数。

预览或解压遇到密码要求时显示密码浮窗，标明当前文件名。输入的密码在当前批次内顺延，普通文件不会清除密码；只有解密失败时才重新询问。不同密码重试成功后使用新密码继续。取消或关闭浮窗只跳过当前对象；之前可用的密码仍保留给后续对象。密码不去除首尾空格，不持久化到配置或日志，批次结束后释放。

前端位于 `ModsManagement.install.ts`。`preview_mod_archive` 新增可选 `password`，`install_mod_archive` 将密码传入 ZIP、7z 和 RAR 解压入口。密码错误通过 `ARCHIVE_PASSWORD_REQUIRED`、`ARCHIVE_PASSWORD_INVALID` 标记；普通损坏或 IO 错误保留普通失败，不触发密码浮窗。ZIP 的加密条目不能静默忽略；7z 的两次读取使用同一密码。

安装继续沿用暂存目录事务：任何解压失败先清理暂存目录，再允许重试，成功解压后才提交目标目录。批次固定开始时的游戏和分组，避免密码输入期间切换界面导致目标漂移。

验证：

```powershell
node --test scripts/tests/test-mod-import-password.mjs scripts/tests/test-mod-snapshot.mjs
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib utils::ssmt_compress_utils
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib commands::mod_manager
bun run build
```

前端回归覆盖同密码复用、不同密码、错误输入、取消继续、单文件导入与文件/目录多选。后端使用真实 AES ZIP、ZipCrypto ZIP、加密 7z 验证预览和解压；Windows ZIP 回归使用仓库附带的 `resources/7za.exe` 生成临时样本。RAR 覆盖库的密码错误码分类，未生成真实加密 RAR 样本。

- 导入结束仅刷新目标分类及新增子树的索引，当前列表按身份合并，保留已有卡片和分析订阅；不走全库结构刷新、不切回根分类、不启用页面级 loading。批次必须汇总失败文件名和原始错误，不能只记录控制台或仅显示成功数。

- 同名目标由后端返回 `MOD_IMPORT_CONFLICT`，单文件与批次均提供覆盖、跳过、重命名、直接写入。覆盖保留禁用备份；直接写入在解压完成后合并，保留其他文件并覆盖同名文件，不沿目标链接写入。解压事务仍完整，直接写入阶段的 IO 失败可能已有部分文件更新，错误须明确说明。新 Rust 参数需要重启 `bun tauri dev`。

## Windows 提交目录时拒绝访问

`Failed to finalize install` 表示解压完成后的目录移动失败，与密码无关。安装先对 Windows 错误 5、32、33 做有限重试；目标不存在时，以独占创建目标目录的方式回退到复制。复制失败仅清理本次创建的目标，已有目录必须经过同名处理流程，不能借回退覆盖。替换失败时恢复禁用备份，恢复失败须明确显示备份位置。

失败及复制回退会按需调用文件占用诊断，输出路径属性和占用进程（名称、PID、程序路径）；未找到进程不能证明没有目录句柄占用或 ACL 限制。不自动结束进程或放宽 ACL。复制成功但暂存目录仍被占用时，暂存清理按原逻辑尽力执行，不把成功安装误报为失败。

手工诊断真实 ZIP（只读源文件，解压到自动清理的临时目录）：

```powershell
$env:SSMT_ARCHIVE_PROBE = 'C:\path\mod.zip'
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib diagnose_local_zip_import -- --ignored --nocapture
```
