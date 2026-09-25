use std::{fs, io::Read, path::Path};

pub struct SSMTFileUtils;

impl SSMTFileUtils {
    pub fn get_filename_with_new_extension(
        path: impl AsRef<Path>,
        new_extension: &str,
    ) -> Result<String, String> {
        let path = path.as_ref();
        let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
            format!(
                "Failed to get file name stem from path: {}",
                path.to_string_lossy()
            )
        })?;

        let normalized_ext = new_extension.trim_start_matches('.');
        Ok(format!("{}.{}", stem, normalized_ext))
    }

    pub fn get_file_name_without_extension(path: impl AsRef<Path>) -> Result<String, String> {
        let path = path.as_ref();
        let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
            format!(
                "Failed to get file name without extension from path: {}",
                path.to_string_lossy()
            )
        })?;
        Ok(stem.to_string())
    }

    pub fn create_folder_if_not_exists(path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();

        if path.exists() {
            if path.is_dir() {
                return Ok(());
            }
            return Err(format!(
                "Path exists but is not a directory: {}",
                path.to_string_lossy()
            ));
        }

        fs::create_dir_all(path).map_err(|e| {
            format!(
                "Failed to create directory {}: {}",
                path.to_string_lossy(),
                e
            )
        })
    }

    pub fn get_file_size(path: impl AsRef<Path>) -> Result<u64, String> {
        fs::metadata(path.as_ref())
            .map(|meta| meta.len())
            .map_err(|e| e.to_string())
    }

    pub fn is_texture_file(path: impl AsRef<Path>) -> bool {
        let ext = path
            .as_ref()
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();

        ext.eq_ignore_ascii_case("dds")
            || ext.eq_ignore_ascii_case("jpg")
            || ext.eq_ignore_ascii_case("jpeg")
            || ext.eq_ignore_ascii_case("png")
            || ext.eq_ignore_ascii_case("bmp")
            || ext.eq_ignore_ascii_case("tga")
            || ext.eq_ignore_ascii_case("gif")
            || ext.eq_ignore_ascii_case("webp")
    }

    /// Copy `source_file_path` to `target_file_path` only if target does not exist.
    /// Returns `Ok(true)` when copied, `Ok(false)` when target already exists.
    pub fn copy_to_file_if_not_exists(
        source_file_path: &str,
        target_file_path: &str,
    ) -> Result<bool, String> {
        let source = Path::new(source_file_path);
        let target = Path::new(target_file_path);

        if target.exists() {
            return Ok(false);
        }

        fs::copy(source, target).map(|_| true).map_err(|e| {
            format!(
                "Failed to copy file from {} to {}: {}",
                source.to_string_lossy(),
                target.to_string_lossy(),
                e
            )
        })
    }

    pub fn try_copy_if_exists(
        source_file_path: impl AsRef<Path>,
        target_file_path: impl AsRef<Path>,
    ) -> Result<(), String> {
        let source = source_file_path.as_ref();
        let target = target_file_path.as_ref();

        if !source.exists() {
            return Ok(());
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create directory {}: {}",
                    parent.to_string_lossy(),
                    e
                )
            })?;
        }

        fs::copy(source, target).map_err(|e| {
            format!(
                "Failed to copy file from {} to {}: {}",
                source.to_string_lossy(),
                target.to_string_lossy(),
                e
            )
        })?;

        Ok(())
    }

    pub fn file_md5(path: impl AsRef<Path>) -> Result<String, String> {
        let path_ref = path.as_ref();
        let mut file = fs::File::open(path_ref)
            .map_err(|e| format!("Failed to open file {}: {}", path_ref.to_string_lossy(), e))?;

        let mut context = md5::Context::new();
        let mut buffer = [0u8; 8192];

        loop {
            let read = file.read(&mut buffer).map_err(|e| {
                format!("Failed to read file {}: {}", path_ref.to_string_lossy(), e)
            })?;

            if read == 0 {
                break;
            }

            context.consume(&buffer[..read]);
        }

        Ok(format!("{:x}", context.compute()))
    }

    // ! Dead Code.
    // pub fn copy_boot_files(app: &AppHandle, target_dir: &Path) {
    //     let resource_dir = PathManager::ssmt_resources_folder();

    //     let files = ["d3d11.dll", "d3dcompiler_47.dll", "Run.exe", "SSMT-Player-Tweaks.dll"];

    //     for filename in files {
    //         let dest_path = target_dir.join(filename);
    //         let source_file_path = resource_dir.join(filename);
    //         println!("Copying {} to {}", filename, dest_path.to_string_lossy());
    //         println!("aaa");
    //         println!("Source file path: {}", source_file_path.to_string_lossy());
    //         println!("bbb");
    //         if let Err(e) = fs::copy(&source_file_path, &dest_path) {
    //             let msg = format!(
    //                 "Failed to copy {}: {}.\nPlease ensure the game is closed.",
    //                 filename, e
    //             );
    //             eprintln!("{}", msg);
    //             app.dialog()
    //                 .message(&msg)
    //                 .title("Copy Failed")
    //                 .kind(MessageDialogKind::Error)
    //                 .show(|_| {});
    //         } else {
    //             println!("Copied {} successfully.", filename);
    //         }
    //     }
    // }

    /// Find the value of `attribute_name` in a migoto ini-like file.
    ///
    /// Match rule is case-insensitive and expects a line like `AttributeName: value`.
    /// Returns an empty string if the attribute does not exist.
    pub fn find_migoto_ini_attribute_in_file(
        file_path: impl AsRef<Path>,
        attribute_name: &str,
    ) -> Result<String, String> {
        let raw = fs::read(file_path.as_ref()).map_err(|e| {
            format!(
                "Error reading file {}: {}",
                file_path.as_ref().to_string_lossy(),
                e
            )
        })?;
        let content = String::from_utf8_lossy(&raw);
        let attribute_name_lower = attribute_name.to_ascii_lowercase();

        for line in content.lines() {
            let trimmed = line.trim();

            if let Some(pos) = trimmed.find(':') {
                let key = trimmed[..pos].trim();
                if key.eq_ignore_ascii_case(&attribute_name_lower) {
                    return Ok(trimmed[pos + 1..].trim().to_string());
                }
            }
        }

        Ok(String::new())
    }
}
