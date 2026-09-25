use std::{env, fs, path::PathBuf};

fn main() {
    build_directxtex_bridge();
    configure_windows_delay_load();
    copy_windivert_runtime_files();
    tauri_build::build()
}

fn build_directxtex_bridge() {
    if env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    println!("cargo:rustc-check-cfg=cfg(directxtex_native)");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let sibling_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("vcpkg"));
    let candidates = env::var_os("VCPKG_ROOT")
        .map(PathBuf::from)
        .into_iter()
        .chain(sibling_root)
        .collect::<Vec<_>>();
    // Cargo 会缓存构建脚本结果；显式声明依赖，DirectXTex 安装、更新或仓库内
    // vendored 文件变动后，下一次构建会自动重新探测，无需手动 cargo clean。
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
    for root in &candidates {
        let installed = root.join("installed").join("x64-windows-static-md");
        let header = installed.join("include").join("DirectXTex.h");
        let lib = installed.join("lib").join("DirectXTex.lib");
        if header.is_file() && lib.is_file() {
            println!("cargo:rerun-if-changed={}", header.display());
            println!("cargo:rerun-if-changed={}", lib.display());
        }
    }
    // 仓库内 vendored 兜底（src-tauri/native/directxtex），保证无 vcpkg 的
    // 环境（离线、CI、新克隆）也能编译链接。rerun-if-changed 对当前不存在的
    // 文件同样生效：文件出现时构建脚本会自动重跑。
    let vendored = manifest.join("native").join("directxtex");
    let vendored_header = vendored.join("include").join("DirectXTex.h");
    let vendored_lib = vendored.join("lib").join("DirectXTex.lib");
    println!("cargo:rerun-if-changed={}", vendored_header.display());
    println!("cargo:rerun-if-changed={}", vendored_lib.display());

    // 解析实际使用的 DirectXTex：(include 目录, lib 目录, 来源描述)。
    let used = candidates
        .iter()
        .find_map(|root| {
            let installed = root.join("installed").join("x64-windows-static-md");
            (installed.join("include").join("DirectXTex.h").is_file()
                && installed.join("lib").join("DirectXTex.lib").is_file())
            .then(|| {
                (
                    installed.join("include"),
                    installed.join("lib"),
                    format!("vcpkg {}", root.display()),
                )
            })
        })
        .or_else(|| {
            (vendored_header.is_file() && vendored_lib.is_file()).then(|| {
                (
                    vendored.join("include"),
                    vendored.join("lib"),
                    "vendored native/directxtex".to_string(),
                )
            })
        });

    let Some((include, lib_dir, source)) = used else {
        // The native DirectXTex bridge is optional: src/native_texture_encoder.rs
        // references the bridge entry only under the `directxtex_native` cfg,
        // and texture conversion runs through the bundled texconv.exe. Only fail
        // the build when the caller explicitly asks for it via
        // SSMT_REQUIRE_NATIVE_DIRECTXTEX.
        if env::var_os("SSMT_REQUIRE_NATIVE_DIRECTXTEX").is_some() {
            let preferred = candidates
                .last()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("vcpkg"));
            panic!(
                "Native DirectXTex is required, but neither vcpkg nor the vendored copy \
                 (src-tauri/native/directxtex) provides it. Run: {} install directxtex[dx11]:x64-windows-static-md",
                preferred.join("vcpkg.exe").display()
            );
        }
        println!(
            "cargo:warning=DirectXTex not found in vcpkg or in src-tauri/native/directxtex; \
             skipping the native texture encoder bridge. Install it via \
             `vcpkg install directxtex[dx11]:x64-windows-static-md` into the repo-sibling \
             D:\\Dev\\vcpkg (or set VCPKG_ROOT), or restore the vendored copy."
        );
        return;
    };
    println!("cargo:warning=Using DirectXTex from {source}");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("native/texture_encoder.cpp")
        .flag_if_supported("/std:c++17")
        .include(&include);
    build.compile("ssmt_texture_encoder");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=DirectXTex");
    println!("cargo:rustc-cfg=directxtex_native");
    // 桥接符号的保留由 Rust 侧 src/native_texture_encoder.rs 的 #[used] 静态
    // 引用完成（linker 的 /OPT:REF 不会丢弃被引用的入口符号）。
    for lib in ["d3d11", "dxgi", "windowscodecs", "ole32"] {
        println!("cargo:rustc-link-lib={lib}");
    }
    println!("cargo:rerun-if-changed=native/texture_encoder.cpp");
}

fn configure_windows_delay_load() {
    if env::var("CARGO_CFG_WINDOWS").is_ok() {
        println!("cargo:rustc-link-lib=delayimp");
        println!("cargo:rustc-link-arg=/DELAYLOAD:WinDivert.dll");
    }
}

fn copy_windivert_runtime_files() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let windivert_dir = manifest_dir.join("resources").join("WinDivert");
    let runtime_files = [
        windivert_dir.join("WinDivert.dll"),
        windivert_dir.join("WinDivert64.sys"),
    ];

    for runtime_file in &runtime_files {
        println!("cargo:rerun-if-changed={}", runtime_file.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let Some(profile_dir) = out_dir
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
    else {
        return;
    };

    for runtime_file in runtime_files {
        if !runtime_file.exists() {
            continue;
        }

        let file_name = runtime_file
            .file_name()
            .expect("runtime file missing file name");
        let target_file = profile_dir.join(file_name);
        if let Err(error) = fs::copy(&runtime_file, &target_file) {
            panic!(
                "failed to copy {} to {}: {}",
                runtime_file.display(),
                target_file.display(),
                error
            );
        }
    }
}
