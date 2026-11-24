use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DIRECTORY: &str = "ZZMISkeletonSource";

fn draw_line(line: &str) -> Option<(&str, &str)> {
    let (draw, payload) = line.split_once(' ')?;
    (draw.len() == 6 && draw.bytes().all(|byte| byte.is_ascii_digit())).then_some((draw, payload))
}

/// Preserve the binding information used by the Blender importer and only the
/// binary inputs for skinning. Dump paths are relative to this compact snapshot.
pub fn write(
    workspace: &Path,
    lines: &[String],
    mut resolve_dump: impl FnMut(&str) -> Option<PathBuf>,
) -> io::Result<()> {
    let mut output_draws = HashSet::new();
    let mut pending_so = None;
    let mut pose_draws = HashSet::new();
    let mut render_draws = HashSet::new();
    for line in lines {
        if let Some((draw, payload)) = draw_line(line) {
            pending_so = payload.starts_with("SOSetTargets(").then_some(draw);
            if payload.starts_with("Draw(VertexCount:") && output_draws.contains(draw) {
                pose_draws.insert(draw);
            } else if payload.starts_with("DrawIndexed(")
                || payload.starts_with("DrawIndexedInstanced(")
            {
                render_draws.insert(draw);
            }
        } else if let Some(draw) = pending_so {
            if line.trim_start().starts_with("0: resource=") && line.contains(" hash=") {
                output_draws.insert(draw);
            }
        }
    }

    let staged = workspace.join(format!(".{DIRECTORY}-{}", std::process::id()));
    let target = workspace.join(DIRECTORY);
    let backup = workspace.join(format!(".{DIRECTORY}-previous-{}", std::process::id()));
    fs::create_dir(&staged)?;
    let result = (|| {
        fs::create_dir(staged.join("deduped"))?;
        let mut output = String::new();
        let mut include_bindings = false;
        let mut copied: HashMap<PathBuf, String> = HashMap::new();
        for line in lines {
            let Some((draw, payload)) = draw_line(line) else {
                if include_bindings && line.trim_start().contains("resource=0x") {
                    output.push_str(line);
                    output.push('\n');
                }
                continue;
            };
            include_bindings = (pose_draws.contains(draw) || render_draws.contains(draw))
                && [
                    "IASetVertexBuffers(",
                    "SOSetTargets(",
                    "VSSetShaderResources(",
                    "VSSetConstantBuffers1(",
                ]
                .iter()
                .any(|prefix| payload.starts_with(prefix));
            if include_bindings
                || payload.starts_with("IASetIndexBuffer(")
                || (pose_draws.contains(draw) && payload.starts_with("Draw(VertexCount:"))
                || (render_draws.contains(draw) && payload.starts_with("DrawIndexed"))
            {
                output.push_str(line);
                output.push('\n');
            }
            let Some((logical, _)) = payload
                .strip_prefix("3DMigoto Dumping Buffer ")
                .and_then(|value| value.split_once(" -> "))
            else {
                continue;
            };
            let filename = logical.rsplit(['\\', '/']).next().unwrap_or_default();
            let (_, binding) = filename.split_once('-').unwrap_or_default();
            if !filename.ends_with(".buf")
                || !((pose_draws.contains(draw)
                    && (binding.starts_with("vs-t0=") || binding.starts_with("vs-t0-")))
                    || (render_draws.contains(draw) && binding.starts_with("vs-cb1=")))
            {
                continue;
            }
            let Some(source) = resolve_dump(filename) else {
                continue;
            };
            let basename = if let Some(name) = copied.get(&source) {
                name.clone()
            } else {
                let name = format!("{:06}.buf", copied.len());
                fs::copy(&source, staged.join("deduped").join(&name))?;
                copied.insert(source, name.clone());
                name
            };
            output.push_str(&format!(
                "{draw} 3DMigoto Dumping Buffer {filename} -> deduped/{basename}\n"
            ));
        }
        fs::write(staged.join("log.txt"), output)?;
        let had_target = target.is_dir();
        if had_target {
            fs::rename(&target, &backup)?;
        }
        if let Err(error) = fs::rename(&staged, &target) {
            if had_target {
                fs::rename(&backup, &target)?;
            }
            return Err(error);
        }
        if had_target {
            fs::remove_dir_all(&backup)?;
        }
        Ok(())
    })();
    if staged.is_dir() {
        fs::remove_dir_all(staged)?;
    }
    result
}
