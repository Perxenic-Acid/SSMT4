use super::{
    client,
    manifest::{self, Asset, Chunk, Manifest},
};
use prost::Message;

fn asset() -> Asset {
    Asset {
        name: "Game_Data/resource.bin".into(),
        md5: format!("{:x}", md5::compute(b"game")),
        size: 4,
        folder: false,
        chunks: vec![Chunk {
            id: "chunk_1".into(),
            offset: 0,
            size: 4,
            compressed_size: 4,
            md5: format!("{:x}", md5::compute(b"game")),
            compressed_md5: String::new(),
        }],
    }
}

#[test]
fn only_supported_games_and_declared_channels_are_exposed() {
    for preset in ["GIMI", "SRMI", "ZZMI", "ZZMIDX12"] {
        let servers = client::servers(preset);
        assert_eq!(servers.len(), 3);
        for server in &servers {
            let url = client::endpoint(server, "getGameBranches").unwrap();
            assert!(url.query().unwrap().contains(&server.game_id));
        }
    }
    assert!(client::servers("HIMI").is_empty());
    assert!(client::servers("WWMI").is_empty());
    assert_eq!(client::servers("GIMI")[2].exe, "GenshinImpact.exe");
}

#[test]
fn manifest_rejects_traversal_windows_aliases_and_invalid_layouts() {
    for path in [
        "../outside",
        "a/../../x",
        "/absolute",
        "C:/game",
        "a:b",
        "a//b",
        "AUX.txt",
        "folder/NUL",
        "a./x",
        "a /x",
        "a/*",
        ".ssmt-maintenance/journal.json",
        "dir\\..\\outside",
    ] {
        assert!(manifest::relative(path).is_err(), "{path}");
    }
    assert!(manifest::relative("Game_Data/资源/test.bin").is_ok());
    let good = asset();
    manifest::validate(&[good.clone()]).unwrap();
    let mut bad = good.clone();
    bad.chunks[0].offset = 1;
    assert!(manifest::validate(&[bad]).is_err());
    let mut duplicate = good.clone();
    duplicate.name = duplicate.name.to_uppercase();
    assert!(manifest::validate(&[good, duplicate]).is_err());
}

#[test]
fn protobuf_and_zstd_round_trip_preserve_checksums() {
    let expected = Manifest {
        files: vec![asset()],
    };
    let encoded = expected.encode_to_vec();
    let compressed = zstd::encode_all(encoded.as_slice(), 1).unwrap();
    let plain = manifest::decompress(&compressed, encoded.len() as u64, true).unwrap();
    assert_eq!(Manifest::decode(plain.as_slice()).unwrap(), expected);
    assert!(manifest::decompress(&compressed, encoded.len() as u64 - 1, true).is_err());
}

#[test]
fn official_source_validation_rejects_lookalikes_and_credentials() {
    for url in [
        "http://autopatchcn.yuanshen.com/file",
        "https://yuanshen.com.evil.test/file",
        "https://user:pass@autopatchcn.yuanshen.com/file",
        "https://127.0.0.1/file",
        "https://mihoyo.com:444/file",
    ] {
        assert!(client::validate_url(url).is_err(), "{url}");
    }
    client::validate_url("https://autopatchcn.yuanshen.com/file").unwrap();
}

#[test]
#[ignore = "只读官方接口检查；不下载客户端或写入游戏目录"]
fn live_official_branches_and_builds_match_requested_channels() {
    let http = client::http().unwrap();
    for preset in ["GIMI", "SRMI", "ZZMI"] {
        for server in client::servers(preset) {
            let (main, pre) = client::branches(&http, &server).unwrap();
            let info = client::build(&http, &server, &main, &["zh-cn".into()]).unwrap();
            let config = client::launch_config(&http, &server).unwrap();
            assert_eq!(config["exe_file_name"], server.exe);
            client::sdk(&http, &server).unwrap();
            eprintln!(
                "{preset}/{}: {} ({} categories), preload={}",
                server.id,
                main.tag,
                info.len(),
                pre.is_some()
            );
            assert!(info.iter().any(|m| m.matching_field == "game"));
            let installed = super::space::sum(info.iter().map(|m| m.install_size)).unwrap();
            let download = super::space::sum(info.iter().map(|m| m.download_size)).unwrap();
            assert!(installed > 0 && download > 0);
            eprintln!(
                "  游戏与中文语音 {:.2} GiB，下载 {:.2} GiB，流式空间 {:.2} GiB（另计清单）",
                installed as f64 / 1073741824.,
                download as f64 / 1073741824.,
                super::space::streaming_required(installed).unwrap() as f64 / 1073741824.
            );
        }
    }
}

use super::{
    engine,
    task::Task,
    transaction::{self, Change, Journal, Workspace},
    transfer,
};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) struct Temp(pub PathBuf);
impl Temp {
    pub(super) fn new() -> Self {
        let base = std::env::temp_dir().join("SSMT-HoYoPlay-tests");
        fs::create_dir_all(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let dir = base.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        // 仅递归删除本测试创建并再次解析确认的独立目录。
        let base = fs::canonicalize(std::env::temp_dir().join("SSMT-HoYoPlay-tests")).unwrap();
        if let Ok(path) = fs::canonicalize(&self.0) {
            assert_eq!(path.parent(), Some(base.as_path()));
            let _ = fs::remove_dir_all(path);
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}
fn change(name: &str, before: Option<&[u8]>, after: Option<&[u8]>) -> Change {
    Change {
        name: name.into(),
        before: before.map(digest),
        after: after.map(digest),
    }
}

#[test]
fn transaction_updates_files_and_only_owned_config_fields() {
    let temp = Temp::new();
    let root = temp.0.join("game");
    let workspace = Workspace::open(&root).unwrap();
    fs::write(root.join("game.bin"), b"old").unwrap();
    fs::write(root.join("my-mod.ini"), b"keep me").unwrap();
    fs::write(workspace.stage.join("game.bin"), b"new").unwrap();
    fs::write(workspace.stage.join("added.bin"), b"added").unwrap();
    let config = temp.0.join("Config.json");
    transaction::write_json(
        &config,
        &json!({"targetExePath":"old.exe","otherSetting":42}),
    )
    .unwrap();
    workspace
        .commit(
            vec![
                change("game.bin", Some(b"old"), Some(b"new")),
                change("added.bin", None, Some(b"added")),
            ],
            &config,
            json!({"targetExePath":"old.exe"}),
            json!({"targetExePath":"new.exe"}),
        )
        .unwrap();
    assert_eq!(fs::read(root.join("game.bin")).unwrap(), b"new");
    assert_eq!(fs::read(root.join("my-mod.ini")).unwrap(), b"keep me");
    let actual: serde_json::Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    assert_eq!(actual, json!({"targetExePath":"new.exe","otherSetting":42}));
    assert!(!workspace.work.join("journal.json").exists());
    assert!(workspace.work.join("completed.json").exists());
    workspace.cleanup_completed().unwrap();
    assert!(!workspace.backup.join("game.bin").exists());
}

#[test]
fn interrupted_commit_restores_old_files_removes_only_created_files() {
    let temp = Temp::new();
    let work = Workspace::open(&temp.0.join("game")).unwrap();
    fs::write(work.root.join("changed"), b"new").unwrap();
    fs::write(work.backup.join("changed"), b"old").unwrap();
    fs::write(work.root.join("created"), b"created").unwrap();
    fs::write(work.root.join("unrelated"), b"mod").unwrap();
    let journal = Journal {
        schema: 1,
        changes: vec![
            change("changed", Some(b"old"), Some(b"new")),
            change("created", None, Some(b"created")),
        ],
        config_before: json!({}),
        config_after: json!({}),
    };
    transaction::write_json(&work.work.join("journal.json"), &journal).unwrap();
    work.recover(Path::new("")).unwrap();
    assert_eq!(fs::read(work.root.join("changed")).unwrap(), b"old");
    assert!(!work.root.join("created").exists());
    assert_eq!(fs::read(work.root.join("unrelated")).unwrap(), b"mod");
    work.recover(Path::new("")).unwrap();
}

#[test]
fn recovery_stops_before_mutation_if_player_changed_any_target() {
    let temp = Temp::new();
    let work = Workspace::open(&temp.0.join("game")).unwrap();
    fs::write(work.root.join("changed"), b"player modification").unwrap();
    fs::write(work.backup.join("changed"), b"old").unwrap();
    let journal = Journal {
        schema: 1,
        changes: vec![change("changed", Some(b"old"), Some(b"new"))],
        config_before: json!({}),
        config_after: json!({}),
    };
    transaction::write_json(&work.work.join("journal.json"), &journal).unwrap();
    assert!(work
        .recover(Path::new(""))
        .unwrap_err()
        .contains("后续修改"));
    assert_eq!(
        fs::read(work.root.join("changed")).unwrap(),
        b"player modification"
    );
    assert_eq!(fs::read(work.backup.join("changed")).unwrap(), b"old");
    assert!(work.work.join("journal.json").exists());
}

#[cfg(windows)]
#[test]
fn locked_second_file_rolls_back_already_applied_first_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = Temp::new();
    let work = Workspace::open(&temp.0.join("game")).unwrap();
    for name in ["first", "second"] {
        fs::write(work.root.join(name), b"old").unwrap();
        fs::write(work.stage.join(name), b"new").unwrap();
    }
    let _locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(work.root.join("second"))
        .unwrap();
    let result = work.commit(
        vec![
            change("first", Some(b"old"), Some(b"new")),
            change("second", Some(b"old"), Some(b"new")),
        ],
        Path::new(""),
        json!({}),
        json!({}),
    );
    assert!(result.unwrap_err().contains("已恢复原文件"));
    assert_eq!(fs::read(work.root.join("first")).unwrap(), b"old");
    assert_eq!(fs::read(work.root.join("second")).unwrap(), b"old");
    assert!(!work.work.join("journal.json").exists());
}

#[test]
fn transaction_rejects_hash_changes_and_preserves_config_edits() {
    let temp = Temp::new();
    let work = Workspace::open(&temp.0.join("game")).unwrap();
    fs::write(work.root.join("target"), b"changed while downloading").unwrap();
    fs::write(work.stage.join("target"), b"new").unwrap();
    assert!(work
        .commit(
            vec![change("target", Some(b"old"), Some(b"new"))],
            Path::new(""),
            json!({}),
            json!({})
        )
        .is_err());
    let config = temp.0.join("Config.json");
    transaction::write_json(
        &config,
        &json!({"targetExePath":"user-choice","setting":true}),
    )
    .unwrap();
    assert!(transaction::patch_config(
        &config,
        &json!({"targetExePath":"old"}),
        &json!({"targetExePath":"new"})
    )
    .is_err());
    assert!(transaction::patch_config(
        &config,
        &json!({"pureMode":null}),
        &json!({"pureMode":true})
    )
    .is_err());
    assert!(fs::read_to_string(config).unwrap().contains("user-choice"));
}

#[test]
fn workspace_rejects_foreign_directory_and_holds_cross_process_lock() {
    let temp = Temp::new();
    let root = temp.0.join("game");
    fs::create_dir_all(root.join(".ssmt-maintenance")).unwrap();
    assert!(Workspace::open(&root).is_err());
    let _first = Workspace::open(&temp.0.join("other")).unwrap();
    #[cfg(windows)]
    assert!(Workspace::open(&temp.0.join("other")).is_err());
}

#[test]
fn channels_are_not_guessed_from_a_shared_executable() {
    let temp = Temp::new();
    fs::write(temp.0.join("StarRail.exe"), b"fixture").unwrap();
    fs::write(
        temp.0.join("config.ini"),
        b"[General]\nchannel=1\ngame_version=4.6.0\n",
    )
    .unwrap();
    assert_eq!(super::detect_server("SRMI", &temp.0).unwrap(), None);
    fs::write(
        temp.0.join("config.ini"),
        b"[General]\nchannel=1\nuapc={\"hkrpg_global\":{}}\n",
    )
    .unwrap();
    assert_eq!(
        super::detect_server("SRMI", &temp.0).unwrap().as_deref(),
        Some("global")
    );
    fs::write(temp.0.join("config.ini"), b"[General]\nchannel=14\n").unwrap();
    assert_eq!(
        super::detect_server("SRMI", &temp.0).unwrap().as_deref(),
        Some("bilibili")
    );
}

#[test]
fn config_conversion_preserves_other_sections_and_comments() {
    let original = ";keep\n[General]\nchannel=14\nsub_channel=0\ngame_version=7.1.0\ncustom=keep\n[Other]\nchannel=untouched\n";
    let result = engine::config_ini(original, &client::servers("GIMI")[2], "7.1.0");
    assert!(result.contains(";keep"));
    assert!(result.contains("custom=keep"));
    assert!(result.contains("[Other]\r\nchannel=untouched"));
    assert!(result.contains("game_biz=hk4e_global"));
    assert!(result.contains("cps=mihoyo"));
    assert!(result.contains("channel=1\r\n"));
}

#[test]
fn sdk_zip_traversal_is_rejected_without_modifying_client() {
    let temp = Temp::new();
    let work = Workspace::open(&temp.0.join("game")).unwrap();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("../outside.dll", zip::write::FileOptions::default())
        .unwrap();
    writer.write_all(b"bad").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(engine::sdk_entries(&work, &bytes, &mut Default::default(), true).is_err());
    assert!(!temp.0.join("outside.dll").exists());
}

#[test]
fn chunk_checks_both_checksums_and_uses_verified_cache_when_offline() {
    let temp = Temp::new();
    let plain = b"verified game chunk";
    let zipped = zstd::encode_all(plain.as_slice(), 1).unwrap();
    let chunk = Chunk {
        id: "opaque_chunk".into(),
        md5: digest(plain),
        compressed_md5: digest(&zipped),
        offset: 0,
        size: plain.len() as u64,
        compressed_size: zipped.len() as u64,
    };
    let cache = transfer::cache_path(&temp.0, &chunk).unwrap();
    fs::write(cache, &zipped).unwrap();
    let source = client::Download {
        url_prefix: "https://autopatchcn.yuanshen.com".into(),
        url_suffix: "".into(),
        compression: 1,
        encryption: 0,
    };
    let task = Task::new("fixture".into(), temp.0.display().to_string());
    assert_eq!(
        transfer::chunk(&client::http().unwrap(), &source, &chunk, &temp.0, &task).unwrap(),
        plain
    );
    let mut corrupt = zipped;
    corrupt[0] ^= 1;
    assert!(transfer::decode(&corrupt, &chunk, true).is_err());
    task.cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(transfer::chunk(&client::http().unwrap(), &source, &chunk, &temp.0, &task).is_err());
}

fn response(status: &str, extra: &str, body: &'static [u8]) -> reqwest::blocking::Response {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    });
    reqwest::blocking::Client::new()
        .get(format!("http://{address}/chunk"))
        .send()
        .unwrap()
}

#[test]
fn range_resume_handles_ignored_range_and_rejects_wrong_interval() {
    let temp = Temp::new();
    let part = temp.0.join("part");
    let task = Task::new("fixture".into(), String::new());
    fs::write(&part, b"ga").unwrap();
    assert_eq!(
        transfer::append_response(
            response(
                "206 Partial Content",
                "Content-Range: bytes 2-3/4\r\n",
                b"me"
            ),
            &part,
            4,
            2,
            &task
        )
        .unwrap(),
        b"game"
    );
    fs::write(&part, b"ga").unwrap();
    assert_eq!(
        transfer::append_response(response("200 OK", "", b"game"), &part, 4, 2, &task).unwrap(),
        b"game"
    );
    fs::write(&part, b"ga").unwrap();
    assert!(transfer::append_response(
        response(
            "206 Partial Content",
            "Content-Range: bytes 1-2/4\r\n",
            b"me"
        ),
        &part,
        4,
        2,
        &task
    )
    .is_err());
    assert_eq!(fs::read(part).unwrap(), b"ga");
}

#[test]
fn preload_never_blocks_launch_even_during_initial_queries() {
    let task = Task::new("fixture".into(), String::new());
    task.snapshot.lock().unwrap().operation = "preload".into();
    super::task::reserve(task.clone()).unwrap();
    for phase in ["query", "manifest", "verify", "preload"] {
        task.phase(phase, "fixture");
        super::task::assert_launch_allowed(Some("fixture")).unwrap();
        super::task::assert_launch_allowed(None).unwrap();
    }
    task.snapshot.lock().unwrap().operation = "update".into();
    assert!(super::task::assert_launch_allowed(Some("fixture")).is_err());
    super::task::assert_launch_allowed(Some("other-game")).unwrap();
    task.phase("commit", "fixture");
    assert!(super::task::assert_launch_allowed(None).is_err());
    task.finish(Ok(()));
    super::task::assert_launch_allowed(Some("fixture")).unwrap();
}

#[cfg(windows)]
#[test]
fn junction_paths_are_rejected_before_writing_outside_the_workspace() {
    use std::os::windows::process::CommandExt;
    let temp = Temp::new();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"do not change").unwrap();
    let work = Workspace::open(&temp.0.join("game")).unwrap();
    let link = work.root.join("linked");
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(status.status.success(), "无法创建隔离测试 junction");
    assert!(transaction::safe(&work.root, "linked/keep")
        .unwrap_err()
        .contains("链接"));
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"do not change");
    fs::remove_dir(link).unwrap();
}

#[test]
#[ignore = "只读下载一份官方资源清单，不下载客户端"]
fn live_manifest_checksum_and_schema_match_the_current_game() {
    let http = client::http().unwrap();
    let server = &client::servers("GIMI")[0];
    let (main, _) = client::branches(&http, server).unwrap();
    let info = client::build(&http, server, &main, &["zh-cn".into()]).unwrap();
    let files = manifest::decode(
        &http,
        info.iter().find(|m| m.matching_field == "game").unwrap(),
    )
    .unwrap();
    assert!(files.iter().any(|f| f.name == server.exe));
    eprintln!(
        "官方 {} 清单：{} 项，摘要与分块布局通过",
        main.tag,
        files.len()
    );
}
