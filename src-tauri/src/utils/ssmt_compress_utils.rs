use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::config::path_manager::PathManager;

pub struct SSMTCompressUtils;

#[derive(Debug, Serialize, Clone)]
pub struct ArchivePreview {
    pub root_dirs: Vec<String>,
    pub file_count: usize,
    pub has_ini: bool,
    pub format: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractResult {
    pub processed: u64,
    pub total: u64,
}

impl SSMTCompressUtils {
    fn run_cli(program_path: &Path, arguments: &[String]) -> Result<String, String> {
        if !program_path.exists() {
            return Err(format!(
                "Program not found: {}",
                program_path.to_string_lossy()
            ));
        }

        let mut command = Command::new(program_path);
        command.args(arguments);

        #[cfg(windows)]
        {
            command.creation_flags(0x08000000);
        }

        let output = command
            .output()
            .map_err(|e| format!("Failed to run {}: {}", program_path.to_string_lossy(), e))?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(format!(
            "Program exited with code {:?}: {}\nstdout: {}\nstderr: {}",
            output.status.code(),
            program_path.to_string_lossy(),
            stdout,
            stderr
        ))
    }

    fn find_rar_exe() -> Option<PathBuf> {
        let mut candidates = vec![PathManager::ssmt_resources_folder().join("rar.exe")];

        if let Ok(program_files) = std::env::var("ProgramFiles") {
            candidates.push(PathBuf::from(&program_files).join("WinRAR").join("rar.exe"));
        }

        if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
            candidates.push(
                PathBuf::from(&program_files_x86)
                    .join("WinRAR")
                    .join("rar.exe"),
            );
        }

        if let Ok(path_value) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_value) {
                candidates.push(dir.join("rar.exe"));
            }
        }

        candidates.into_iter().find(|path| path.exists())
    }

    fn bundled_7za_path() -> PathBuf {
        #[cfg(windows)]
        {
            PathManager::ssmt_resources_folder().join("7za.exe")
        }

        #[cfg(not(windows))]
        {
            PathManager::ssmt_resources_folder().join("7za")
        }
    }

    fn is_ignorable_archive_entry(name: &str) -> bool {
        let normalized = name.replace('\\', "/");
        normalized.starts_with("__MACOSX") || normalized.ends_with(".DS_Store")
    }

    fn archive_entry_output_path(dest_dir: &Path, entry_name: &str) -> Result<PathBuf, String> {
        let normalized = entry_name.replace('\\', "/");
        if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
            return Err(format!("Unsafe archive entry path: {}", entry_name));
        }

        let mut outpath = dest_dir.to_path_buf();
        let mut has_segment = false;
        for segment in normalized.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." || segment.contains(':') {
                return Err(format!("Unsafe archive entry path: {}", entry_name));
            }
            outpath.push(segment);
            has_segment = true;
        }

        if !has_segment {
            return Err(format!("Empty archive entry path: {}", entry_name));
        }

        Ok(outpath)
    }

    fn resolve_single_root_dir(
        root_dirs: &HashSet<String>,
        root_files: &HashSet<String>,
    ) -> Option<String> {
        if root_dirs.len() == 1 && root_files.is_empty() {
            return root_dirs.iter().next().cloned();
        }
        None
    }

    fn collect_effective_root_items(dir: &Path) -> Result<Vec<PathBuf>, String> {
        let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
        let mut items = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name == "__MACOSX" || name == ".DS_Store" {
                continue;
            }
            items.push(path);
        }

        Ok(items)
    }

    fn collect_paths_recursive(source: &Path) -> Result<Vec<PathBuf>, String> {
        fn visit(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
            output.push(path.to_path_buf());
            if path.is_dir() {
                for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    visit(&entry.path(), output)?;
                }
            }
            Ok(())
        }

        let mut paths = Vec::new();
        visit(source, &mut paths)?;
        Ok(paths)
    }

    fn move_extracted_temp_contents(temp_dir: &Path, dest_dir: &Path) -> Result<(), String> {
        let root_items = Self::collect_effective_root_items(temp_dir)?;
        let should_strip = root_items.len() == 1 && root_items[0].is_dir();

        if should_strip {
            let root_dir = &root_items[0];
            let sub_entries = fs::read_dir(root_dir).map_err(|e| e.to_string())?;
            for sub in sub_entries {
                let sub = sub.map_err(|e| e.to_string())?;
                let sub_name = sub.file_name();
                let target = dest_dir.join(sub_name);
                fs::rename(sub.path(), target).map_err(|e| e.to_string())?;
            }
        } else {
            for item in root_items {
                let name = item
                    .file_name()
                    .ok_or_else(|| format!("Invalid extracted path: {}", item.to_string_lossy()))?;
                let target = dest_dir.join(name);
                fs::rename(&item, target).map_err(|e| e.to_string())?;
            }
        }

        let _ = fs::remove_dir_all(temp_dir);
        Ok(())
    }

    pub fn decode_zip_name(file: &zip::read::ZipFile) -> String {
        let raw = file.name_raw();
        if let Ok(s) = std::str::from_utf8(raw) {
            return s.to_string();
        }
        let (cow, _encoding, _malformed) = encoding_rs::GBK.decode(raw);
        cow.to_string()
    }

    pub fn preview_archive(path: &Path) -> Result<ArchivePreview, String> {
        Self::preview_archive_with_password(path, None)
    }

    pub fn preview_archive_with_password(
        path: &Path,
        password: Option<&str>,
    ) -> Result<ArchivePreview, String> {
        if !path.exists() {
            return Err("File not found".to_string());
        }

        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let mut root_dirs = HashSet::new();
        let mut root_files = HashSet::new();
        let mut file_count = 0;
        let mut has_ini = false;

        if ext == "zip" {
            let file = fs::File::open(path).map_err(|e| e.to_string())?;
            let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

            for i in 0..archive.len() {
                if let Ok(file) = archive.by_index(i) {
                    let name = Self::decode_zip_name(&file);
                    if Self::is_ignorable_archive_entry(&name) {
                        continue;
                    }
                    if name.ends_with('/') {
                        let parts: Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
                        if let Some(first) = parts.first() {
                            root_dirs.insert(first.to_string());
                        }
                    } else {
                        file_count += 1;
                        if name.to_lowercase().ends_with(".ini") {
                            has_ini = true;
                        }
                        let parts: Vec<&str> = name.split('/').collect();
                        if parts.len() == 1 {
                            root_files.insert(parts[0].to_string());
                        } else if let Some(first) = parts.first() {
                            root_dirs.insert(first.to_string());
                        }
                    }
                }
            }
        } else if ext == "7z" {
            let file = fs::File::open(path).map_err(|e| e.to_string())?;
            let len = file.metadata().map_err(|e| e.to_string())?.len();
            let password = sevenz_rust::Password::from("");
            let mut reader =
                sevenz_rust::SevenZReader::new(file, len, password).map_err(|e| e.to_string())?;

            reader
                .for_each_entries(|entry, _| {
                    let name = entry.name();
                    if Self::is_ignorable_archive_entry(name) {
                        return Ok(true);
                    }
                    if name.to_lowercase().ends_with(".ini") {
                        has_ini = true;
                    }
                    let parts: Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
                    if parts.len() == 1 {
                        if entry.is_directory() {
                            root_dirs.insert(parts[0].to_string());
                        } else {
                            root_files.insert(parts[0].to_string());
                        }
                    } else if let Some(first) = parts.first() {
                        root_dirs.insert(first.to_string());
                    }
                    file_count += 1;
                    Ok(true)
                })
                .map_err(|e| e.to_string())?;
        } else if ext == "rar" {
            let path_string = path.to_string_lossy().to_string();
            let archive = match password.filter(|value| !value.is_empty()) {
                Some(value) => unrar::Archive::with_password(path_string, value.to_string()),
                None => unrar::Archive::new(path_string),
            }
            .list()
            .map_err(|e| format!("Failed to open RAR archive: {}", e))?;

            for entry in archive {
                let entry = entry.map_err(|e| format!("Failed to read RAR entry: {}", e))?;
                let entry_name = entry.filename;
                file_count += 1;
                if entry_name.to_lowercase().ends_with(".ini") {
                    has_ini = true;
                }
                let normalized = entry_name.replace('\\', "/");
                let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
                if let Some(first) = parts.first() {
                    root_dirs.insert(first.to_string());
                }
            }
        } else {
            return Err("Unsupported format for preview (Currently Zip/7z/Rar)".to_string());
        }

        Ok(ArchivePreview {
            root_dirs: root_dirs.into_iter().collect(),
            file_count,
            has_ini,
            format: ext,
        })
    }

    pub fn extract_zip_archive<F>(
        archive_path: &Path,
        dest_dir: &Path,
        on_progress: F,
    ) -> Result<ExtractResult, String>
    where
        F: FnMut(u64, u64),
    {
        Self::extract_zip_archive_excluding(archive_path, dest_dir, &[], on_progress)
    }

    /// 排除指定文件名，供核心配置包更新保护由 Runtime 更新链管理的 DLL。
    pub fn extract_zip_archive_excluding<F>(
        archive_path: &Path,
        dest_dir: &Path,
        excluded_files: &[&str],
        mut on_progress: F,
    ) -> Result<ExtractResult, String>
    where
        F: FnMut(u64, u64),
    {
        let file = fs::File::open(archive_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

        let mut root_dirs = HashSet::new();
        let mut root_files = HashSet::new();

        for i in 0..archive.len() {
            if let Ok(file) = archive.by_index(i) {
                let name = Self::decode_zip_name(&file);
                if Self::is_ignorable_archive_entry(&name) {
                    continue;
                }
                let parts: Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
                if parts.is_empty() {
                    continue;
                }
                if parts.len() == 1 {
                    if name.ends_with('/') {
                        root_dirs.insert(parts[0].to_string());
                    } else {
                        root_files.insert(parts[0].to_string());
                    }
                } else {
                    root_dirs.insert(parts[0].to_string());
                }
            }
        }

        let total_entries = archive.len() as u64;
        let mut processed_entries = 0;
        on_progress(processed_entries, total_entries);

        let prefix_to_strip = Self::resolve_single_root_dir(&root_dirs, &root_files);

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = Self::decode_zip_name(&file);
            if Self::is_ignorable_archive_entry(&name) {
                continue;
            }

            let mut target_name = name.clone();
            if let Some(prefix) = &prefix_to_strip {
                if target_name.starts_with(prefix) {
                    if target_name.len() > prefix.len() {
                        target_name = target_name[prefix.len()..]
                            .trim_start_matches('/')
                            .to_string();
                    } else {
                        continue;
                    }
                }
            }

            if target_name.is_empty() {
                continue;
            }

            let outpath = Self::archive_entry_output_path(dest_dir, &target_name)?;
            if outpath
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    excluded_files
                        .iter()
                        .any(|excluded| name.eq_ignore_ascii_case(excluded))
                })
            {
                processed_entries += 1;
                on_progress(processed_entries, total_entries);
                continue;
            }

            if name.ends_with('/') {
                fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                }
                let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
                std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
            }

            processed_entries += 1;
            on_progress(processed_entries, total_entries);
        }

        Ok(ExtractResult {
            processed: processed_entries,
            total: total_entries,
        })
    }

    pub fn extract_7z_archive<F>(
        archive_path: &Path,
        dest_dir: &Path,
        mut on_progress: F,
    ) -> Result<ExtractResult, String>
    where
        F: FnMut(u64, u64),
    {
        let file = fs::File::open(archive_path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let password = sevenz_rust::Password::from("");
        let mut reader =
            sevenz_rust::SevenZReader::new(file, len, password).map_err(|e| e.to_string())?;

        let mut root_dirs = HashSet::new();
        let mut root_files = HashSet::new();
        let mut total_entries: u64 = 0;

        reader
            .for_each_entries(|entry, _| {
                let name = entry.name();
                if Self::is_ignorable_archive_entry(name) {
                    return Ok(true);
                }
                let parts: Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
                if !parts.is_empty() {
                    if parts.len() == 1 {
                        if entry.is_directory() {
                            root_dirs.insert(parts[0].to_string());
                        } else {
                            root_files.insert(parts[0].to_string());
                        }
                    } else {
                        root_dirs.insert(parts[0].to_string());
                    }
                }
                total_entries += 1;
                Ok(true)
            })
            .map_err(|e| e.to_string())?;

        let prefix_to_strip = Self::resolve_single_root_dir(&root_dirs, &root_files);

        let file = fs::File::open(archive_path).map_err(|e| e.to_string())?;
        let mut reader = sevenz_rust::SevenZReader::new(file, len, sevenz_rust::Password::from(""))
            .map_err(|e| e.to_string())?;

        let mut processed_entries: u64 = 0;
        on_progress(processed_entries, total_entries);

        reader
            .for_each_entries(|entry, reader| {
                let name = entry.name();
                if Self::is_ignorable_archive_entry(name) {
                    return Ok(true);
                }
                let mut target_name = name.to_string();

                if let Some(prefix) = &prefix_to_strip {
                    if target_name.starts_with(prefix) {
                        if target_name.len() > prefix.len() {
                            target_name = target_name[prefix.len()..]
                                .trim_start_matches('/')
                                .to_string();
                        } else {
                            return Ok(true);
                        }
                    }
                }

                if target_name.is_empty() {
                    return Ok(true);
                }

                let outpath =
                    Self::archive_entry_output_path(dest_dir, &target_name).map_err(|e| {
                        sevenz_rust::Error::Io(
                            std::io::Error::new(std::io::ErrorKind::Other, e),
                            std::borrow::Cow::Borrowed("safe path"),
                        )
                    })?;

                if entry.is_directory() {
                    fs::create_dir_all(&outpath).map_err(|e| {
                        sevenz_rust::Error::Io(
                            std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
                            std::borrow::Cow::Borrowed("create dir"),
                        )
                    })?;
                } else {
                    if let Some(p) = outpath.parent() {
                        fs::create_dir_all(p).map_err(|e| {
                            sevenz_rust::Error::Io(
                                std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
                                std::borrow::Cow::Borrowed("create parent"),
                            )
                        })?;
                    }
                    sevenz_rust::default_entry_extract_fn(entry, reader, &outpath)?;
                }

                processed_entries += 1;
                on_progress(processed_entries, total_entries);
                Ok(true)
            })
            .map_err(|e| e.to_string())?;

        Ok(ExtractResult {
            processed: processed_entries,
            total: total_entries,
        })
    }

    pub fn extract_rar_archive<F>(
        archive_path: &Path,
        dest_dir: &Path,
        on_progress: F,
    ) -> Result<ExtractResult, String>
    where
        F: FnMut(u64, u64),
    {
        Self::extract_rar_archive_with_password(archive_path, dest_dir, None, on_progress)
    }

    pub fn extract_rar_archive_with_password<F>(
        archive_path: &Path,
        dest_dir: &Path,
        password: Option<&str>,
        mut on_progress: F,
    ) -> Result<ExtractResult, String>
    where
        F: FnMut(u64, u64),
    {
        let archive_path_string = archive_path.to_string_lossy().to_string();

        // Count total entries for progress reporting
        let total_entries: u64 = {
            let archive = match password.filter(|value| !value.is_empty()) {
                Some(value) => {
                    unrar::Archive::with_password(archive_path_string.clone(), value.to_string())
                }
                None => unrar::Archive::new(archive_path_string.clone()),
            }
            .list()
            .map_err(|e| format!("Failed to open RAR archive: {}", e))?;
            archive.count() as u64
        };

        on_progress(0, total_entries);

        let temp_dir = dest_dir.join("_temp_extract");
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).map_err(|e| e.to_string())?;
        }
        fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

        // Extract to temp directory using native unrar crate
        // NOTE: extract_to only opens the archive; actual extraction happens
        // when iterating through the OpenArchive (RARProcessFile per entry).
        let archive = match password.filter(|value| !value.is_empty()) {
            Some(value) => unrar::Archive::with_password(archive_path_string, value.to_string()),
            None => unrar::Archive::new(archive_path_string),
        }
        .extract_to(temp_dir.to_string_lossy().to_string())
        .map_err(|e| format!("RAR extraction failed: {}", e))?;

        // Consume the iterator to trigger extraction for every entry
        for entry in archive {
            entry.map_err(|e| format!("RAR extraction entry error: {}", e))?;
        }

        Self::move_extracted_temp_contents(&temp_dir, dest_dir)?;

        let processed_entries = if total_entries == 0 { 1 } else { total_entries };
        on_progress(processed_entries, total_entries);

        Ok(ExtractResult {
            processed: processed_entries,
            total: total_entries,
        })
    }

    pub fn create_rar_archive(source_dir: &Path, output_path: &Path) -> Result<(), String> {
        if !source_dir.is_dir() {
            return Err(format!(
                "RAR source directory not found: {}",
                source_dir.to_string_lossy()
            ));
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        if output_path.exists() {
            fs::remove_file(output_path).map_err(|e| e.to_string())?;
        }

        let rar_exe = Self::find_rar_exe().ok_or_else(|| {
            "Cannot create RAR archive because rar.exe was not found. Install WinRAR, add rar.exe to PATH, or place rar.exe in the SSMT resources folder.".to_string()
        })?;

        let source_pattern = source_dir.join("*");
        Self::run_cli(
            &rar_exe,
            &[
                "a".to_string(),
                "-r".to_string(),
                "-ep1".to_string(),
                output_path.to_string_lossy().to_string(),
                source_pattern.to_string_lossy().to_string(),
            ],
        )
        .map(|_| ())
    }

    pub fn create_zip_archive(
        source_dir: &Path,
        output_path: &Path,
        password: Option<&str>,
    ) -> Result<(), String> {
        if let Some(value) = password.filter(|value| !value.is_empty()) {
            if !source_dir.is_dir() {
                return Err(format!(
                    "ZIP source directory not found: {}",
                    source_dir.to_string_lossy()
                ));
            }

            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }

            if output_path.exists() {
                fs::remove_file(output_path).map_err(|e| e.to_string())?;
            }

            // Use source_dir\* so files land at the archive root
            // rather than inside a folder named after the mod directory.
            let source_pattern = source_dir.join("*");
            return Self::run_cli(
                &Self::bundled_7za_path(),
                &[
                    "a".to_string(),
                    "-tzip".to_string(),
                    "-r".to_string(),
                    "-mem=AES256".to_string(),
                    format!("-p{}", value),
                    output_path.to_string_lossy().to_string(),
                    source_pattern.to_string_lossy().to_string(),
                ],
            )
            .map(|_| ());
        }

        if !source_dir.is_dir() {
            return Err(format!(
                "ZIP source directory not found: {}",
                source_dir.to_string_lossy()
            ));
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        if output_path.exists() {
            fs::remove_file(output_path).map_err(|e| e.to_string())?;
        }

        let file = fs::File::create(output_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);

        // Strip from source_dir itself so files land at the archive root
        // rather than inside an extra folder named after the mod directory.
        let paths = Self::collect_paths_recursive(source_dir)?;

        for path in paths {
            let relative_name = path
                .strip_prefix(source_dir)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if relative_name.is_empty() {
                continue;
            }

            if path.is_dir() {
                zip.add_directory(format!("{}/", relative_name.trim_end_matches('/')), options)
                    .map_err(|e| e.to_string())?;
            } else {
                zip.start_file(relative_name, options)
                    .map_err(|e| e.to_string())?;
                let mut source_file = fs::File::open(&path).map_err(|e| e.to_string())?;
                std::io::copy(&mut source_file, &mut zip).map_err(|e| e.to_string())?;
            }
        }

        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn create_7z_archive(
        source_dir: &Path,
        output_path: &Path,
        password: Option<&str>,
    ) -> Result<(), String> {
        if !source_dir.is_dir() {
            return Err(format!(
                "7z source directory not found: {}",
                source_dir.to_string_lossy()
            ));
        }

        if output_path.exists() {
            fs::remove_file(output_path).map_err(|e| e.to_string())?;
        }

        match password.filter(|value| !value.is_empty()) {
            Some(value) => sevenz_rust::compress_to_path_encrypted(
                source_dir,
                output_path,
                sevenz_rust::Password::from(value),
            ),
            None => sevenz_rust::compress_to_path(source_dir, output_path),
        }
        .map_err(|e| e.to_string())
    }

    pub fn create_mod_archive(
        source_dir: &Path,
        output_path: &Path,
        format: &str,
        password: Option<&str>,
    ) -> Result<(), String> {
        match format.to_lowercase().as_str() {
            "zip" => Self::create_zip_archive(source_dir, output_path, password),
            "7z" => Self::create_7z_archive(source_dir, output_path, password),
            "rar" => {
                if let Some(value) = password.filter(|value| !value.is_empty()) {
                    let rar_exe = Self::find_rar_exe().ok_or_else(|| {
                        "Cannot create RAR archive because rar.exe was not found. Install WinRAR, add rar.exe to PATH, or place rar.exe in the SSMT resources folder.".to_string()
                    })?;
                    if !source_dir.is_dir() {
                        return Err(format!(
                            "RAR source directory not found: {}",
                            source_dir.to_string_lossy()
                        ));
                    }
                    if let Some(parent) = output_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    if output_path.exists() {
                        fs::remove_file(output_path).map_err(|e| e.to_string())?;
                    }
                    let source_pattern = source_dir.join("*");
                    Self::run_cli(
                        &rar_exe,
                        &[
                            "a".to_string(),
                            "-r".to_string(),
                            "-ep1".to_string(),
                            format!("-p{}", value),
                            output_path.to_string_lossy().to_string(),
                            source_pattern.to_string_lossy().to_string(),
                        ],
                    )
                    .map(|_| ())
                } else {
                    Self::create_rar_archive(source_dir, output_path)
                }
            }
            _ => Err(format!("Unsupported archive format: {}", format)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SSMTCompressUtils;
    use std::path::PathBuf;

    #[test]
    fn archive_entry_output_path_allows_normal_relative_paths() {
        let dest = PathBuf::from("stage");
        let outpath = SSMTCompressUtils::archive_entry_output_path(&dest, "Root/Mod/mod.ini")
            .expect("normal archive path should be accepted");

        assert_eq!(outpath, dest.join("Root").join("Mod").join("mod.ini"));
    }

    #[test]
    fn archive_entry_output_path_rejects_parent_traversal() {
        let dest = PathBuf::from("stage");

        assert!(SSMTCompressUtils::archive_entry_output_path(&dest, "../evil.ini").is_err());
        assert!(SSMTCompressUtils::archive_entry_output_path(&dest, "Mod/../../evil.ini").is_err());
    }

    #[test]
    fn archive_entry_output_path_rejects_absolute_or_drive_paths() {
        let dest = PathBuf::from("stage");

        assert!(SSMTCompressUtils::archive_entry_output_path(&dest, "/evil.ini").is_err());
        assert!(SSMTCompressUtils::archive_entry_output_path(&dest, "C:/evil.ini").is_err());
        assert!(SSMTCompressUtils::archive_entry_output_path(&dest, "Mod/C:/evil.ini").is_err());
    }
}
