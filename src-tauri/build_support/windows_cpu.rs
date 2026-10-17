/// 发布构建不得将构建机的可选指令集变成所有玩家的最低硬件要求。
pub fn validate_release_features(
    target: &str,
    profile: &str,
    features: &str,
) -> Result<(), String> {
    if target != "x86_64-pc-windows-msvc" || profile != "release" {
        return Ok(());
    }
    // SSE3 与 CMPXCHG16B 属于当前 Rust Windows x64 目标的默认基线。
    // crt-static 只决定 CRT 链接方式，不增加 CPU 要求。
    const BASELINE: &[&str] = &["cmpxchg16b", "fxsr", "sse", "sse2", "sse3", "crt-static"];
    let optional: Vec<_> = features
        .split(',')
        .filter(|feature| !feature.is_empty() && !BASELINE.contains(feature))
        .collect();
    if optional.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Windows x64 发布构建启用了非通用 CPU 指令集：{}。这会导致其他电脑在插件包校验等操作中闪退。请移除 Cargo 配置或 RUSTFLAGS / CARGO_ENCODED_RUSTFLAGS 中的 target-cpu=native、特定 CPU 型号及额外 target-feature，使用 target-cpu=x86-64。",
        optional.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_windows_x64_baseline() {
        for features in [
            "fxsr,sse,sse2",
            "cmpxchg16b,fxsr,sse,sse2,sse3",
            "crt-static,fxsr,sse,sse2",
        ] {
            assert!(
                validate_release_features("x86_64-pc-windows-msvc", "release", features).is_ok()
            );
        }
    }

    #[test]
    fn rejects_build_machine_features_in_release() {
        for feature in ["sha", "avx", "avx2", "sse4.1", "aes", "bmi2", "fma"] {
            let result = validate_release_features(
                "x86_64-pc-windows-msvc",
                "release",
                &format!("sse,sse2,{feature}"),
            );
            assert!(result.unwrap_err().contains(feature));
        }
    }

    #[test]
    fn does_not_restrict_local_debug_or_other_targets() {
        assert!(validate_release_features("x86_64-pc-windows-msvc", "debug", "sha,avx2").is_ok());
        assert!(validate_release_features("aarch64-pc-windows-msvc", "release", "neon").is_ok());
    }
}
