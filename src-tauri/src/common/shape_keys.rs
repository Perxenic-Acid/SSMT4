use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NativeMorphKey {
    pub shader_hash: String,
    pub source_hash: String,
    pub constant_buffer_hash: String,
    pub offset: u32,
    pub count: u32,
    pub shape_key_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShapeKeys {
    pub offsets_hash: String,
    pub scale_hash: String,
    /// cs-t0 buffer 的 hash（ShapeKeyVertexId）
    pub vertex_ids_hash: String,
    /// cs-t1 buffer 的 hash（ShapeKeyVertexOffset）
    pub vertex_offsets_hash: String,
    /// 原生 morph 全属性增量缓冲 hash（Position/Normal/Tangent XYZ，f32）。
    #[serde(default)]
    pub vertex_attribute_offsets_hash: String,
    #[serde(default)]
    pub vertex_attribute_channel_count: u32,
    /// ShapeKey 影响的顶点总数（从 cs-cb0 前128个u32的最后一项读取）
    pub vertex_count: i32,
    pub dispatch_y: i32,
    /// checksum = cb0_u32[0] + cb0_u32[1] + cb0_u32[2] + cb0_u32[3]
    pub checksum: i32,
    /// ZZMI 原生 morph 驱动所需的 Dispatch 元数据，不适用于 WWMI。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_morph_keys: Vec<NativeMorphKey>,
}
