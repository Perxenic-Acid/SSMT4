use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;

pub struct PluginLogWriter {
    path: PathBuf,
}

impl PluginLogWriter {
    pub fn new(plugin_id: &str) -> Result<Self, io::Error> {
        validate_plugin_id(plugin_id)?;
        let directory = crate::config::path_manager::PathManager::ssmt_global_config_folder()
            .join("PluginLogs");
        fs::create_dir_all(&directory)?;
        Ok(Self {
            path: directory.join(format!("{plugin_id}.log")),
        })
    }

    pub fn append(&self, stream: &str, line: &str) -> Result<(), io::Error> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "[Plugin:{}] [{}] {}", self.plugin_id(), stream, line)?;
        trim_log(&self.path)
    }

    pub fn read(&self) -> Result<String, io::Error> {
        fs::read_to_string(&self.path).or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(String::new())
            } else {
                Err(error)
            }
        })
    }

    pub fn clear(&self) -> Result<(), io::Error> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn plugin_id(&self) -> &str {
        self.path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
    }
}

fn trim_log(path: &PathBuf) -> Result<(), io::Error> {
    if fs::metadata(path)?.len() as usize <= MAX_LOG_BYTES {
        return Ok(());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.read_to_end(&mut bytes)?;
    let temporary = path.with_extension("log.tmp");
    fs::write(
        &temporary,
        &bytes[bytes.len().saturating_sub(MAX_LOG_BYTES)..],
    )?;
    fs::rename(temporary, path)
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), io::Error> {
    if plugin_id.is_empty()
        || plugin_id.len() > 128
        || !plugin_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid plugin id",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_path_injection() {
        assert!(validate_plugin_id("../escape").is_err());
        assert!(validate_plugin_id("ssmt.fixture").is_ok());
    }
}
