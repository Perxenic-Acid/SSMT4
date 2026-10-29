use ssmt4_lib::plugin::local_packages;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;

#[path = "../build_support/windows_cpu.rs"]
mod windows_cpu;

#[test]
fn app_resources_exclude_development_packages_and_player_tweaks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let patterns = config["bundle"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pattern| {
            root.join(pattern.as_str().unwrap())
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    // 使用打包器相同的枚举器，旧资源目录与遗留 DLL 即使仍在磁盘上也不能被带入安装器。
    let files = tauri::utils::resources::ResourcePaths::new(&patterns, true)
        .map(|path| path.unwrap())
        .collect::<Vec<_>>();
    for name in ["Run.exe", "SSMT-PluginHost.dll", "d3d11.dll"] {
        assert!(files
            .iter()
            .any(|file| file.file_name().is_some_and(|actual| actual == name)));
    }
    for file in files {
        assert!(
            !file
                .components()
                .any(|part| part.as_os_str() == "PluginPackages" || part.as_os_str() == "Plugins"),
            "{}",
            file.display()
        );
        assert_ne!(file.file_name().unwrap(), "SSMT-Player-Tweaks.dll");
        assert!(
            !file
                .extension()
                .is_some_and(|ext| ext == "ssmtpkg" || ext == "log"),
            "{}",
            file.display()
        );
    }
}

#[test]
fn discovers_compressed_packages_on_small_stack() {
    const PROBE: &str = "SSMT_MARKETPLACE_STACK_PROBE";
    if let Some(directory) = std::env::var_os(PROBE) {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                let packages = local_packages::discover(std::path::Path::new(&directory)).unwrap();
                assert_eq!(packages.len(), 1);
                assert_eq!(packages[0].inspection.file_count, 2);
            })
            .unwrap()
            .join()
            .unwrap();
        return;
    }

    let directory = std::env::temp_dir().join(format!(
        "ssmt-marketplace-stack-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir(&directory).unwrap();
    let mut archive = zip::ZipWriter::new(File::create(directory.join("fixture.ssmtpkg")).unwrap());
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    archive.start_file("ssmt-plugin.json", options).unwrap();
    archive
        .write_all(
            br#"{
        "schemaVersion":1,"id":"example.fixture","name":"Fixture","version":"1.0.0","author":"Test",
        "compatibility":{"ssmt":">=4.0.0","platforms":["windows-x64"]},"contributions":{}
    }"#,
        )
        .unwrap();
    archive.start_file("assets/payload.dat", options).unwrap();
    let block = vec![0x42; 64 * 1024];
    for _ in 0..128 {
        archive.write_all(&block).unwrap();
    }
    archive.finish().unwrap();

    // 栈溢出不能被 catch_unwind 恢复；子进程用于保留测试进程和退出码证据。
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovers_compressed_packages_on_small_stack",
            "--nocapture",
        ])
        .env(PROBE, &directory)
        .output()
        .unwrap();
    fs::remove_dir_all(directory).unwrap();
    assert!(
        result.status.success(),
        "status: {:?}\n{}\n{}",
        result.status.code(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
