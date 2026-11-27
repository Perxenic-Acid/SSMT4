use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::common::d3d11_element::D3D11Element;

#[derive(Debug, Clone, Default)]
pub struct VertexBufferTxtFile {
    pub d3d11_element_list: Vec<D3D11Element>,
    pub element_name_d3d11_element_dict: HashMap<String, D3D11Element>,
    pub input_slot_by_element_name: HashMap<String, u64>,
    pub aligned_byte_offset_by_element_name: HashMap<String, u64>,
    pub stride: String,
    pub first_vertex: String,

    //注意，vertex_count不一定在txt中存在，如果不存在的话
    //应该手工读取对应buf的文件大小和stride来计算vertex_count
    pub vertex_count: String,
    pub topology: String,
    pub vertex_data_show_element_list: Vec<String>,
    pub vb_txt_file_lines: Vec<String>,
}

impl VertexBufferTxtFile {
    pub fn new(file_path: impl AsRef<Path>) -> Result<Self, String> {
        let raw = fs::read(file_path.as_ref()).map_err(|e| {
            format!(
                "Failed to read VertexBufferTxtFile {}: {}",
                file_path.as_ref().to_string_lossy(),
                e
            )
        })?;
        let content = String::from_utf8_lossy(&raw).into_owned();

        let mut out = Self {
            vb_txt_file_lines: content.lines().map(|s| s.to_string()).collect(),
            ..Self::default()
        };

        out.parse_attributes();
        out.parse_element_list();
        out.parse_vertex_data();

        Ok(out)
    }

    fn parse_attributes(&mut self) {
        for line in &self.vb_txt_file_lines {
            if line.starts_with("stride: ") {
                self.stride = line["stride: ".len()..].to_string();
            } else if line.starts_with("first vertex: ") {
                self.first_vertex = line["first vertex: ".len()..].to_string();
            } else if line.starts_with("vertex count: ") {
                self.vertex_count = line["vertex count: ".len()..].to_string();
            } else if line.starts_with("topology: ") {
                self.topology = line["topology: ".len()..].to_string();
            } else if line.starts_with("element[") {
                break;
            }
        }
    }

    fn finalize_element(
        &mut self,
        tmp_semantic_name: &str,
        tmp_semantic_index: u64,
        tmp_format: &str,
        tmp_input_slot: Option<u64>,
        tmp_aligned_byte_offset: Option<u64>,
    ) {
        if tmp_semantic_name.trim().is_empty() || tmp_format.trim().is_empty() {
            return;
        }

        let mut element = D3D11Element::default();
        element.semantic_name = tmp_semantic_name.trim().to_string();
        element.semantic_index = tmp_semantic_index;
        element.format = tmp_format.trim().to_string();
        element.byte_width = get_byte_width_from_format(&element.format).to_string();
        element.element_name = if element.semantic_index == 0 {
            element.semantic_name.clone()
        } else {
            format!("{}{}", element.semantic_name, element.semantic_index)
        };

        self.element_name_d3d11_element_dict
            .insert(element.element_name.clone(), element.clone());
        if let Some(input_slot) = tmp_input_slot {
            self.input_slot_by_element_name
                .insert(element.element_name.clone(), input_slot);
        }
        if let Some(aligned_byte_offset) = tmp_aligned_byte_offset {
            self.aligned_byte_offset_by_element_name
                .insert(element.element_name.clone(), aligned_byte_offset);
        }
        self.d3d11_element_list.push(element);
    }

    fn parse_element_list(&mut self) {
        self.d3d11_element_list.clear();
        self.element_name_d3d11_element_dict.clear();
        self.input_slot_by_element_name.clear();
        self.aligned_byte_offset_by_element_name.clear();

        let mut meet_element = false;
        let mut tmp_semantic_name = String::new();
        let mut tmp_semantic_index: u64 = 0;
        let mut tmp_format = String::new();
        let mut tmp_input_slot: Option<u64> = None;
        let mut tmp_aligned_byte_offset: Option<u64> = None;

        let lines = self.vb_txt_file_lines.clone();
        for line in &lines {
            if !meet_element {
                if line.starts_with("element[") {
                    meet_element = true;
                    continue;
                }
            } else {
                let trim_line = line.trim();
                let trim_lower = trim_line.to_ascii_lowercase();

                if trim_lower.starts_with("semanticname") {
                    if let Some((_, rhs)) = trim_line.split_once(':') {
                        tmp_semantic_name = rhs.trim().to_string();
                    }
                } else if trim_lower.starts_with("semanticindex") {
                    if let Some((_, rhs)) = trim_line.split_once(':') {
                        tmp_semantic_index = rhs.trim().parse::<u64>().unwrap_or(0);
                    }
                } else if trim_lower.starts_with("format") {
                    if let Some((_, rhs)) = trim_line.split_once(':') {
                        tmp_format = rhs.trim().to_string();
                    }
                } else if trim_lower.starts_with("inputslot:") {
                    if let Some((_, rhs)) = trim_line.split_once(':') {
                        tmp_input_slot = rhs.trim().parse::<u64>().ok();
                    }
                } else if trim_lower.starts_with("alignedbyteoffset:") {
                    if let Some((_, rhs)) = trim_line.split_once(':') {
                        tmp_aligned_byte_offset = rhs.trim().parse::<u64>().ok();
                    }
                } else if line.starts_with("element[") {
                    self.finalize_element(
                        &tmp_semantic_name,
                        tmp_semantic_index,
                        &tmp_format,
                        tmp_input_slot,
                        tmp_aligned_byte_offset,
                    );
                    tmp_semantic_name.clear();
                    tmp_semantic_index = 0;
                    tmp_format.clear();
                    tmp_input_slot = None;
                    tmp_aligned_byte_offset = None;
                } else if line.starts_with("vertex-data:") {
                    self.finalize_element(
                        &tmp_semantic_name,
                        tmp_semantic_index,
                        &tmp_format,
                        tmp_input_slot,
                        tmp_aligned_byte_offset,
                    );
                    tmp_semantic_name.clear();
                    tmp_semantic_index = 0;
                    tmp_format.clear();
                    tmp_input_slot = None;
                    tmp_aligned_byte_offset = None;
                    break;
                }
            }
        }

        if !tmp_semantic_name.is_empty() && !tmp_format.is_empty() {
            self.finalize_element(
                &tmp_semantic_name,
                tmp_semantic_index,
                &tmp_format,
                tmp_input_slot,
                tmp_aligned_byte_offset,
            );
        }
    }

    fn parse_vertex_data(&mut self) {
        let mut meet_vertex_data = false;
        let mut meet_real_line_count: usize = 0;
        let mut show_element_data_set: HashSet<String> = HashSet::new();

        for line in &self.vb_txt_file_lines {
            if meet_real_line_count > 10 {
                break;
            }

            if !meet_vertex_data {
                if line.starts_with("vertex-data:") {
                    meet_vertex_data = true;
                }
                continue;
            }

            let trim_line = line.trim();
            if !trim_line.to_ascii_lowercase().starts_with("vb") {
                continue;
            }

            meet_real_line_count += 1;

            if let Some((left, _)) = trim_line.split_once(':') {
                let mut left_tokens = left.split_whitespace();
                let _vb_slot = left_tokens.next();
                if let Some(element_name) = left_tokens.next() {
                    show_element_data_set.insert(element_name.trim().to_string());
                }
            }
        }

        self.vertex_data_show_element_list = show_element_data_set.into_iter().collect();
    }
}

fn get_byte_width_from_format(format: &str) -> u64 {
    let fmt = format.to_lowercase();
    let bytes: u64 = fmt.as_bytes().windows(3).fold(0, |acc, w| {
        if w[1] == b'8' {
            acc + 1
        } else if w[1] == b'3' && w[2] == b'2' {
            acc + 4
        } else if w[1] == b'1' && w[2] == b'6' {
            acc + 2
        } else {
            acc
        }
    });

    if bytes > 0 {
        bytes
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_aligned_offsets_and_aliases_without_duplicating_last_element() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ssmt-vb-layout-{}-{suffix}.txt",
            std::process::id()
        ));
        fs::write(
            &path,
            [
                "stride: 20",
                "first vertex: 0",
                "vertex count: 1",
                "topology: trianglelist",
                "element[0]:",
                "  SemanticName: COLOR",
                "  SemanticIndex: 0",
                "  Format: R8G8B8A8_UNORM",
                "  InputSlot: 1",
                "  AlignedByteOffset: 0",
                "  InputSlotClass: per-vertex",
                "element[1]:",
                "  SemanticName: TEXCOORD",
                "  SemanticIndex: 0",
                "  Format: R32G32_FLOAT",
                "  InputSlot: 1",
                "  AlignedByteOffset: 4",
                "  InputSlotClass: per-vertex",
                "element[2]:",
                "  SemanticName: TEXCOORD",
                "  SemanticIndex: 1",
                "  Format: R32G32_FLOAT",
                "  InputSlot: 1",
                "  AlignedByteOffset: 12",
                "  InputSlotClass: per-vertex",
                "element[3]:",
                "  SemanticName: TEXCOORD",
                "  SemanticIndex: 2",
                "  Format: R32G32_FLOAT",
                "  InputSlot: 1",
                "  AlignedByteOffset: 12",
                "  InputSlotClass: per-vertex",
                "vertex-data:",
            ]
            .join("\n"),
        )
        .unwrap();

        let parsed = VertexBufferTxtFile::new(&path).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(parsed.stride, "20");
        assert_eq!(parsed.d3d11_element_list.len(), 4);
        assert_eq!(parsed.aligned_byte_offset_by_element_name["TEXCOORD"], 4);
        assert_eq!(parsed.aligned_byte_offset_by_element_name["TEXCOORD1"], 12);
        assert_eq!(parsed.aligned_byte_offset_by_element_name["TEXCOORD2"], 12);
        assert_eq!(parsed.input_slot_by_element_name["TEXCOORD"], 1);
    }
}
