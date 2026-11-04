# Runtime 临时部署

未锁定时，游戏注入目录不是 Runtime 来源。DX11 从随包资源与已验证的官方下载缓存选择来源，不比较注入目录里残留 DLL 的版本；DX12 沿用其随包来源。旧 3Dmigoto 的 `1.3.16` 不能压过 SSMT Runtime 的 `1.2.2.26`。

部署由 `runtime_deployment.rs` 在目标目录暂存，按 hardlink → symlink → copy 顺序回退，再 rename 替换目标。失败保留原目标文件，但停止本次注入，不静默使用旧 DLL。锁定且已有 DLL 时不替换、不联网、不登记清理；锁定但缺失时从随包资源初始化持久文件。

后端 `launch_console.rs` 为 Run.exe 会话领取本次部署记录，收到 `launch_complete` 后撤除目标链接或复制文件；`launch_error` 和控制台结束也执行清理。前端启动尚未提交就中止时撤回未领取的部署。清理比较卷号及文件 ID，避免误删后来替换的用户文件；源文件始终保留。清理失败记录后端警告，后续部署可重试；应用被强制终止时仍可能留下文件，下次未锁定启动会重新部署。

UPX 只处理独立缓存副本，完成后再以该副本为链接来源，不得对资源硬链接就地打包。缓存按原始资源哈希区分，不重复打包同一资源。

验证入口：

- `node --test scripts/tests/test-runtime-sync.mjs`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib commands::runtime_deployment::tests --no-default-features`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib commands::ssmt_runtime::tests --no-default-features`
- `bunx vue-tsc --noEmit`

Windows 局部测试覆盖已加载 DLL 的硬链接撤除及源文件保留，不等于真实游戏注入验收。实际验收应确认同次日志注入的是本次来源，注入完成后工作目录不再保留未锁定的 DLL。
