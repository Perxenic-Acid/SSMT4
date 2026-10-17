use ssmt4_lib::plugin::local_packages;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;

#[path = "../build_support/windows_cpu.rs"]
mod windows_cpu;

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
