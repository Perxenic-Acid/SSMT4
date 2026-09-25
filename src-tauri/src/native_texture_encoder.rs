//! DirectXTex 原生纹理编码桥接（C ABI）。
//!
//! 对应实现见 `native/texture_encoder.cpp`，由 `build.rs` 在检测到 vcpkg
//! 安装的 DirectXTex 后编译并链接（此时才设置 `directxtex_native` cfg）。
//! 当前运行时仍走 texconv.exe，尚无 Rust 调用方；`#[used]` 静态引用确保
//! release 链接器的 `/OPT:REF` 不会丢弃桥接入口，打包出的 exe 真正携带
//! 原生 DirectXTex 编码器，后续接通 FFI 时立即可用、无需改动构建系统。
//! 未检测到 DirectXTex 时，整个桥接被 `directxtex_native` cfg 关闭，
//! 避免生成对不存在的 `ssmt_encode_rgba_dds` 的未定义引用。
#![allow(dead_code)]

#[cfg(directxtex_native)]
use std::ffi::c_char;

#[cfg(directxtex_native)]
extern "C" {
    #[link_name = "ssmt_encode_rgba_dds"]
    fn ssmt_encode_rgba_dds(
        pixels: *const u8,
        width: u32,
        height: u32,
        format_name: *const c_char,
        quick: bool,
        output_path: *const u16,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
}

/// 与 C++ 侧 `extern "C"` 声明保持一致的函数指针。
#[cfg(directxtex_native)]
type NativeEncodeFn = unsafe extern "C" fn(
    *const u8,
    u32,
    u32,
    *const c_char,
    bool,
    *const u16,
    *mut c_char,
    usize,
) -> i32;

/// 保留桥接入口：阻止链接器丢弃未调用的原生编码器。
#[cfg(directxtex_native)]
#[used]
static KEEP_NATIVE_TEXTURE_ENCODER: NativeEncodeFn = ssmt_encode_rgba_dds;
