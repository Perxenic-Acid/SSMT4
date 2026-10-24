use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::common::d3d11_element::D3D11Element;
use crate::common::shape_keys::ShapeKeys;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubMeshJson {
    #[serde(rename = "GamePreset")]
    pub game_preset: String,

    #[serde(rename = "VertexLimitVB")]
    pub vertex_limit_vb: String,

    #[serde(
        rename = "CSOutputVertexLimitVB",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub cs_output_vertex_limit_vb: String,

    #[serde(rename = "CategoryHash")]
    pub category_hash_dict: BTreeMap<String, String>,

    #[serde(rename = "match_cs", default, skip_serializing_if = "String::is_empty")]
    pub match_cs: String,

    #[serde(
        rename = "match_uav_bytes",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub match_uav_bytes: Option<usize>,

    #[serde(rename = "CategoryDrawCategoryMap")]
    pub category_draw_category_map: BTreeMap<String, String>,

    #[serde(rename = "WorkGameType")]
    pub work_game_type: String,

    #[serde(rename = "GPU-PreSkinning")]
    pub gpu_pre_skinning: bool,

    /// cb4 skeleton buffer 的 hash
    #[serde(rename = "CB4Hash")]
    pub cb4_hash: String,

    /// 导出的骨骼矩阵 buf 文件名（来自 vs-cb4，每骨骼 48 字节 = 3×float4 矩阵）
    /// Blender 插件可通过 VGMap 的 global bone index 查此 buffer 得到骨骼变换矩阵
    #[serde(
        rename = "BoneMatrixFileName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub bone_matrix_file_name: String,

    /// 此 submesh 在完整 VB 中的顶点起始位置（等同于 IB 的 min_number）
    #[serde(rename = "VertexOffset")]
    pub vertex_offset: i64,

    /// 此 submesh 涉及的唯一顶点数量
    #[serde(rename = "VertexCount")]
    pub vertex_count: i64,

    /// 此 submesh 的 IB 数据在全量 IB 中的起始索引（match_first_index）
    #[serde(rename = "IndexOffset")]
    pub index_offset: i64,

    /// 此 submesh 的 IB 元素数量（三角形数 * 3）
    #[serde(rename = "IndexCount")]
    pub index_count: i64,

    /// 生成此 submesh 的实际 FrameAnalysis draw-call 编号。
    /// 将其随 submesh 一起持久化，避免后续依赖易过期的全局组件映射。
    #[serde(
        rename = "DrawCallIndexList",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub draw_call_index_list: Vec<String>,

    /// 此 submesh 使用的顶点组数量（导出的 Blend local VG 数量）
    #[serde(rename = "VGCount")]
    pub vg_count: i32,

    /// 此 submesh 在合并骨架中的 VG 起始偏移
    /// 对 WWMI 来说，导出的 local blend index 需要通过 VGOffset 和 VGMap 还原到 merged 骨架语义
    #[serde(rename = "VGOffset")]
    pub vg_offset: i32,

    /// local VG id -> merged/global VG id 的映射
    #[serde(rename = "VGMap", default, skip_serializing_if = "HashMap::is_empty")]
    pub vg_map: HashMap<String, i32>,

    /// ShapeKey 元数据（vertex_count / dispatch_y / checksum 等）
    /// 所有 submesh 共享同一组 ShapeKey buffer，此处记录供 Blender 插件使用
    #[serde(rename = "ShapeKeysInfo")]
    pub shape_keys_info: ShapeKeys,

    #[serde(rename = "IndexBufferList")]
    pub index_buffer_list: Vec<SubMeshIndexBuffer>,

    #[serde(rename = "CategoryBufferList")]
    pub category_buffer_list: Vec<SubMeshCategoryBuffer>,

    #[serde(
        rename = "TextureMarkUpInfoList",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub texture_mark_up_info_list: Vec<SubMeshTextureMarkUpInfo>,

    /// 标记后的 DiffuseMap 文件名列表，相对当前 SubMesh JSON 所在目录。
    /// 是 TextureMarkUpInfoList 的紧凑投影，方便导入器按顺序加载叠加层。
    #[serde(rename = "DiffuseMap", default, skip_serializing_if = "Vec::is_empty")]
    pub diffuse_map: Vec<String>,

    /// 由后处理页标记的语义角色，供 Blender 插件识别面部、脖颈和眼部 SubMesh。
    /// 缺失表示该 SubMesh 未被标记。
    #[serde(
        rename = "SubMeshRole",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub sub_mesh_role: Option<SubMeshRole>,
}

impl SubMeshJson {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, save_path: P) -> Result<(), String> {
        let save_path = save_path.as_ref();
        let mut output = serde_json::to_value(self).map_err(|e| e.to_string())?;
        // 重新提取只更新几何元数据，保留同一模型上用户已有的角色标记。
        // null 是前端明确清除角色的记录，也必须保留，避免旧记忆重新补回。
        if self.sub_mesh_role.is_none() && save_path.exists() {
            let previous: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(save_path).map_err(|e| e.to_string())?)
                    .map_err(|e| {
                        format!("无法读取已有 SubMesh 元数据 {}: {}", save_path.display(), e)
                    })?;
            if previous["GamePreset"] == self.game_preset
                && previous["WorkGameType"] == self.work_game_type
            {
                if let Some(role) = previous.get("SubMeshRole") {
                    serde_json::from_value::<Option<SubMeshRole>>(role.clone())
                        .map_err(|e| format!("已有 SubMesh 角色无效: {}", e))?;
                    output["SubMeshRole"] = role.clone();
                }
            }
        }
        let content = serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?;
        fs::write(save_path, content).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubMeshIndexBuffer {
    #[serde(rename = "DXGI_FORMAT")]
    pub dxgi_format: String,

    #[serde(rename = "FileName")]
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubMeshCategoryBuffer {
    #[serde(rename = "FileName")]
    pub file_name: String,

    #[serde(rename = "Type")]
    pub buffer_type: String,

    #[serde(rename = "D3D11ElementList")]
    pub d3d11_element_list: Vec<SubMeshD3D11Element>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubMeshD3D11Element {
    #[serde(rename = "SemanticName")]
    pub semantic_name: String,

    #[serde(rename = "SemanticIndex")]
    pub semantic_index: String,

    #[serde(rename = "Format")]
    pub format: String,

    #[serde(rename = "ByteWidth")]
    pub byte_width: String,

    #[serde(rename = "ExtractSlot")]
    pub extract_slot: String,

    #[serde(rename = "ExtractTechnique")]
    pub extract_technique: String,

    #[serde(rename = "Category")]
    pub category: String,

    #[serde(rename = "DrawCategory")]
    pub draw_category: String,
}

impl SubMeshD3D11Element {
    pub fn from_d3d11_element(element: &D3D11Element) -> Self {
        Self {
            semantic_name: element.semantic_name.clone(),
            semantic_index: element.semantic_index.to_string(),
            format: element.format.clone(),
            byte_width: element.byte_width.clone(),
            extract_slot: element.extract_slot.clone(),
            extract_technique: element.extract_technique.clone(),
            category: element.category.clone(),
            draw_category: element.draw_category.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubMeshTextureMarkUpInfo {
    #[serde(rename = "MarkName")]
    pub mark_name: String,

    #[serde(rename = "MarkHash")]
    pub mark_hash: String,

    #[serde(rename = "MarkSlot")]
    pub mark_slot: String,

    #[serde(rename = "MarkType")]
    pub mark_type: String,

    #[serde(rename = "MarkFileName")]
    pub mark_file_name: String,

    #[serde(rename = "FaceSDFChannel", skip_serializing_if = "Option::is_none")]
    pub face_sdf_channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubMeshRole {
    Face,
    Neck,
    Eye,
}

#[cfg(test)]
mod role_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reextract_preserves_roles_and_explicit_clear_while_updating_geometry() {
        let dir = std::env::temp_dir().join(format!(
            "ssmt-submesh-roles-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mesh.json");
        for role in ["Face", "Neck", "Eye", "null"] {
            let value = if role == "null" {
                serde_json::Value::Null
            } else {
                role.into()
            };
            let mut old = serde_json::to_value(SubMeshJson::default()).unwrap();
            old["SubMeshRole"] = value.clone();
            fs::write(&path, serde_json::to_string(&old).unwrap()).unwrap();
            let mut extracted = SubMeshJson::default();
            extracted.vertex_count = 1382;
            extracted.save_to_file(&path).unwrap();
            let saved: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(saved["SubMeshRole"], value);
            assert_eq!(saved["VertexCount"], 1382);
            let decoded: SubMeshJson = serde_json::from_value(saved).unwrap();
            assert_eq!(decoded.sub_mesh_role.is_none(), role == "null");
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
