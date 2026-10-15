// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod native_texture_encoder;

fn main() {
    let mut args = std::env::args_os();
    if args.nth(1).as_deref() == Some(std::ffi::OsStr::new("--ssmt-graphics-cleanup")) {
        if let (Some(game), Some(parent), Some(session)) = (args.next(), args.next(), args.next()) {
            if let Ok(pid) = parent.to_string_lossy().parse::<u32>() {
                let path = std::path::PathBuf::from(game);
                if let Err(error) = ssmt4_lib::plugin::managed_stack::cleanup_helper(&path, pid, &session.to_string_lossy()) {
                    eprintln!("[GraphicsStack] Cleanup helper failed: {error}");
                    std::process::exit(1);
                }
                return;
            }
        }
        std::process::exit(2);
    }
    ssmt4_lib::run()
}
