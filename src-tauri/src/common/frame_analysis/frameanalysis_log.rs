use crate::common::shader_resource::ShaderResource;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct FrameAnalysisSingleLog {
    //所在的FrameAnalysis文件夹路径
    pub dir: String,

    /// Full path to the log.txt file (as String)
    pub log_filename: String,

    /// Convenience: the parsed lines for the primary log file
    pub lines: Vec<String>,

    /// Map: original dumped file name -> deduped file name.
    pub filename_deduped_filename_map: HashMap<String, String>,
}

impl FrameAnalysisSingleLog {
    /// Create a new FrameAnalysisSingleLog from a FrameAnalysis folder path.
    /// The function will look for `log.txt` under the given folder, read it
    /// and parse it into lines. Returns `Err(String)` on IO/parse errors.
    pub fn new(frame_analysis_folder: &str) -> Result<Self, String> {
        let folder = Path::new(frame_analysis_folder);
        let log_path: PathBuf = folder.join("log.txt");
        let log_path_str = log_path.to_string_lossy().to_string();

        let raw = std::fs::read(&log_path).map_err(|e| e.to_string())?;
        let content = String::from_utf8_lossy(&raw).into_owned();
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let filename_deduped_filename_map = Self::build_filename_deduped_filename_map(&lines);

        Ok(Self {
            dir: folder.to_string_lossy().to_string(),
            log_filename: log_path_str,
            lines,
            filename_deduped_filename_map,
        })
    }

    pub fn empty(frame_analysis_folder: &str) -> Self {
        let folder = Path::new(frame_analysis_folder);
        Self {
            dir: folder.to_string_lossy().to_string(),
            log_filename: String::new(),
            lines: Vec::new(),
            filename_deduped_filename_map: HashMap::new(),
        }
    }

    /// Reload the primary `log.txt` from disk and update stored lines.
    pub fn reload(&mut self) -> Result<(), String> {
        let path = PathBuf::from(&self.log_filename);
        let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
        let content = String::from_utf8_lossy(&raw).into_owned();
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let filename_deduped_filename_map = Self::build_filename_deduped_filename_map(&lines);
        self.lines = lines;
        self.filename_deduped_filename_map = filename_deduped_filename_map;
        Ok(())
    }

    /// 从日志行里提取 `原始文件名 -> 去重后文件名` 映射。
    fn build_filename_deduped_filename_map(lines: &[String]) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        let dumping_texture_2d = "Dumping Texture2D";
        let dumping_buffer = "Dumping Buffer";

        for log_line in lines {
            if !log_line.contains("->") {
                continue;
            }

            let start = if let Some(idx) = log_line.find(dumping_texture_2d) {
                idx + dumping_texture_2d.len()
            } else if let Some(idx) = log_line.find(dumping_buffer) {
                idx + dumping_buffer.len()
            } else {
                continue;
            };

            let path_splits: Vec<&str> = log_line[start..].split("->").collect();
            if path_splits.len() < 2 {
                continue;
            }

            let original_raw = path_splits[path_splits.len() - 2].trim();
            let deduped_raw = path_splits[path_splits.len() - 1].trim();

            let original_file_name = {
                let name = Path::new(original_raw)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(original_raw);
                name.strip_suffix(".lnk").unwrap_or(name).to_string()
            };
            let deduped_file_name = {
                let name = Path::new(deduped_raw)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(deduped_raw);
                name.strip_suffix(".lnk").unwrap_or(name).to_string()
            };

            out.insert(original_file_name, deduped_file_name);
        }

        out
    }

    /// 优先使用日志文件名；缺少映射时从真实文件或链接目标取得名称。
    pub fn get_deduped_filename(&self, frame_analysis_filename: &str) -> String {
        if let Some(filename) = self
            .filename_deduped_filename_map
            .get(frame_analysis_filename)
        {
            return filename.clone();
        }
        // 贴图同步等调用方也需要文件名，不能路径恢复成功却返回空名称。
        self.resolve_dump_filepath(frame_analysis_filename)
            .ok()
            .and_then(|path| fs::canonicalize(path).ok())
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default()
    }

    /// 日志是索引而非文件存在的唯一证据；原始转储可能是有效的符号链接或硬链接。
    pub fn resolve_dump_filepath(&self, frame_analysis_filename: &str) -> Result<PathBuf, String> {
        let name = Path::new(frame_analysis_filename);
        if frame_analysis_filename.is_empty()
            || name.components().count() != 1
            || !matches!(
                name.components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(format!(
                "原始 Dump 文件名无效或未找到: {frame_analysis_filename}"
            ));
        }
        let deduped_filename = self
            .filename_deduped_filename_map
            .get(frame_analysis_filename)
            .cloned()
            .unwrap_or_default();
        let mapped = Path::new(&self.dir).join("deduped").join(&deduped_filename);
        if !deduped_filename.is_empty() && mapped.is_file() {
            return Ok(mapped);
        }
        let original = Path::new(&self.dir).join(name);
        if original.is_file() {
            crate::extract_log!(
                "日志映射缺失或目标不可读，使用原始 Dump 文件（自动跟随文件系统链接）: {}",
                original.display()
            );
            return Ok(original);
        }
        if deduped_filename.is_empty() {
            Err(format!(
                "日志映射缺失，原始 Dump 文件也不存在或链接失效: {}",
                original.display()
            ))
        } else {
            Err(format!(
                "日志映射的 deduped 文件不存在或不可读: {}；原始 Dump 也不可读: {}",
                mapped.display(),
                original.display()
            ))
        }
    }

    // 保留旧接口；需要具体失败原因的调用点使用 resolve_dump_filepath。
    pub fn get_deduped_filepath(&self, frame_analysis_filename: &str) -> String {
        self.resolve_dump_filepath(frame_analysis_filename)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    ///通过DrawIB在日志文件中查找最后一个匹配的DrawCall索引，并返回对应的PointList索引。
    pub fn get_last_pointlist_index_by_hash(&self, draw_ib: &str) -> Option<String> {
        //此处逻辑不允许AI修改，只能等待人工后续完善

        let drawcall_index_list: Vec<String> = self.get_drawcall_index_list_by_hash(draw_ib, false);

        if drawcall_index_list.is_empty() {
            return None;
        }

        let first_trianglelist_index = &drawcall_index_list[0];
        let trianglelist_index_line_list = self.get_line_list_by_index(first_trianglelist_index);

        let mut vb0_hash = String::new();
        let mut find_ia_set_vb = false;

        for call_line in trianglelist_index_line_list {
            if call_line.contains("IASetVertexBuffers") && !find_ia_set_vb {
                find_ia_set_vb = true;
                continue;
            }

            if find_ia_set_vb {
                if !call_line.starts_with("00") {
                    let ia_resource = ShaderResource::new(&call_line);
                    if ia_resource.index == "0" {
                        vb0_hash = ia_resource.hash.clone();
                    }
                } else {
                    break;
                }
            }
        }

        if vb0_hash.is_empty() {
            return None;
        }

        let find_str = format!("hash={}", vb0_hash);
        let mut current_index = "";
        let trianglelist_index_number = match first_trianglelist_index.parse::<i32>() {
            Ok(n) => n,
            Err(_) => return None,
        };

        let mut possible_index_list: Vec<String> = Vec::new();
        for log_line in self.lines.iter() {
            if log_line.starts_with("00") {
                current_index = log_line.get(0..6).unwrap_or("");
            }

            //注意如果是CopyResource之类的操作，这并不是GPU-PreSKinning
            //例如: e9d29b50 在此处是复制的目标，而不是UnorderedAccessView或者直接设置的VB
            //000007 CopyResource(pDstResource:0x000001AE9A947120, pSrcResource:0x000001AE9A93BBA0)
            // Src: resource=0x000001AE9A93BBA0 hash=e4d2fec9
            // Dst: resource=0x000001AE9A947120 hash=e9d29b50
            if log_line.contains(&find_str) && !log_line.to_lowercase().contains("dst") {
                let pointlist_index_number = match current_index.parse::<i32>() {
                    Ok(n) => n,
                    Err(_) => continue,
                };

                if pointlist_index_number < trianglelist_index_number {
                    if !possible_index_list.contains(&current_index.to_string()) {
                        possible_index_list.push(current_index.to_string());
                    }
                }
            }
        }

        //这里返回的是最后一个，所以在部分游戏，例如Naraka手游中无法正确匹配
        //HSR也出现过这个问题
        //特殊情况下，第一个匹配到的才是正确的，所以部分情况下用的是第一个
        if !possible_index_list.is_empty() {
            return possible_index_list.last().cloned();
        }

        //如果前面的逻辑没有找到，这里默认返回None
        None
    }

    /// 读取指定 drawcall 的 `IASetVertexBuffers` 块，返回「槽位索引 -> 资源hash」映射。
    ///
    /// 例如 `{"0": "83c82ff6", "1": "5e9d4833", "2": "4a6f7397"}`。
    pub fn get_ia_vertex_buffer_hash_map(&self, drawcall_index: &str) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();

        let mut find_ia_set_vb = false;
        for call_line in self.get_line_list_by_index(drawcall_index) {
            if call_line.contains("IASetVertexBuffers") && !find_ia_set_vb {
                find_ia_set_vb = true;
                continue;
            }

            if find_ia_set_vb {
                if !call_line.starts_with("00") {
                    let ia_resource = ShaderResource::new(&call_line);
                    if !ia_resource.hash.is_empty() {
                        out.entry(ia_resource.index.clone())
                            .or_insert(ia_resource.hash.clone());
                    }
                } else {
                    break;
                }
            }
        }

        out
    }

    /// 读取指定 drawcall 中 `SOSetTargets` 写出的资源 hash 列表。
    ///
    /// GPU-PreSkinning 的预蒙皮 draw 会把蒙皮结果写进这份缓冲，
    /// 之后的渲染 draw 再把它当作顶点缓冲读回来。
    pub fn get_so_set_targets_hash_list(&self, drawcall_index: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut in_so_set_targets = false;

        for call_line in self.get_line_list_by_index(drawcall_index) {
            if call_line.contains("SOSetTargets") {
                in_so_set_targets = true;
                continue;
            }

            if in_so_set_targets {
                if !call_line.starts_with("00") {
                    let so_resource = ShaderResource::new(&call_line);
                    if !so_resource.hash.is_empty() {
                        out.push(so_resource.hash.clone());
                    }
                } else {
                    in_so_set_targets = false;
                }
            }
        }

        out
    }

    /// 通过指定渲染 draw 的 vb1(贴图坐标) hash 精确定位它对应的 PointList 索引。
    ///
    /// 与 `get_last_pointlist_index_by_hash` 的区别：
    /// 同一个 IB 的多个部件会共用同一份 GPU-PreSkinning 顶点缓冲，
    /// 只按 vb0 hash 收集候选再「取最后一个」会把别的部件的预蒙皮 draw 挑进来。
    /// 这里额外要求候选 draw 同时满足：
    /// 1) `IASetVertexBuffers` 的 vb1(贴图坐标) hash 与渲染 draw 一致；
    /// 2) 它通过 `SOSetTargets` 写出了「本渲染 draw 正在读取」的缓冲
    ///    —— 这条用来把同为渲染 draw 的候选（只读不写）剔除掉。
    ///
    /// 若无法匹配则回退到 `get_last_pointlist_index_by_hash` 原来的「取最后一个候选」行为。
    pub fn get_pointlist_index_by_texcoord_hash(
        &self,
        trianglelist_index: &str,
    ) -> Option<String> {
        let render_ia = self.get_ia_vertex_buffer_hash_map(trianglelist_index);
        let vb0_hash = render_ia.get("0").cloned().unwrap_or_default();
        let vb1_hash = render_ia.get("1").cloned().unwrap_or_default();

        if vb0_hash.is_empty() {
            return None;
        }

        let read_hash_list: Vec<String> = render_ia.values().cloned().collect();

        let trianglelist_index_number = match trianglelist_index.parse::<i32>() {
            Ok(n) => n,
            Err(_) => return None,
        };

        let find_str = format!("hash={}", vb0_hash);
        let mut current_index = "";
        let mut possible_index_list: Vec<String> = Vec::new();
        for log_line in self.lines.iter() {
            if log_line.starts_with("00") {
                current_index = log_line.get(0..6).unwrap_or("");
            }

            if log_line.contains(&find_str) && !log_line.to_lowercase().contains("dst") {
                let pointlist_index_number = match current_index.parse::<i32>() {
                    Ok(n) => n,
                    Err(_) => continue,
                };

                if pointlist_index_number < trianglelist_index_number
                    && !possible_index_list.contains(&current_index.to_string())
                {
                    possible_index_list.push(current_index.to_string());
                }
            }
        }

        if possible_index_list.is_empty() {
            return None;
        }

        if !vb1_hash.is_empty() {
            for candidate_index in possible_index_list.iter().rev() {
                let candidate_ia = self.get_ia_vertex_buffer_hash_map(candidate_index);
                if candidate_ia.get("1") != Some(&vb1_hash) {
                    continue;
                }
                let written_hash_list = self.get_so_set_targets_hash_list(candidate_index);
                if !written_hash_list
                    .iter()
                    .any(|hash| read_hash_list.contains(hash))
                {
                    continue;
                }
                return Some(candidate_index.to_string());
            }
        }

        possible_index_list.last().cloned()
    }

    /// 返回匹配给定 `draw_ib` 的 DrawCall 索引列表。
    /// 若 `only_match_first` 为 true，则找到第一个匹配即返回。
    /// TODO 这个有时候不好用，建议还是用基于FAData分析的方式
    pub fn get_drawcall_index_list_by_hash(
        &self,
        draw_ib: &str,
        only_match_first: bool,
    ) -> Vec<String> {
        let mut index_list: Vec<String> = Vec::new();
        let mut current_index = String::new();

        for line in &self.lines {
            if line.starts_with("00") && line.len() >= 6 {
                current_index = line[..6].to_string();
            }

            if line.contains(&format!("hash={}", draw_ib)) {
                if !index_list.contains(&current_index) {
                    index_list.push(current_index.clone());
                }
                if only_match_first {
                    break;
                }
            }
        }

        index_list
    }

    pub fn get_vb_category_hash_map_from_ia_set_vertex_buffer_by_index(
        &self,
        index: &str,
    ) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        let line_list = self.get_line_list_by_index(index);
        let mut find_ia_set_vb = false;

        for call_line in line_list {
            if call_line.contains("IASetVertexBuffers") && !find_ia_set_vb {
                find_ia_set_vb = true;
                continue;
            }

            if find_ia_set_vb {
                if call_line.starts_with("00") {
                    break;
                }

                let ia_resource = ShaderResource::new(&call_line);
                if ia_resource.index.is_empty() || ia_resource.hash.is_empty() {
                    continue;
                }

                out.insert(format!("vb{}", ia_resource.index), ia_resource.hash);
            }
        }

        out
    }

    pub fn get_compute_shader_slot_hash_map_from_csset_shader_resources_by_index(
        &self,
        index: &str,
    ) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        let line_list = self.get_line_list_by_index(index);
        let mut find_cs_set_sr = false;
        let command_str = "CSSetShaderResources";

        crate::extract_log!(
            "Get_ComputeShaderSlotHashMap_FromCSSetShaderResources_ByIndex:{}",
            index
        );

        for call_line in line_list {
            let call_line_trim = call_line.trim();

            if find_cs_set_sr {
                crate::extract_log!("FindCSSetShaderResources");
                if !call_line_trim.starts_with("00") {
                    crate::extract_log!("Processing {}", call_line);
                    let cs_resource = ShaderResource::new(&call_line);
                    if cs_resource.index.is_empty() || cs_resource.hash.is_empty() {
                        continue;
                    }

                    let slot = format!("cs-t{}", cs_resource.index);
                    crate::extract_log!("Slot: {} Hash: {}", slot, cs_resource.hash);
                    out.insert(slot, cs_resource.hash);
                } else if call_line.contains(command_str) {
                } else {
                    crate::extract_log!("Set FindCSSetShaderResources = False");
                    find_cs_set_sr = false;
                }
            } else if call_line.contains(command_str) {
                crate::extract_log!("Processing: {}", call_line);
                find_cs_set_sr = true;
            }
        }

        out
    }

    /// 通过 DrawIndex 来获取所有的日志行
    /// 等价于原 C# `Get_LineList_ByIndex` 方法
    pub fn get_line_list_by_index(&self, index: &str) -> Vec<String> {
        let mut index_line_list: Vec<String> = Vec::new();

        let index_number: i32 = match index.parse() {
            Ok(n) => n,
            Err(_) => return index_line_list,
        };

        let mut find_index = false;

        for line in &self.lines {
            if line.starts_with("00") && !find_index {
                if let Some(sub) = line.get(0..6) {
                    if let Ok(current_index_number) = sub.parse::<i32>() {
                        if current_index_number == index_number {
                            find_index = true;
                            index_line_list.push(line.clone());
                            continue;
                        }
                    }
                }
            }

            if find_index {
                if line.starts_with("00") {
                    if let Some(sub) = line.get(0..6) {
                        if let Ok(current_index_number) = sub.parse::<i32>() {
                            if current_index_number > index_number {
                                break;
                            } else {
                                index_line_list.push(line.clone());
                            }
                        } else {
                            index_line_list.push(line.clone());
                        }
                    } else {
                        index_line_list.push(line.clone());
                    }
                } else {
                    index_line_list.push(line.clone());
                }
            }
        }

        index_line_list
    }

    /// For `DrawIndexedInstancedIndirect` calls found in the log, parse the real
    /// `StartIndexLocation` and `IndexCount` from the indirect args buffer.
    ///
    /// Returns `Some((start_index, index_count))` on success.
    ///
    /// The buffer hash and byte offset are extracted from a log line like:
    /// ```text
    /// 000245 DrawIndexedInstancedIndirect(pBufferForArgs:0x..., AlignedByteOffsetForArgs:20) hash=96406ccf
    /// ```
    ///
    /// When multiple captures of the same buffer exist (same hash at different
    /// frame indices), the one with the highest index ≤ `draw_index` is chosen
    /// because earlier captures may contain stale/zeroed data.
    pub fn resolve_indirect_draw_params(
        &self,
        draw_index: &str,
    ) -> Option<(u64 /* StartIndexLocation */, u64 /* IndexCount */)> {
        let search = format!("{} DrawIndexedInstancedIndirect", draw_index);
        let log_line = self.lines.iter().find(|l| l.contains(&search))?;

        // Extract hash and offset: "...hash=XXXXXXXX" and "AlignedByteOffsetForArgs:NNN"
        let hash = log_line
            .split("hash=")
            .nth(1)?
            .split(|c: char| !c.is_alphanumeric())
            .next()?;
        let offset_str = log_line
            .split("AlignedByteOffsetForArgs:")
            .nth(1)?
            .split(|c: char| !c.is_ascii_digit())
            .next()?;
        let byte_offset: usize = offset_str.parse().ok()?;

        // Find the args buffer file captured closest to (but before or at)
        // the current draw call.
        let draw_index_number: i32 = draw_index.parse().ok()?;
        let hash_pattern = format!("={}", hash);
        let mut candidates: Vec<(i32, PathBuf)> = Vec::new();

        // Scan root folder
        let root_dir = Path::new(&self.dir);
        if let Ok(entries) = fs::read_dir(root_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains(&hash_pattern) {
                    if let Ok(idx) = name[..6].parse::<i32>() {
                        candidates.push((idx, entry.path()));
                    }
                }
            }
        }
        // Also scan deduped folder
        let deduped_dir = root_dir.join("deduped");
        if let Ok(entries) = fs::read_dir(&deduped_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains(&hash_pattern) {
                    if let Ok(idx) = name[..6].parse::<i32>() {
                        candidates.push((idx, entry.path()));
                    }
                }
            }
        }

        if candidates.is_empty() {
            return None;
        }

        // Pick the candidate with the highest index ≤ draw_index
        candidates.sort_by_key(|(idx, _)| *idx);
        let chosen = candidates
            .iter()
            .filter(|(idx, _)| *idx <= draw_index_number)
            .last()
            .unwrap_or(&candidates[0]);

        let args_bytes = fs::read(&chosen.1).ok()?;

        // DrawIndexedInstancedIndirect args layout (20 bytes per call):
        //   [0..4)  IndexCountPerInstance  (UINT)
        //   [4..8)  InstanceCount           (UINT)
        //   [8..12) StartIndexLocation      (UINT)
        //   [12..16) BaseVertexLocation     (INT)
        //   [16..20) StartInstanceLocation  (UINT)
        if byte_offset + 20 > args_bytes.len() {
            return None;
        }
        let index_count = u64::from(u32::from_le_bytes(
            args_bytes[byte_offset..byte_offset + 4].try_into().ok()?,
        ));
        let start_index = u64::from(u32::from_le_bytes(
            args_bytes[byte_offset + 8..byte_offset + 12]
                .try_into()
                .ok()?,
        ));
        Some((start_index, index_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_dump_uses_real_files_and_links_without_guessing_hashes() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-dump-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("deduped")).unwrap();
        let name = "000002-vb0=54c32104-vs=653c63ba4a73ca8b.buf";
        let target = root.join("deduped/54c32104.buf");
        fs::write(&target, [1, 2, 3]).unwrap();
        let mut log = FrameAnalysisSingleLog::empty(root.to_str().unwrap());

        // Hash 同名文件存在并不足以证明属于这个 Draw，不能据此静默恢复。
        assert!(log
            .resolve_dump_filepath(name)
            .unwrap_err()
            .contains("日志映射缺失"));
        fs::hard_link(&target, root.join(name)).unwrap();
        assert_eq!(log.get_deduped_filename(name), name);
        assert_eq!(
            fs::read(log.resolve_dump_filepath(name).unwrap()).unwrap(),
            [1, 2, 3]
        );

        log.filename_deduped_filename_map
            .insert(name.into(), "missing.buf".into());
        assert_eq!(log.resolve_dump_filepath(name).unwrap(), root.join(name));
        fs::remove_file(root.join(name)).unwrap();
        assert!(log
            .resolve_dump_filepath(name)
            .unwrap_err()
            .contains("deduped 文件不存在"));

        log.filename_deduped_filename_map
            .insert(name.into(), "54c32104.buf".into());
        fs::write(root.join(name), [9]).unwrap();
        assert_eq!(log.resolve_dump_filepath(name).unwrap(), target);
        assert_eq!(
            fs::read(log.resolve_dump_filepath(name).unwrap()).unwrap(),
            [1, 2, 3]
        );
        assert_eq!(log.get_deduped_filename(name), "54c32104.buf");
        for invalid in ["", "../54c32104.buf", "deduped/54c32104.buf"] {
            assert!(log
                .resolve_dump_filepath(invalid)
                .unwrap_err()
                .contains("文件名无效"));
        }
        // 只删除测试自身在临时目录中创建的目录。
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }
}
