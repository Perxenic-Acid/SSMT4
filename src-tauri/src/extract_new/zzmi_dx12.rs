use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::frame_analysis::frameanalysis_data::FrameAnalysisData;
use crate::common::frame_analysis::frameanalysis_dx12_log::{
    FrameAnalysisDX12DrawCall, FrameAnalysisDX12Log, FrameAnalysisDX12ResourceBinding,
    FrameAnalysisDX12VertexBinding,
};
use crate::common::frame_analysis::frameanalysis_log::FrameAnalysisSingleLog;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::common::vertex_buffer_txt_file::VertexBufferTxtFile;
use crate::config::drawib_config::{DrawIBConfig, DrawIBEntry};
use crate::config::path_manager::PathManager;
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::helper::mark_texture_helper::{
    get_workspace_component_name_draw_call_index_list_json_path,
    get_workspace_trianglelist_deduped_filename_json_path, ComponentNameDrawCallIndexListJson,
    TrianglelistDedupedFileNameJson, TrianglelistDedupedTextureProperty,
};
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::utils::ssmt_string_utils::SSMTStringUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshD3D11Element, SubMeshIndexBuffer, SubMeshJson,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct DX12ResolvedBinding {
    slot: u32,
    bytes: usize,
    stride: u64,
    offset: usize,
    file: String,
    /// IA-format hash (DX12HashIaBufferView) — for VertexLimitVB, mod VB matching
    override_hash: String,
    /// CS descriptor-format hash (DX12HashDescriptorBufferView) — for CategoryHash,
    /// match_cs_t0/t1 INI fields, matches against CS SRV BindResource records
    cs_descriptor_hash: String,
    match_cs: String,
    match_uav_bytes: Option<usize>,
    source: String,
}

#[derive(Debug, Clone)]
struct DX12GpuSourceSet {
    candidates: Vec<FrameAnalysisDX12ResourceBinding>,
    match_cs: String,
    match_uav_bytes: Option<usize>,
    producer_call_index: u64,
    source_label: String,
}

pub struct ZZMIDX12NewExtractor {
    fa: FrameAnalysis,
    dx12_log: Option<FrameAnalysisDX12Log>,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl ZZMIDX12NewExtractor {
    fn dx12_resolve_binding_path(&self, relative_path: &str) -> PathBuf {
        self.dx12_log
            .as_ref()
            .map(|log| log.resolve_binding_path(relative_path))
            .unwrap_or_else(|| Path::new(&self.fa.folder_path).join(relative_path))
    }

    /// Parse extract slot string to a slot number.
    /// "vb0".."vbN" → N,  "cs-t0".."cs-tN" → 100+N (CS register marker).
    fn dx12_slot_from_extract_slot(extract_slot: &str) -> Option<u32> {
        let lower = extract_slot.trim().to_ascii_lowercase();
        if let Some(digits) = lower.strip_prefix("vb") {
            return digits.parse::<u32>().ok();
        }
        if let Some(digits) = lower.strip_prefix("cs-t") {
            return digits.parse::<u32>().ok().map(|reg| 100 + reg);
        }
        None
    }

    /// Check if extract slot is a CS register slot (cs-tN).
    fn dx12_is_cs_slot(extract_slot: &str) -> bool {
        extract_slot.trim().to_ascii_lowercase().starts_with("cs-t")
    }

    fn dx12_match_hash(hunt_hash: &str, hash: &str, file: &str) -> String {
        let hunt_hash = hunt_hash.trim();
        if !hunt_hash.is_empty() {
            return hunt_hash.to_string();
        }

        let hash = hash.trim();
        if !hash.is_empty() {
            return hash.to_string();
        }

        Path::new(file)
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split('-').next())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    /// Returns the CS descriptor-format hash for CategoryHash.
    /// This is DX12HashDescriptorBufferView — matches against CS SRV BindResource records.
    fn dx12_category_hash(binding: Option<&DX12ResolvedBinding>) -> String {
        binding
            .map(|b| b.cs_descriptor_hash.clone())
            .unwrap_or_default()
    }

    /// Returns the IA-format hash for VertexLimitVB.
    /// This is DX12HashIaBufferView — matches against BindIA records.
    fn dx12_vertex_limit_hash(binding: Option<&DX12ResolvedBinding>) -> String {
        binding.map(|b| b.override_hash.clone()).unwrap_or_default()
    }

    fn dx12_index_hash(draw: &FrameAnalysisDX12DrawCall) -> String {
        draw.index_binding
            .as_ref()
            .map(|ib| Self::dx12_match_hash(&ib.hunt_hash, &ib.hash, &ib.file))
            .unwrap_or_else(|| format!("DX12-{}", draw.call_index))
    }

    fn dx12_submesh_name(draw: &FrameAnalysisDX12DrawCall) -> String {
        format!(
            "{}-{}-{}",
            Self::dx12_index_hash(draw),
            draw.index_count,
            draw.start_index
        )
    }

    fn dx12_effective_offset(file_size: usize, offset: usize, bytes: usize) -> usize {
        if offset >= file_size || (bytes > 0 && bytes == file_size) {
            0
        } else {
            offset
        }
    }

    fn dx12_binding_vertex_count(&self, binding: &DX12ResolvedBinding) -> Result<u64, String> {
        if binding.stride == 0 {
            return Ok(0);
        }
        let path = self.dx12_resolve_binding_path(&binding.file);
        if !path.exists() {
            return Ok(0);
        }
        let file_size = SSMTFileUtils::get_file_size(&path)? as usize;
        let offset = Self::dx12_effective_offset(file_size, binding.offset, binding.bytes);
        let available = if binding.bytes > 0 && binding.bytes <= file_size.saturating_sub(offset) {
            binding.bytes
        } else {
            file_size.saturating_sub(offset)
        };
        Ok((available as u64) / binding.stride)
    }

    fn dx12_binding_from_vertex(binding: &FrameAnalysisDX12VertexBinding) -> DX12ResolvedBinding {
        let override_hash = Self::dx12_match_hash(&binding.hunt_hash, &binding.hash, &binding.file);
        let (use_offset, use_bytes) = if binding.file_offset > 0 || binding.file_bytes > 0 {
            (binding.file_offset, binding.file_bytes)
        } else {
            (binding.offset, binding.bytes)
        };
        DX12ResolvedBinding {
            slot: binding.slot,
            bytes: use_bytes,
            stride: binding.stride,
            offset: use_offset,
            file: binding.file.clone(),
            override_hash: override_hash.clone(),
            cs_descriptor_hash: override_hash,
            match_cs: String::new(),
            match_uav_bytes: None,
            source: format!("draw-vb{}", binding.slot),
        }
    }

    fn dx12_binding_from_resource(
        binding: &FrameAnalysisDX12ResourceBinding,
        slot: u32,
        override_hash: String,
        cs_descriptor_hash: String,
        match_cs: String,
        match_uav_bytes: Option<usize>,
        source: impl Into<String>,
    ) -> DX12ResolvedBinding {
        DX12ResolvedBinding {
            slot,
            bytes: binding.bytes,
            stride: binding.stride,
            offset: binding.offset,
            file: binding.file.clone(),
            override_hash,
            cs_descriptor_hash,
            match_cs,
            match_uav_bytes,
            source: source.into(),
        }
    }

    fn dx12_draw_slot_binding(
        draw: &FrameAnalysisDX12DrawCall,
        slot: u32,
    ) -> Option<DX12ResolvedBinding> {
        draw.vertex_bindings
            .get(&slot)
            .map(Self::dx12_binding_from_vertex)
    }

    fn dx12_is_gpu_preskinning_vb(binding: &FrameAnalysisDX12VertexBinding) -> bool {
        binding
            .skin_source
            .trim()
            .eq_ignore_ascii_case("gpu_preskinning")
    }

    fn dx12_field_eq(lhs: &str, rhs: &str) -> bool {
        let lhs = lhs.trim();
        let rhs = rhs.trim();
        !lhs.is_empty() && !rhs.is_empty() && lhs.eq_ignore_ascii_case(rhs)
    }

    fn dx12_bind_is_compute(bind: &str) -> bool {
        bind.trim().eq_ignore_ascii_case("compute_cbv_srv_uav")
    }

    fn dx12_gpu_source_sets_for_draw(
        &self,
        draw: &FrameAnalysisDX12DrawCall,
    ) -> Vec<DX12GpuSourceSet> {
        let Some(dx12_log) = self.dx12_log.as_ref() else {
            return Vec::new();
        };

        let mut gpu_bindings = draw
            .vertex_bindings
            .values()
            .filter(|binding| Self::dx12_is_gpu_preskinning_vb(binding))
            .collect::<Vec<&FrameAnalysisDX12VertexBinding>>();
        gpu_bindings.sort_by_key(|binding| binding.slot);

        let mut result = Vec::new();
        for vb in gpu_bindings {
            if let Some(source_set) = self.dx12_direct_gpu_source_set(vb) {
                result.push(source_set);
                continue;
            }

            if let Some(source_set) = self.dx12_resource_fallback_gpu_source_set(draw, vb, dx12_log)
            {
                result.push(source_set);
                continue;
            }

            crate::extract_log!(
                "[ZZMIDX12][GPU-PreSkinning] call_index={} vb_slot={} producer_call={} resource={} has no usable producer SRVs; fallback to draw VB",
                draw.call_index, vb.slot, vb.producer_call_index, vb.resource
            );
        }

        result
    }

    fn dx12_direct_gpu_source_set(
        &self,
        vb: &FrameAnalysisDX12VertexBinding,
    ) -> Option<DX12GpuSourceSet> {
        if vb.producer_call_index == 0
            || vb.producer_cs.trim().is_empty()
            || vb.producer_cs.trim() == "-"
        {
            return None;
        }
        let dx12_log = self.dx12_log.as_ref()?;
        let producer_bindings = dx12_log.resource_bindings_for_call(vb.producer_call_index);
        let matched_uav = producer_bindings.iter().find(|binding| {
            binding.kind.eq_ignore_ascii_case("UAV")
                && binding.root == vb.producer_root
                && binding.reg == vb.producer_reg
                && binding.bytes == vb.bytes
                && binding.stride == vb.stride
                && (vb.resource.trim().is_empty()
                    || binding.resource.trim().is_empty()
                    || Self::dx12_field_eq(&binding.resource, &vb.resource))
        })?;

        let candidates = producer_bindings
            .iter()
            .filter(|binding| {
                binding.kind.eq_ignore_ascii_case("SRV")
                    && Self::dx12_field_eq(&binding.bind, &vb.producer_bind)
            })
            .cloned()
            .collect::<Vec<FrameAnalysisDX12ResourceBinding>>();

        if candidates.is_empty() {
            return None;
        }

        Some(DX12GpuSourceSet {
            candidates,
            match_cs: vb.producer_cs.clone(),
            match_uav_bytes: Some(matched_uav.bytes),
            producer_call_index: vb.producer_call_index,
            source_label: "dispatch-direct".to_string(),
        })
    }

    fn dx12_resource_fallback_gpu_source_set(
        &self,
        draw: &FrameAnalysisDX12DrawCall,
        vb: &FrameAnalysisDX12VertexBinding,
        dx12_log: &FrameAnalysisDX12Log,
    ) -> Option<DX12GpuSourceSet> {
        let resource = vb.resource.trim();
        if resource.is_empty() {
            return None;
        }

        let producer_limit = if vb.producer_call_index > 0 {
            vb.producer_call_index.min(draw.call_index)
        } else {
            draw.call_index
        };

        let matched_uav = dx12_log
            .resource_bindings
            .iter()
            .filter(|binding| {
                binding.call_index <= producer_limit
                    && binding.kind.eq_ignore_ascii_case("UAV")
                    && Self::dx12_bind_is_compute(&binding.bind)
                    && Self::dx12_field_eq(&binding.resource, resource)
                    && (vb.stride == 0 || binding.stride == 0 || binding.stride == vb.stride)
            })
            .max_by_key(|binding| binding.call_index)?;

        let producer_bindings = dx12_log.resource_bindings_for_call(matched_uav.call_index);
        let candidates = producer_bindings
            .iter()
            .filter(|binding| {
                binding.kind.eq_ignore_ascii_case("SRV")
                    && Self::dx12_field_eq(&binding.bind, &matched_uav.bind)
            })
            .cloned()
            .collect::<Vec<FrameAnalysisDX12ResourceBinding>>();

        if candidates.is_empty() {
            return None;
        }

        Some(DX12GpuSourceSet {
            candidates,
            match_cs: matched_uav.cs.clone(),
            match_uav_bytes: Some(matched_uav.bytes),
            producer_call_index: matched_uav.call_index,
            source_label: "dispatch-resource".to_string(),
        })
    }

    fn dx12_select_gpu_source_binding(
        source_sets: &[DX12GpuSourceSet],
        draw_binding: Option<&FrameAnalysisDX12VertexBinding>,
        category_name: &str,
        expected_stride: u64,
        used_regs: &mut HashSet<(u64, u32)>,
    ) -> Option<DX12ResolvedBinding> {
        source_sets.iter().find_map(|source_set| {
            Self::dx12_select_gpu_source_binding_from_set(
                source_set,
                draw_binding,
                category_name,
                expected_stride,
                used_regs,
            )
        })
    }

    fn dx12_select_gpu_source_binding_from_set(
        source_set: &DX12GpuSourceSet,
        draw_binding: Option<&FrameAnalysisDX12VertexBinding>,
        category_name: &str,
        expected_stride: u64,
        used_regs: &mut HashSet<(u64, u32)>,
    ) -> Option<DX12ResolvedBinding> {
        let candidates = &source_set.candidates;
        let matching_stride = candidates
            .iter()
            .filter(|binding| {
                binding.stride == expected_stride
                    && !used_regs.contains(&(source_set.producer_call_index, binding.reg))
            })
            .collect::<Vec<&FrameAnalysisDX12ResourceBinding>>();

        // For GPU pre-skinning, the compute shader outputs SRVs in a
        // predictable order matching the VB slot layout:
        //   r0 = Position, r1 = Blend, r2+ = other data (shape keys etc.)
        // Texcoord is typically NOT in the GPU CS output — it comes from
        // IA vertex buffer (trianglelist). Each category with a preferred_reg
        // does strict reg matching; if the exact reg+stride doesn't exist,
        // it falls back to IA VB via the caller.
        let preferred_reg = match category_name {
            "POSITION" => Some(0u32),
            "BLEND" => Some(1u32),
            "TEXCOORD" => Some(2u32),
            _ => None,
        };

        // Strict reg match — no max_bytes fallback. If the preferred reg
        // doesn't have the right stride, the category should NOT steal
        // another category's SRV. The caller will fall back to IA VB.
        let selected = if let Some(reg) = preferred_reg {
            matching_stride
                .iter()
                .copied()
                .find(|binding| binding.reg == reg)?
        } else {
            matching_stride
                .iter()
                .copied()
                .max_by_key(|binding| binding.bytes)?
        };

        // IA-format hash (DX12HashIaBufferView): from draw_binding (BindIA record).
        // Used for VertexLimitVB — mod system matches against BindIA entries.
        let override_hash = draw_binding
            .map(|binding| Self::dx12_match_hash(&binding.hunt_hash, &binding.hash, &binding.file))
            .unwrap_or_else(|| {
                Self::dx12_match_hash(&selected.hunt_hash, &selected.hash, &selected.file)
            });

        // CS descriptor-format hash (DX12HashDescriptorBufferView): from the
        // SRV resource binding. Used for CategoryHash — mod INI's
        // match_cs_t0/t1_hash matches against CS BindResource SRV records.
        let cs_descriptor_hash =
            Self::dx12_match_hash(&selected.hunt_hash, &selected.hash, &selected.file);

        // Mark this (producer_call, reg) as claimed so subsequent
        // categories cannot reuse the same GPU SRV.
        used_regs.insert((source_set.producer_call_index, selected.reg));

        Some(Self::dx12_binding_from_resource(
            selected,
            selected.reg,
            override_hash,
            cs_descriptor_hash,
            source_set.match_cs.clone(),
            source_set.match_uav_bytes,
            format!(
                "{}-call{}-srv-r{}",
                source_set.source_label, source_set.producer_call_index, selected.reg
            ),
        ))
    }

    fn dx12_resolved_category_bindings(
        &self,
        draw: &FrameAnalysisDX12DrawCall,
        d3d11_game_type: &D3D11GameType,
    ) -> Option<HashMap<String, DX12ResolvedBinding>> {
        let mut result = HashMap::new();
        let gpu_source_sets = if d3d11_game_type.gpu_pre_skinning {
            self.dx12_gpu_source_sets_for_draw(draw)
        } else {
            Vec::new()
        };

        // Track (producer_call_index, reg) pairs already claimed by a
        // previous category. Prevents two categories from resolving to
        // the same GPU SRV (e.g. Texcoord and Blend both at stride 32
        // would otherwise both pick r1).
        let mut used_regs: HashSet<(u64, u32)> = HashSet::new();

        for category_name in &d3d11_game_type.ordered_category_name_list {
            let slot = d3d11_game_type.category_slot_dict.get(category_name)?;
            let is_cs_slot = Self::dx12_is_cs_slot(slot);
            let dx12_slot = Self::dx12_slot_from_extract_slot(slot)?;
            let stride = d3d11_game_type
                .category_stride_dict
                .get(category_name)
                .copied()
                .unwrap_or(0);

            // Per-category topology routing:
            //   pointlist / cs-tN → resolve from GPU pre-skinning CS output
            //   trianglelist       → resolve from draw IA vertex buffer
            let topology = d3d11_game_type
                .category_topology_dict
                .get(category_name)
                .map(|s| s.as_str())
                .unwrap_or("");
            let is_pointlist = topology.eq_ignore_ascii_case("pointlist");

            // CS slots (cs-t0, cs-t1, ...) always use GPU source — they
            // have no corresponding IA VB. If no GPU SRV exists, the
            // game type does not match.
            let use_gpu = is_cs_slot || (d3d11_game_type.gpu_pre_skinning && is_pointlist);

            let binding = if use_gpu {
                // CS slots have no draw VB binding.
                let draw_binding = if is_cs_slot {
                    None
                } else {
                    draw.vertex_bindings.get(&dx12_slot)
                };
                let binding = Self::dx12_select_gpu_source_binding(
                    &gpu_source_sets,
                    draw_binding,
                    category_name,
                    stride,
                    &mut used_regs,
                );
                match binding {
                    Some(binding) => {
                        if binding.file.is_empty()
                            || !self.dx12_resolve_binding_path(&binding.file).exists()
                        {
                            return None;
                        }
                        binding
                    }
                    None => {
                        // CS slots: no GPU SRV → fail.
                        // VB pointlist: no GPU SRV → fall back to IA VB.
                        if is_cs_slot {
                            return None;
                        }
                        Self::dx12_draw_slot_binding(draw, dx12_slot)?
                    }
                }
            } else {
                Self::dx12_draw_slot_binding(draw, dx12_slot)?
            };
            result.insert(category_name.clone(), binding);
        }
        Some(result)
    }

    fn dx12_matching_gametypes(
        &self,
        draw: &FrameAnalysisDX12DrawCall,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<Vec<D3D11GameType>, String> {
        let possible_gpu =
            self.dx12_matching_gametypes_by_preskinning(draw, data_type_filter, true)?;

        if !possible_gpu.is_empty() {
            // GPU types: keep all matches (matching zzmi.rs behavior).
            // Do NOT filter by max category count — multiple GPU data types
            // with different texcoord layouts should all be exported.
            crate::extract_log!(
                "[ZZMIDX12][GPU-PreSkinning] call_index={} matched {} GPU data type(s)",
                draw.call_index,
                possible_gpu.len()
            );
            return Ok(possible_gpu);
        }

        let possible_cpu =
            self.dx12_matching_gametypes_by_preskinning(draw, data_type_filter, false)?;
        // CPU types: only keep those with max category count to avoid
        // spurious subset matches.
        Ok(Self::dx12_keep_max_category_count(possible_cpu))
    }

    fn dx12_explain_unmatched_draw(&self, draw: &FrameAnalysisDX12DrawCall) -> String {
        let mut reasons = Vec::new();

        if let Some(ib) = draw.index_binding.as_ref() {
            if ib.file.is_empty() {
                reasons.push(format!(
                    "missing dumped IB file for gpu={} bytes={} scope={}",
                    ib.gpu, ib.bytes, ib.dump_scope
                ));
            }
        } else {
            reasons.push("missing index binding".to_string());
        }

        if draw.vertex_bindings.is_empty() {
            let graphics_buffer_srvs = self
                .dx12_log
                .as_ref()
                .map(|log| {
                    log.resource_bindings_for_call(draw.call_index)
                        .iter()
                        .filter(|binding| {
                            binding.kind.eq_ignore_ascii_case("SRV")
                                && binding.bind.eq_ignore_ascii_case("graphics_cbv_srv_uav")
                                && binding.stride > 0
                        })
                        .map(|binding| {
                            format!(
                                "r{}:{}Bx{}:{}",
                                binding.reg, binding.bytes, binding.stride, binding.file
                            )
                        })
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();

            if graphics_buffer_srvs.is_empty() {
                reasons.push("no IA vertex bindings and no graphics SRV buffers".to_string());
            } else {
                reasons.push(format!(
                    "no IA vertex bindings; graphics SRV buffers only [{}]",
                    graphics_buffer_srvs.join(", ")
                ));
            }
        }

        let gpu_source_sets = self.dx12_gpu_source_sets_for_draw(draw);
        if !gpu_source_sets.is_empty() {
            let summaries = gpu_source_sets
                .iter()
                .map(|source| {
                    let strides = source
                        .candidates
                        .iter()
                        .map(|binding| format!("r{}:{}", binding.reg, binding.stride))
                        .collect::<Vec<String>>()
                        .join("/");
                    format!(
                        "{}@call{} [{}]",
                        source.source_label, source.producer_call_index, strides
                    )
                })
                .collect::<Vec<String>>();
            reasons.push(format!("gpu source sets {}", summaries.join(", ")));
        }

        if reasons.is_empty() {
            "no compatible ZZMIDX12 category bindings found".to_string()
        } else {
            reasons.join("; ")
        }
    }

    fn dx12_matching_gametypes_by_preskinning(
        &self,
        draw: &FrameAnalysisDX12DrawCall,
        data_type_filter: FullExtractDataTypeFilter,
        gpu_pre_skinning: bool,
    ) -> Result<Vec<D3D11GameType>, String> {
        if !data_type_filter.allows(gpu_pre_skinning) {
            return Ok(Vec::new());
        }

        let mut possible = Vec::new();
        for gt in self.d3d11_gametype_lv2.d3d11_game_type_list.iter() {
            if gt.gpu_pre_skinning != gpu_pre_skinning {
                continue;
            }
            if !data_type_filter.allows(gt.gpu_pre_skinning) {
                continue;
            }
            let Some(category_bindings) = self.dx12_resolved_category_bindings(draw, gt) else {
                continue;
            };

            let mut vertex_number = 0u64;
            let mut all_match = true;
            for category_name in &gt.ordered_category_name_list {
                let stride = gt
                    .category_stride_dict
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);
                if stride == 0 {
                    all_match = false;
                    break;
                }
                let Some(binding) = category_bindings.get(category_name) else {
                    all_match = false;
                    break;
                };
                let count = self.dx12_binding_vertex_count(binding)?;
                if count == 0 {
                    all_match = false;
                    break;
                }

                if binding.stride != stride {
                    all_match = false;
                    break;
                }

                if vertex_number == 0 {
                    vertex_number = count;
                } else if vertex_number != count {
                    all_match = false;
                    break;
                }
            }

            if all_match {
                possible.push(gt.clone());
            }
        }

        Ok(possible)
    }

    fn dx12_keep_max_category_count(mut possible: Vec<D3D11GameType>) -> Vec<D3D11GameType> {
        if !possible.is_empty() {
            let max_category_number = possible
                .iter()
                .map(|gt| gt.category_slot_dict.len())
                .max()
                .unwrap_or(0);
            possible.retain(|gt| gt.category_slot_dict.len() == max_category_number);
        }
        possible
    }

    fn dx12_copy_category_buffer(
        &self,
        binding: &DX12ResolvedBinding,
        output_path: &Path,
        vertex_offset: u32,
        vertex_count: u32,
    ) -> Result<(), String> {
        let input_path = self.dx12_resolve_binding_path(&binding.file);
        let bytes = fs::read(&input_path)
            .map_err(|e| format!("Failed to read DX12 VB {}: {}", input_path.display(), e))?;
        let offset = Self::dx12_effective_offset(bytes.len(), binding.offset, binding.bytes);
        let available = bytes.len().saturating_sub(offset);
        let read_len = if binding.bytes > 0 && binding.bytes <= available {
            binding.bytes
        } else {
            available
        };
        let end = offset.checked_add(read_len).ok_or_else(|| {
            format!(
                "DX12 VB slice overflow: offset={} len={} file={}",
                offset, read_len, binding.file
            )
        })?;
        let selected = &bytes[offset..end];
        if binding.stride == 0 || vertex_count == 0 {
            fs::write(output_path, selected)
                .map_err(|e| format!("Failed to write DX12 category buffer: {}", e))?;
            return Ok(());
        }

        let stride = binding.stride as usize;
        let start = (vertex_offset as usize)
            .checked_mul(stride)
            .ok_or_else(|| {
                format!(
                    "DX12 VB vertex slice start overflow: vertex_offset={} stride={} file={}",
                    vertex_offset, stride, binding.file
                )
            })?;
        let len = (vertex_count as usize).checked_mul(stride).ok_or_else(|| {
            format!(
                "DX12 VB vertex slice length overflow: vertex_count={} stride={} file={}",
                vertex_count, stride, binding.file
            )
        })?;
        let vertex_end = start.checked_add(len).ok_or_else(|| {
            format!(
                "DX12 VB vertex slice end overflow: start={} len={} file={}",
                start, len, binding.file
            )
        })?;
        if vertex_end > selected.len() {
            return Err(format!(
                "DX12 VB vertex slice [{}..{}) exceeds selected bytes {} for {}",
                start,
                vertex_end,
                selected.len(),
                binding.file
            ));
        }

        fs::write(output_path, &selected[start..vertex_end])
            .map_err(|e| format!("Failed to write DX12 category buffer: {}", e))
    }

    fn dx12_export_draw(
        &self,
        draw: &FrameAnalysisDX12DrawCall,
        d3d11_game_type: &D3D11GameType,
    ) -> Result<(), String> {
        let ib = draw
            .index_binding
            .as_ref()
            .ok_or_else(|| format!("DX12 draw {} missing IB", draw.call_index))?;
        if ib.file.trim().is_empty() {
            return Err(format!(
                "DX12 draw {} missing dumped IB file for gpu={} bytes={} fmt={}",
                draw.call_index, ib.gpu, ib.bytes, ib.fmt_name
            ));
        }
        let category_bindings = self
            .dx12_resolved_category_bindings(draw, d3d11_game_type)
            .ok_or_else(|| {
                format!(
                    "DX12 draw {} missing category binding for {}",
                    draw.call_index, d3d11_game_type.game_type_name
                )
            })?;

        let ib_path = self.dx12_resolve_binding_path(&ib.file);
        let ib_file_size = SSMTFileUtils::get_file_size(&ib_path)? as usize;
        let (ib_use_offset, ib_use_bytes) = if ib.file_offset > 0 || ib.file_bytes > 0 {
            (ib.file_offset, ib.file_bytes)
        } else {
            (ib.offset, ib.bytes)
        };
        let ib_offset = Self::dx12_effective_offset(ib_file_size, ib_use_offset, ib_use_bytes);
        let ib_read_len = if ib_use_bytes > 0 {
            ib_use_bytes
        } else if ib.fmt_name.eq_ignore_ascii_case("DXGI_FORMAT_R32_UINT") {
            draw.index_count.saturating_mul(4)
        } else {
            draw.index_count.saturating_mul(2)
        };
        let mut ib_buf_file = IndexBufferBufFile::from_file_byteoffset_read_length(
            &ib_path,
            &ib.fmt_name,
            ib_offset,
            ib_read_len,
        )?;
        if !ib.dump_is_draw_slice && !ib.dump_scope.eq_ignore_ascii_case("draw_slice") {
            ib_buf_file.self_divide(draw.start_index, draw.index_count);
        }
        if draw.base_vertex != 0 {
            let shifted = ib_buf_file
                .number_list
                .iter()
                .map(|n| {
                    let value = (*n as i64) + (draw.base_vertex as i64);
                    if value < 0 || value > u32::MAX as i64 {
                        Err(format!(
                            "DX12 draw {} index overflow after base_vertex {}",
                            draw.call_index, draw.base_vertex
                        ))
                    } else {
                        Ok(value as u32)
                    }
                })
                .collect::<Result<Vec<u32>, String>>()?;
            ib_buf_file.number_list = shifted;
            ib_buf_file.recalculate_cached_stats();
        }

        let vertex_offset = ib_buf_file.min_number;
        let vertex_count = if ib_buf_file.number_list.is_empty() {
            0
        } else {
            ib_buf_file
                .max_number
                .saturating_sub(ib_buf_file.min_number)
                .saturating_add(1)
        };

        let name_prefix = Self::dx12_submesh_name(draw);
        let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);
        let game_type_output_path = PathBuf::from(&self.workspace_path)
            .join(&name_prefix)
            .join(&game_type_folder_name);
        SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;

        crate::extract_log!(
            "[ZZMIDX12][Export] call_index={} pso={} game_type={} ib={} index_count={} start_index={} output={}",
            draw.call_index,
            draw.pso,
            d3d11_game_type.game_type_name,
            ib.file,
            draw.index_count,
            draw.start_index,
            game_type_output_path.display()
        );

        let output_ib_buf_file_path = game_type_output_path.join(format!("{}.ib", name_prefix));
        let ib_local_offset = i32::try_from(vertex_offset)
            .map_err(|_| format!("DX12 vertex offset {} exceeds i32 range", vertex_offset))?;
        ib_buf_file.save_to_file_uint32(&output_ib_buf_file_path, -ib_local_offset)?;

        for category_name in &d3d11_game_type.ordered_category_name_list {
            let Some(binding) = category_bindings.get(category_name) else {
                continue;
            };
            let category_output_buf_file_path =
                game_type_output_path.join(format!("{}-{}.buf", name_prefix, category_name));
            crate::extract_log!(
                "[ZZMIDX12][Export]   category={} source={} slot={} slot_file={} stride={} bytes={} -> {}",
                category_name,
                binding.source,
                binding.slot,
                binding.file,
                binding.stride,
                binding.bytes,
                category_output_buf_file_path.display()
            );
            self.dx12_copy_category_buffer(
                binding,
                &category_output_buf_file_path,
                vertex_offset,
                vertex_count,
            )?;
        }

        let mut submesh_json = SubMeshJson::new();
        submesh_json.game_preset = "ZZMIDX12".to_string();
        submesh_json.vertex_limit_vb = category_bindings
            .get("POSITION")
            .or_else(|| category_bindings.get("Position"))
            .map(|b| Self::dx12_vertex_limit_hash(Some(b)))
            .unwrap_or_default();
        if let Some(position_binding) = category_bindings
            .get("POSITION")
            .or_else(|| category_bindings.get("Position"))
        {
            submesh_json.match_cs = position_binding.match_cs.clone();
            submesh_json.match_uav_bytes = position_binding.match_uav_bytes;
        }
        submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
        submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
        submesh_json.vertex_offset = 0;
        submesh_json.vertex_count = vertex_count as i64;
        submesh_json.index_offset = draw.start_index as i64;
        submesh_json.index_count = draw.index_count as i64;
        submesh_json.index_buffer_list.push(SubMeshIndexBuffer {
            dxgi_format: "DXGI_FORMAT_R32_UINT".to_string(),
            file_name: format!("{}.ib", name_prefix),
        });

        for category_name in &d3d11_game_type.ordered_category_name_list {
            let binding = category_bindings.get(category_name);
            submesh_json
                .category_hash_dict
                .insert(category_name.clone(), Self::dx12_category_hash(binding));
            submesh_json.category_draw_category_map.insert(
                category_name.clone(),
                d3d11_game_type
                    .category_draw_category_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default(),
            );
            submesh_json
                .category_buffer_list
                .push(SubMeshCategoryBuffer {
                    file_name: format!("{}-{}.buf", name_prefix, category_name),
                    buffer_type: "Normal".to_string(),
                    d3d11_element_list: self
                        .build_submesh_elements_for_category(d3d11_game_type, category_name),
                });
        }

        submesh_json.save_to_file(game_type_output_path.join(name_prefix + ".json"))?;
        Ok(())
    }

    fn run_extract_dx12(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        let dx12_log = self
            .dx12_log
            .as_ref()
            .ok_or_else(|| "ZZMIDX12 extractor missing FrameAnalysisDX12Log".to_string())?;
        let draws = &dx12_log.draw_calls;
        crate::extract_log!("[ZZMIDX12][Extract] DX12 log: {}", dx12_log.log_filename);
        crate::extract_log!("[ZZMIDX12][Extract] Draw calls parsed: {}", draws.len());
        let mut exported = 0usize;
        let mut skipped = 0usize;
        let mut candidates = 0usize;
        let mut component_drawcall_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();

        for draw in draws.iter() {
            if draw.topology != "TRIANGLELIST"
                || draw.index_count < 12
                || draw.index_binding.is_none()
            {
                continue;
            }
            candidates += 1;
            crate::extract_log!(
                "[ZZMIDX12][Draw] call_index={} pso={} vs={} index_count={} start_index={} base_vertex={} vb_slots={:?}",
                draw.call_index,
                draw.pso,
                draw.vs,
                draw.index_count,
                draw.start_index,
                draw.base_vertex,
                draw.vertex_bindings.keys().collect::<Vec<&u32>>()
            );
            for binding in draw.vertex_bindings.values() {
                crate::extract_log!(
                    "[ZZMIDX12][Draw]   VB slot={} stride={} bytes={} offset={} skin={} file={}",
                    binding.slot,
                    binding.stride,
                    binding.bytes,
                    binding.offset,
                    binding.skin_source,
                    binding.file
                );
            }
            if let Some(ib) = draw.index_binding.as_ref() {
                crate::extract_log!(
                    "[ZZMIDX12][Draw]   IB fmt={} bytes={} offset={} file={}",
                    ib.fmt_name,
                    ib.bytes,
                    ib.offset,
                    ib.file
                );
                if ib.file.trim().is_empty() {
                    skipped += 1;
                    crate::extract_new::log_skipped_drawib(
                        &format!("DX12-{}", draw.call_index),
                        format!(
                            "missing dumped DX12 index buffer. gpu={} bytes={} fmt={}; latest dump recorded draw metadata but no readable IB file",
                            ib.gpu, ib.bytes, ib.fmt_name
                        ),
                    );
                    crate::extract_log!(
                        "[ZZMIDX12][Skip] call_index={} reason=missing dumped IB file",
                        draw.call_index
                    );
                    continue;
                }
            }

            let possible_gametypes = self.dx12_matching_gametypes(draw, data_type_filter)?;
            if possible_gametypes.is_empty() {
                skipped += 1;
                crate::extract_new::log_skipped_drawib(
                    &format!("DX12-{}", draw.call_index),
                    format!(
                        "no valid ZZMIDX12 data type matched. pso={} vs={} vb_slots={:?} skin_sources={:?}; {}",
                        draw.pso,
                        draw.vs,
                        draw.vertex_bindings.keys().collect::<Vec<&u32>>(),
                        draw.vertex_bindings
                            .values()
                            .map(|binding| binding.skin_source.as_str())
                            .collect::<Vec<&str>>(),
                        self.dx12_explain_unmatched_draw(draw)
                    ),
                );
                continue;
            }
            crate::extract_log!(
                "[ZZMIDX12][Match] call_index={} matched game types: {}",
                draw.call_index,
                possible_gametypes
                    .iter()
                    .map(|gt| gt.game_type_name.as_str())
                    .collect::<Vec<&str>>()
                    .join(", ")
            );

            for gt in possible_gametypes.iter() {
                match self.dx12_export_draw(draw, gt) {
                    Ok(()) => {
                        exported += 1;
                        let submesh_name = Self::dx12_submesh_name(draw);
                        let draw_call = draw.call_index.to_string();
                        let draw_calls = component_drawcall_index_list_dict
                            .entry(submesh_name)
                            .or_default();
                        if !draw_calls.contains(&draw_call) {
                            draw_calls.push(draw_call);
                        }
                    }
                    Err(err) => {
                        skipped += 1;
                        crate::extract_new::log_skipped_drawib(
                            &format!("DX12-{}", draw.call_index),
                            format!("failed to export game_type={}: {}", gt.game_type_name, err),
                        );
                        crate::extract_log!(
                            "[ZZMIDX12][Skip] call_index={} game_type={} reason={}",
                            draw.call_index,
                            gt.game_type_name,
                            err
                        );
                    }
                }
            }
        }

        crate::extract_log!(
            "[ZZMIDX12][Extract] Finished. candidates={} exported={} skipped={}",
            candidates,
            exported,
            skipped
        );
        self.sync_zzmidx12_textures_and_json(component_drawcall_index_list_dict)?;
        Ok(())
    }

    fn build_vertex_limit_vb(&self, trianglelist_index: &str) -> String {
        let vb0_file_name = self
            .fa
            .data
            .filter_first_file(&format!("{}-vb0", trianglelist_index), ".txt")
            .unwrap_or_default();
        if vb0_file_name.is_empty() {
            String::new()
        } else {
            vb0_file_name.chars().skip(11).take(8).collect()
        }
    }

    fn dx12_is_texture_binding(binding: &FrameAnalysisDX12ResourceBinding) -> bool {
        if !binding.kind.eq_ignore_ascii_case("SRV") {
            return false;
        }
        if !binding.dim.to_ascii_uppercase().starts_with("TEXTURE") {
            return false;
        }

        let lower = binding.file.to_ascii_lowercase();
        lower.ends_with(".dds") || lower.ends_with(".jpg") || lower.ends_with(".png")
    }

    fn dx12_texture_file_hash(file_name: &str) -> String {
        Self::dx12_texture_source_file_name(file_name)
            .split('-')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    fn dx12_texture_source_file_name(file_name: &str) -> String {
        Path::new(file_name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    }

    fn dx12_texture_virtual_file_name(binding: &FrameAnalysisDX12ResourceBinding) -> String {
        let file_hash = Self::dx12_texture_file_hash(&binding.file);
        let source_file_name = Self::dx12_texture_source_file_name(&binding.file);
        let ps_hash = if binding.ps.trim().is_empty() || binding.ps.trim() == "-" {
            "0000000000000000"
        } else {
            binding.ps.trim()
        };

        format!(
            "{}-ps-t{}={}-ps={}-{}",
            binding.call_index, binding.reg, file_hash, ps_hash, source_file_name
        )
    }

    fn sync_zzmidx12_textures_and_json(
        &self,
        component_drawcall_index_list_dict: HashMap<String, Vec<String>>,
    ) -> Result<(), String> {
        let Some(dx12_log) = self.dx12_log.as_ref() else {
            return Ok(());
        };

        if component_drawcall_index_list_dict.is_empty() {
            return Ok(());
        }

        let deduped_folder_path = PathBuf::from(&self.workspace_path).join("DedupedTextures");
        SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path)?;

        let mut texture_property_map: HashMap<String, TrianglelistDedupedTextureProperty> =
            HashMap::new();
        let mut copied_target_files: HashSet<String> = HashSet::new();

        for draw_call_list in component_drawcall_index_list_dict.values() {
            for draw_call in draw_call_list {
                let call_index = draw_call.trim().parse::<u64>().unwrap_or(0);
                if call_index == 0 {
                    continue;
                }

                for binding in dx12_log.resource_bindings_for_call(call_index) {
                    if !Self::dx12_is_texture_binding(binding) {
                        continue;
                    }

                    let source_texture_path = dx12_log.resolve_binding_path(&binding.file);
                    if !source_texture_path.exists() {
                        continue;
                    }

                    let file_hash = Self::dx12_texture_file_hash(&binding.file);
                    if file_hash.is_empty() {
                        continue;
                    }

                    let source_file_name = Self::dx12_texture_source_file_name(&binding.file);
                    if source_file_name.is_empty() {
                        continue;
                    }

                    let target_file_name = format!("{}_{}", file_hash, source_file_name);
                    if copied_target_files.insert(target_file_name.clone()) {
                        let target_texture_path = deduped_folder_path.join(&target_file_name);
                        SSMTFileUtils::copy_to_file_if_not_exists(
                            source_texture_path.to_string_lossy().as_ref(),
                            target_texture_path.to_string_lossy().as_ref(),
                        )?;
                    }

                    texture_property_map.insert(
                        Self::dx12_texture_virtual_file_name(binding),
                        TrianglelistDedupedTextureProperty {
                            fa_log_deduped_file_name: target_file_name.clone(),
                            fa_data_deduped_file_name: target_file_name.clone(),
                            format: SSMTStringUtils::extract_texture_format_from_deduped_file_name(
                                &target_file_name,
                            ),
                        },
                    );
                }
            }
        }

        let component_json_path =
            get_workspace_component_name_draw_call_index_list_json_path(&self.workspace_path)?;
        let component_json =
            ComponentNameDrawCallIndexListJson::from_map(component_drawcall_index_list_dict);
        component_json.save_to_file(&component_json_path)?;

        let trianglelist_json_path =
            get_workspace_trianglelist_deduped_filename_json_path(&self.workspace_path)?;
        let trianglelist_json = TrianglelistDedupedFileNameJson::from_map(texture_property_map);
        trianglelist_json.save_to_file(&trianglelist_json_path)?;

        Ok(())
    }

    fn build_category_hash_from_buf_file_name(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
        buf_file_name: &str,
    ) -> Result<String, String> {
        let category_slot = d3d11_game_type
            .category_slot_dict
            .get(category_name)
            .ok_or_else(|| format!("Category slot not found for category: {}", category_name))?;
        let start_index = 8usize + category_slot.len();
        let hash: String = buf_file_name.chars().skip(start_index).take(8).collect();
        if hash.len() != 8 {
            return Err(format!(
                "Cannot parse hash from buf file name: {} (category={}, slot={})",
                buf_file_name, category_name, category_slot
            ));
        }
        Ok(hash)
    }

    fn build_submesh_elements_for_category(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
    ) -> Vec<SubMeshD3D11Element> {
        let mut result = Vec::new();
        for element_name in &d3d11_game_type.ordered_full_element_list {
            let Some(element) = d3d11_game_type
                .element_name_d3d11_element_dict
                .get(element_name)
            else {
                continue;
            };
            if element.category == category_name {
                result.push(SubMeshD3D11Element::from_d3d11_element(element));
            }
        }
        result
    }

    fn export_category_buffer(
        &self,
        category_name: &str,
        category_buf_filename: &str,
        gpu_pre_skinning: bool,
        category_output_buf_file_path: &Path,
    ) -> Result<(), String> {
        let category_buf_file_path = self.fa.log.get_deduped_filepath(category_buf_filename);
        if category_buf_file_path.is_empty() {
            return Err(format!(
                "Category {} deduped path is empty: {}",
                category_name, category_buf_filename
            ));
        }

        if gpu_pre_skinning {
            fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
                format!(
                    "Failed to copy category buffer file for category {}: {}",
                    category_name, e
                )
            })?;
            return Ok(());
        }

        let category_txt_filename =
            SSMTFileUtils::get_filename_with_new_extension(category_buf_filename, "txt")?;
        let category_txt_file_path = self.fa.log.get_deduped_filepath(&category_txt_filename);
        if !category_txt_file_path.is_empty() && Path::new(&category_txt_file_path).exists() {
            let metadata = SSMTBinaryUtils::read_migoto_buffer_metadata(&category_txt_file_path)?;
            if metadata.stride > 0 && metadata.vertex_count > 0 {
                let category_buf_bytes = fs::read(&category_buf_file_path).map_err(|e| {
                    format!(
                        "Failed to read category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                let read_len = metadata
                    .vertex_count
                    .checked_mul(metadata.stride)
                    .ok_or_else(|| {
                        format!(
                            "Category {} slice length overflow: vertex_count={} stride={}",
                            category_name, metadata.vertex_count, metadata.stride
                        )
                    })?;
                let end = metadata.byte_offset.checked_add(read_len).ok_or_else(|| {
                    format!(
                        "Category {} slice end overflow: byte_offset={} read_len={}",
                        category_name, metadata.byte_offset, read_len
                    )
                })?;
                let sliced_bytes = SSMTBinaryUtils::get_range_bytes(
                    &category_buf_bytes,
                    metadata.byte_offset,
                    end,
                )
                .map_err(|e| {
                    format!(
                        "Failed to slice category buffer for category {}: {}",
                        category_name, e
                    )
                })?;
                fs::write(category_output_buf_file_path, sliced_bytes).map_err(|e| {
                    format!(
                        "Failed to write category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                return Ok(());
            }
        }

        fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
            format!(
                "Failed to copy category buffer file for category {}: {}",
                category_name, e
            )
        })?;
        Ok(())
    }

    fn collect_match_first_index_ib_map(
        &self,
        trianglelist_index_list: &[String],
    ) -> Result<BTreeMap<u64, String>, String> {
        let mut result: BTreeMap<u64, String> = BTreeMap::new();
        for trianglelist_index in trianglelist_index_list {
            let ib_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                .unwrap_or_default();
            if ib_file_name.is_empty() {
                continue;
            }
            let ib_file_path = self.fa.log.get_deduped_filepath(&ib_file_name);
            if ib_file_path.is_empty() {
                continue;
            }
            let ib_txt_file = IndexBufferTxtFile::new(&ib_file_path, false)?;
            let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            result.entry(match_first_index).or_insert(ib_file_name);
        }
        Ok(result)
    }

    fn export_unity_vs_submeshes(
        &self,
        game_preset: &str,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
        possible_d3d11_game_type_list: &[D3D11GameType],
    ) -> Result<bool, String> {
        if possible_d3d11_game_type_list.is_empty() {
            return Ok(false);
        }
        let match_first_index_ib_txt_file_name_dict =
            self.collect_match_first_index_ib_map(trianglelist_index_list)?;
        for (match_first_index, ib_file_name) in &match_first_index_ib_txt_file_name_dict {
            crate::extract_log!(
                "MatchFirstIndex: {} IBFileName: {}",
                match_first_index,
                ib_file_name
            );
        }
        for d3d11_game_type in possible_d3d11_game_type_list {
            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa.data,
                trianglelist_index_list,
                d3d11_game_type,
            );
            let vertex_limit_vb = self.build_vertex_limit_vb(&trianglelist_index);
            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);
            for ib_txt_file_name in match_first_index_ib_txt_file_name_dict.values() {
                let per_ib_trianglelist_index: String = ib_txt_file_name.chars().take(6).collect();
                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                let ib_txt_file_path = self.fa.log.get_deduped_filepath(ib_txt_file_name);
                let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);
                if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
                    continue;
                }
                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
                let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
                for category_name in &d3d11_game_type.ordered_category_name_list {
                    let topology = d3d11_game_type
                        .category_topology_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let mut extract_index = per_ib_trianglelist_index.clone();
                    if topology == "pointlist" && !pointlist_index.is_empty() {
                        extract_index = pointlist_index.to_string();
                    }
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let filename_search_str = format!("{}-{}", extract_index, category_slot);
                    let category_buf_filename = self
                        .fa
                        .data
                        .filter_first_file(&filename_search_str, ".buf")?;
                    category_buf_filename_map.insert(category_name.clone(), category_buf_filename);
                }
                let unique_str_folder_name = format!(
                    "{}-{}-{}",
                    draw_ib, ib_txt_file.index_number_count, ib_txt_file.first_index
                );
                let game_type_output_path = PathBuf::from(&self.workspace_path)
                    .join(&unique_str_folder_name)
                    .join(&game_type_folder_name);
                SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;
                let name_prefix = unique_str_folder_name.clone();
                let output_ib_buf_file_path =
                    game_type_output_path.join(format!("{}.ib", name_prefix));
                let mut ib_buf_file =
                    IndexBufferBufFile::from_file(&ib_buf_file_path, &ib_txt_file.format)?;
                ib_buf_file.self_divide(
                    ib_txt_file.first_index.parse::<usize>().unwrap_or(0),
                    ib_txt_file.index_number_count as usize,
                );
                ib_buf_file.save_to_file_uint32(&output_ib_buf_file_path, 0)?;
                for category_name in &d3d11_game_type.ordered_category_name_list {
                    let category_buf_filename = category_buf_filename_map
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_output_buf_file_path = game_type_output_path
                        .join(format!("{}-{}.buf", name_prefix, category_name));
                    self.export_category_buffer(
                        category_name,
                        &category_buf_filename,
                        d3d11_game_type.gpu_pre_skinning,
                        &category_output_buf_file_path,
                    )?;
                }
                let mut submesh_json = SubMeshJson::new();
                submesh_json.game_preset = game_preset.to_string();
                submesh_json.vertex_limit_vb = vertex_limit_vb.clone();
                submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
                submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
                submesh_json.index_buffer_list.push(SubMeshIndexBuffer {
                    dxgi_format: "DXGI_FORMAT_R32_UINT".to_string(),
                    file_name: format!("{}.ib", name_prefix),
                });
                for category_name in &d3d11_game_type.ordered_category_name_list {
                    let category_buf_filename = category_buf_filename_map
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_hash = self.build_category_hash_from_buf_file_name(
                        d3d11_game_type,
                        category_name,
                        &category_buf_filename,
                    )?;
                    submesh_json
                        .category_hash_dict
                        .insert(category_name.clone(), category_hash);
                    submesh_json.category_draw_category_map.insert(
                        category_name.clone(),
                        d3d11_game_type
                            .category_draw_category_dict
                            .get(category_name)
                            .cloned()
                            .unwrap_or_default(),
                    );
                    submesh_json
                        .category_buffer_list
                        .push(SubMeshCategoryBuffer {
                            file_name: format!("{}-{}.buf", name_prefix, category_name),
                            buffer_type: "Normal".to_string(),
                            d3d11_element_list: self.build_submesh_elements_for_category(
                                d3d11_game_type,
                                category_name,
                            ),
                        });
                }
                submesh_json
                    .save_to_file(game_type_output_path.join(name_prefix.to_string() + ".json"))?;
            }
        }
        Ok(true)
    }

    pub fn new(
        frame_analysis_folder: &String,
        workspace_path: &String,
        is_full_extract: bool,
    ) -> Result<Self, String> {
        if Path::new(frame_analysis_folder).join("log.jsonl").exists() {
            return Self::new_dx12(frame_analysis_folder, workspace_path, is_full_extract);
        }
        Self::new_internal(frame_analysis_folder, workspace_path, !is_full_extract)
    }

    pub fn new_dx12(
        frame_analysis_folder: &String,
        workspace_path: &String,
        is_full_extract: bool,
    ) -> Result<Self, String> {
        let frame_analysis_dir = PathBuf::from(frame_analysis_folder);
        if !frame_analysis_dir.exists() {
            return Err(format!(
                "FrameAnalysis folder not found: {}",
                frame_analysis_folder
            ));
        }

        crate::extract_log!(
            "[ZZMIDX12][Init] FrameAnalysis folder: {}",
            frame_analysis_folder
        );
        crate::extract_log!("[ZZMIDX12][Init] Workspace path: {}", workspace_path);
        crate::extract_log!("[ZZMIDX12][Init] Full extract: {}", is_full_extract);

        let dx12_log = FrameAnalysisDX12Log::new(frame_analysis_folder)?;
        let fa = FrameAnalysis {
            folder_path: frame_analysis_folder.clone(),
            data: FrameAnalysisData::new(frame_analysis_folder)?,
            log: FrameAnalysisSingleLog::empty(frame_analysis_folder),
        };
        let drawib_config = if !is_full_extract {
            DrawIBConfig::new_from_workspace(workspace_path)
                .map_err(|e| format!("Failed to read DrawIB config: {}", e))?
        } else {
            DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            }
        };

        let gametype_folder_path = PathManager::bundled_gametype_folder();
        let current_gametype_folder_path = gametype_folder_path.join("ZZMIDX12");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            dx12_log: Some(dx12_log),
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract: !is_full_extract,
            d3d11_gametype_lv2,
        })
    }

    fn new_internal(
        frame_analysis_folder: &String,
        workspace_path: &String,
        specify_drawib_extract: bool,
    ) -> Result<Self, String> {
        let frame_analysis_dir = PathBuf::from(frame_analysis_folder);
        if !frame_analysis_dir.exists() {
            return Err(format!(
                "FrameAnalysis 文件夹未找到: {}",
                frame_analysis_folder
            ));
        }

        let fa = FrameAnalysis::new(frame_analysis_folder)?;
        let drawib_config = if specify_drawib_extract {
            DrawIBConfig::new_from_workspace(workspace_path)
                .map_err(|e| format!("Failed to read DrawIB config: {}", e))?
        } else {
            DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            }
        };

        let gametype_folder_path = PathManager::bundled_gametype_folder();
        let current_gametype_folder_path = gametype_folder_path.join("ZZMIDX12");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            dx12_log: None,
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract,
            d3d11_gametype_lv2,
        })
    }

    pub fn get_possible_gametype_list_unity_vs(
        &self,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
    ) -> Result<Vec<D3D11GameType>, String> {
        let mut possible_gametype_list: Vec<D3D11GameType> = Vec::new();
        let mut find_at_least_one_gpu_type = false;

        for d3d11_game_type in self
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if !crate::extract_new::can_match_gametype(
                pointlist_index,
                d3d11_game_type.gpu_pre_skinning,
            ) {
                crate::extract_log!("未找到PointlistIndex，跳过GPU-PreSkinning数据类型");
                continue;
            }

            if find_at_least_one_gpu_type && !d3d11_game_type.gpu_pre_skinning {
                crate::extract_log!(
                    "自动优化:已经找到了满足条件的GPU类型，所以这个CPU类型就不用判断了"
                );
                continue;
            }

            crate::extract_log!("当前数据类型: {}", d3d11_game_type.game_type_name);

            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa.data,
                trianglelist_index_list,
                d3d11_game_type,
            );
            crate::extract_log!("TrianglelistIndex: {}", trianglelist_index);

            if trianglelist_index.is_empty() {
                crate::extract_log!(
                    "当前GameType无法找到符合槽位存在条件的TrianglelistIndex，跳过此项"
                );
                continue;
            }

            let mut category_buf_filesize_map: HashMap<String, u64> = HashMap::new();
            let mut all_file_exists = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let topology = d3d11_game_type
                    .category_topology_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                let mut extract_index = trianglelist_index.clone();
                if topology == "pointlist" && !pointlist_index.is_empty() {
                    extract_index = pointlist_index.to_string();
                }

                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();

                crate::extract_log!(
                    "当前分类: {} 提取Index: {} 提取槽位: {}",
                    category_name,
                    extract_index,
                    category_slot
                );

                let search_key = format!("{}-{}", extract_index, category_slot);
                let category_buf_file_name = self
                    .fa
                    .data
                    .filter_first_file(&search_key, ".buf")
                    .unwrap_or_default();
                let category_buf_txt_file_name = self
                    .fa
                    .data
                    .filter_first_file(&search_key, ".txt")
                    .unwrap_or_default();
                crate::extract_log!("CategoryBufFileName: {}", category_buf_file_name);

                let category_buf_file_path =
                    self.fa.log.get_deduped_filepath(&category_buf_file_name);
                crate::extract_log!(
                    "Category: {} File: {}",
                    category_name,
                    category_buf_file_path
                );

                if category_buf_file_path.is_empty() || !Path::new(&category_buf_file_path).exists()
                {
                    crate::extract_log!("对应Buffer文件未找到,此数据类型无效。");
                    all_file_exists = false;
                    break;
                }

                let category_buf_txt_file_path = self
                    .fa
                    .log
                    .get_deduped_filepath(&category_buf_txt_file_name);
                if !category_buf_txt_file_name.is_empty() && category_buf_txt_file_path.is_empty() {
                    return Err(format!(
                        "Category {} txt file path is empty: {}",
                        category_name, category_buf_txt_file_name
                    ));
                }
                let file_size = if category_buf_txt_file_name.is_empty()
                    || category_buf_txt_file_path.is_empty()
                {
                    SSMTFileUtils::get_file_size(&category_buf_file_path)?
                } else {
                    if d3d11_game_type.gpu_pre_skinning {
                        SSMTFileUtils::get_file_size(&category_buf_file_path)?
                    } else {
                        SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_buf_txt_file_path)?
                    }
                };
                category_buf_filesize_map.insert(category_name.clone(), file_size);
            }

            if !all_file_exists {
                crate::extract_log!("当前数据类型的部分槽位文件无法找到，跳过此数据类型识别。");
                continue;
            }

            let mut vertex_number: u64 = 0;
            let mut all_match = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let category_stride = d3d11_game_type
                    .category_stride_dict
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);
                let file_size = category_buf_filesize_map
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);

                let tmp_number = if category_stride > 0 {
                    file_size / category_stride
                } else {
                    0
                };

                if tmp_number == 0 {
                    crate::extract_log!("槽位的文件大小不能为0，槽位匹配失败，跳过此数据类型");
                    all_match = false;
                    break;
                }

                if !d3d11_game_type.gpu_pre_skinning {
                    let yu_shu = file_size % category_stride;
                    if yu_shu != 0 {
                        crate::extract_log!("余数不为0: {}，槽位匹配失败", yu_shu);
                        all_match = false;
                        break;
                    }
                }

                if vertex_number == 0 {
                    vertex_number = tmp_number;
                } else if vertex_number != tmp_number {
                    crate::extract_log!(
                        "VertexNumber: {} 当前槽位数量: {}",
                        vertex_number,
                        tmp_number
                    );
                    crate::extract_log!("槽位匹配失败");
                    all_match = false;
                    break;
                } else {
                    crate::extract_log!("{} Match!", category_name);

                    if d3d11_game_type.gpu_pre_skinning
                        && category_name == "Texcoord"
                        && !pointlist_index.is_empty()
                    {
                        let category_slot = d3d11_game_type
                            .category_slot_dict
                            .get(category_name)
                            .cloned()
                            .unwrap_or_default();
                        let txt_search_key = format!("{}-{}", pointlist_index, category_slot);
                        let category_txt_file_name = self
                            .fa
                            .data
                            .filter_first_file(&txt_search_key, ".txt")
                            .unwrap_or_default();

                        if category_txt_file_name.is_empty() {
                            crate::extract_log!("GPU Texcoord 校验失败：txt 文件不存在");
                            all_match = false;
                            break;
                        }

                        let category_txt_file_path =
                            self.fa.log.get_deduped_filepath(&category_txt_file_name);
                        if category_txt_file_path.is_empty()
                            || !Path::new(&category_txt_file_path).exists()
                        {
                            crate::extract_log!("GPU Texcoord 校验失败：txt 路径不存在");
                            all_match = false;
                            break;
                        }

                        let vb_txt_file = VertexBufferTxtFile::new(&category_txt_file_path)?;

                        let mut all_byte_width_match = true;
                        for (element_name, original_element) in
                            d3d11_game_type.element_name_d3d11_element_dict.iter()
                        {
                            if original_element.category != "Texcoord" {
                                continue;
                            }

                            if let Some(txt_element) = vb_txt_file
                                .element_name_d3d11_element_dict
                                .get(element_name)
                            {
                                if original_element.byte_width != txt_element.byte_width {
                                    all_byte_width_match = false;
                                    break;
                                }
                            }
                        }

                        if !all_byte_width_match {
                            let mut vbtxt_data_element_length: u64 = 0;
                            let mut vbtxt_element_number: u64 = 0;

                            for element_name in vb_txt_file.vertex_data_show_element_list.iter() {
                                if let Some(d3d11_element) = vb_txt_file
                                    .element_name_d3d11_element_dict
                                    .get(element_name)
                                {
                                    vbtxt_data_element_length += d3d11_element.byte_width_int();
                                    vbtxt_element_number += 1;
                                }
                            }

                            let mut game_type_element_length: u64 = 0;
                            let mut game_type_element_number: u64 = 0;

                            for original_element in
                                d3d11_game_type.element_name_d3d11_element_dict.values()
                            {
                                if original_element.category != "Texcoord" {
                                    continue;
                                }

                                game_type_element_length += original_element.byte_width_int();
                                game_type_element_number += 1;
                            }

                            if vbtxt_data_element_length != game_type_element_length
                                || vbtxt_element_number != game_type_element_number
                            {
                                crate::extract_log!(
                                    "GPU Texcoord 校验失败：元素ByteWidth和总长度校验均未通过"
                                );
                                all_match = false;
                                break;
                            }
                        }
                    }
                }
            }

            if !d3d11_game_type.gpu_pre_skinning && d3d11_game_type.category_slot_dict.len() == 1 {
                let first_category_name = d3d11_game_type
                    .ordered_category_name_list
                    .get(0)
                    .cloned()
                    .unwrap_or_default();
                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(&first_category_name)
                    .cloned()
                    .unwrap_or_default();

                let txt_search_key = format!("{}-{}", trianglelist_index, category_slot);
                let category_txt_file_name = self
                    .fa
                    .data
                    .filter_first_file(&txt_search_key, ".txt")
                    .unwrap_or_default();

                if category_txt_file_name.is_empty() {
                    crate::extract_log!("单分类CPU校验失败：txt 文件不存在");
                    all_match = false;
                } else {
                    let category_txt_file_path =
                        self.fa.log.get_deduped_filepath(&category_txt_file_name);
                    if category_txt_file_path.is_empty()
                        || !Path::new(&category_txt_file_path).exists()
                    {
                        all_match = false;
                    } else {
                        let show_stride = SSMTFileUtils::find_migoto_ini_attribute_in_file(
                            Path::new(&category_txt_file_path),
                            "stride",
                        )?;
                        if !show_stride.trim().is_empty() {
                            let show_stride_count = show_stride.parse::<u64>().unwrap_or(0);
                            let game_type_stride = d3d11_game_type.get_self_stride();
                            if show_stride_count != game_type_stride {
                                crate::extract_log!(
                                    "单分类CPU校验失败：txt stride 与 GameType stride 不一致"
                                );
                                all_match = false;
                            }
                        }
                    }
                }
            }

            if all_match {
                crate::extract_log!("MatchGameType: {}", d3d11_game_type.game_type_name);
                possible_gametype_list.push(d3d11_game_type.clone());
            }

            if !find_at_least_one_gpu_type {
                for gt in possible_gametype_list.iter() {
                    if gt.gpu_pre_skinning {
                        find_at_least_one_gpu_type = true;
                        break;
                    }
                }
            }
        }

        let all_cpu_type = !possible_gametype_list.iter().any(|gt| gt.gpu_pre_skinning);
        if all_cpu_type && !possible_gametype_list.is_empty() {
            let max_category_number = possible_gametype_list
                .iter()
                .map(|gt| gt.category_slot_dict.len())
                .max()
                .unwrap_or(0);

            possible_gametype_list = possible_gametype_list
                .into_iter()
                .filter(|gt| gt.category_slot_dict.len() == max_category_number)
                .collect();
        }

        if possible_gametype_list.is_empty() {
            crate::extract_log!("无法识别 DrawIB {} 对应的数据类型", draw_ib);
        } else {
            crate::extract_log!("All Matched GameType:");
            for gt in possible_gametype_list.iter() {
                crate::extract_log!("{}", gt.game_type_name);
            }
        }

        Ok(possible_gametype_list)
    }

    pub fn extract_fee307b98a965c16(
        &self,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<bool, String> {
        let mut possible_d3d11_game_type_list = self.get_possible_gametype_list_unity_vs(
            draw_ib,
            pointlist_index,
            trianglelist_index_list,
        )?;

        possible_d3d11_game_type_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));

        self.export_unity_vs_submeshes(
            "ZZMIDX12",
            draw_ib,
            pointlist_index,
            trianglelist_index_list,
            &possible_d3d11_game_type_list,
        )
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        if self.dx12_log.is_some() {
            return self.run_extract_dx12(data_type_filter);
        }

        let draw_ib_list = if self.specify_drawib_extract {
            self.drawib_config
                .entries
                .iter()
                .map(|entry| entry.draw_ib.clone())
                .collect::<Vec<String>>()
        } else {
            self.fa.data.get_all_drawib_list()
        };

        for draw_ib in draw_ib_list.iter() {
            crate::extract_log!("DrawIB: {}", draw_ib);

            let pointlist_index = self
                .fa
                .log
                .get_last_pointlist_index_by_hash(&draw_ib)
                .unwrap_or_default();
            crate::extract_log!("PointlistIndex: {:?}", pointlist_index);

            if pointlist_index.is_empty() {
                crate::extract_log!("未找到对应PointlistIndex，该DrawIB可能为CPU-PreSkinning类型");
            }

            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(&draw_ib);
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TriangleListIndex: {}", trianglelist_index);
            }

            let extract_success = self.extract_fee307b98a965c16(
                &draw_ib,
                &pointlist_index,
                &trianglelist_index_list,
                data_type_filter,
            )?;

            if !extract_success {
                crate::extract_new::log_skipped_drawib(
                    draw_ib,
                    format!(
                        "no valid data type matched. PointlistIndex: {} TrianglelistIndex: {:?}",
                        pointlist_index, trianglelist_index_list
                    ),
                );
                continue;
            }
        }

        self.sync_zzmi_textures_and_json(draw_ib_list)?;
        Ok(())
    }

    /// ZZMIDX12 专用贴图同步方法。
    ///
    /// 与通用 `sync_workspace_deduped_textures_and_json` 不同，
    /// 本方法使用与模型提取阶段完全相同的 IB 文件选取逻辑
    /// （`collect_match_first_index_ib_map` + BOTH txt/buf 存在检查），
    /// 确保生成的子网格文件夹名与模型提取一致。
    fn sync_zzmi_textures_and_json(&self, draw_ib_list: Vec<String>) -> Result<(), String> {
        use std::collections::BTreeMap;

        let drawib_config = DrawIBConfig {
            path: String::new(),
            entries: draw_ib_list
                .into_iter()
                .map(|draw_ib| DrawIBEntry {
                    draw_ib: draw_ib.clone(),
                    alias: draw_ib,
                })
                .collect(),
        };

        let mut component_drawcall_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();

        for entry in drawib_config.entries.iter() {
            let draw_ib = entry.draw_ib.trim();
            if draw_ib.is_empty() {
                continue;
            }

            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(draw_ib);
            if trianglelist_index_list.is_empty() {
                continue;
            }

            // 使用与模型提取完全相同的 IB 文件选取逻辑
            let match_first_index_ib_map =
                self.collect_match_first_index_ib_map(&trianglelist_index_list)?;

            let mut first_index_index_count_map: BTreeMap<u64, String> = BTreeMap::new();
            let mut first_index_trianglelist_index_map: BTreeMap<u64, Vec<String>> =
                BTreeMap::new();

            for (&first_index, ib_txt_file_name) in match_first_index_ib_map.iter() {
                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                let ib_txt_file_path = self.fa.log.get_deduped_filepath(ib_txt_file_name);
                let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);

                // 与 export_unity_vs_submeshes 一致：BOTH txt 和 buf 必须存在
                if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
                let submesh_index_count = ib_txt_file.index_number_count.to_string();

                first_index_index_count_map.insert(first_index, submesh_index_count);

                // 收集此 first_index 对应的所有 trianglelist_index
                for tl_idx in &trianglelist_index_list {
                    let search_key = format!("{}-ib", tl_idx);
                    let candidate_ib = self
                        .fa
                        .data
                        .filter_first_file(&search_key, ".txt")
                        .unwrap_or_default();
                    if candidate_ib.is_empty() {
                        continue;
                    }
                    let candidate_path = self.fa.log.get_deduped_filepath(&candidate_ib);
                    if candidate_path.is_empty() {
                        continue;
                    }
                    let candidate_ib_file = IndexBufferTxtFile::new(&candidate_path, false)?;
                    let candidate_first_index =
                        candidate_ib_file.first_index.parse::<u64>().unwrap_or(0);
                    if candidate_first_index == first_index {
                        let list = first_index_trianglelist_index_map
                            .entry(first_index)
                            .or_default();
                        if !list.contains(tl_idx) {
                            list.push(tl_idx.clone());
                        }
                    }
                }
            }

            if match_first_index_ib_map.is_empty() {
                component_drawcall_index_list_dict
                    .insert(draw_ib.to_string(), trianglelist_index_list.clone());
                continue;
            }

            for (first_index, submesh_trianglelist_index_list) in first_index_trianglelist_index_map
            {
                let index_count = first_index_index_count_map
                    .get(&first_index)
                    .cloned()
                    .unwrap_or_default();
                let submesh_folder_name = format!("{}-{}-{}", draw_ib, index_count, first_index);
                component_drawcall_index_list_dict
                    .insert(submesh_folder_name, submesh_trianglelist_index_list);
            }
        }

        if component_drawcall_index_list_dict.is_empty() {
            return Ok(());
        }

        let deduped_folder_path = PathBuf::from(&self.workspace_path).join("DedupedTextures");
        SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path)?;

        // 复制去重贴图
        for index_list in component_drawcall_index_list_dict.values() {
            for trianglelist_index in index_list.iter() {
                let content_str = format!("{}-ps-t", trianglelist_index);
                let ps_texture_all_filename_list =
                    self.fa.data.filter_texture_filename_list(&content_str);

                for ps_texture_filename in ps_texture_all_filename_list {
                    let deduped_filepath = self.fa.log.get_deduped_filepath(&ps_texture_filename);
                    if deduped_filepath.is_empty() {
                        continue;
                    }

                    let deduped_filename = self.fa.log.get_deduped_filename(&ps_texture_filename);
                    let texture_unique_hash =
                        SSMTStringUtils::get_file_hash_from_file_name(&ps_texture_filename);

                    let target_texture_path = deduped_folder_path
                        .join(format!("{}_{}", texture_unique_hash, deduped_filename));
                    let target_texture_path_str = target_texture_path.to_string_lossy().to_string();
                    let _ = SSMTFileUtils::copy_to_file_if_not_exists(
                        &deduped_filepath,
                        &target_texture_path_str,
                    );
                }
            }
        }

        // 写入 ComponentName_DrawCallIndexList.json
        let component_json_path =
            get_workspace_component_name_draw_call_index_list_json_path(&self.workspace_path)?;
        let component_json = ComponentNameDrawCallIndexListJson::from_map(
            component_drawcall_index_list_dict.clone(),
        );
        if let Err(e) = component_json.save_to_file(&component_json_path) {
            crate::extract_error!(
                "Failed to write {}: {}",
                component_json_path.to_string_lossy(),
                e
            );
        }

        // 构建 TrianglelistDedupedFileName.json
        let mut trianglelist_texture_file_name_list: Vec<String> = Vec::new();
        for index_list in component_drawcall_index_list_dict.values() {
            for trianglelist_index in index_list.iter() {
                let content_str = format!("{}-ps-t", trianglelist_index);
                let ps_texture_all_filename_list =
                    self.fa.data.filter_texture_filename_list(&content_str);
                trianglelist_texture_file_name_list.extend(ps_texture_all_filename_list);
            }
        }

        let mut trianglelist_deduped_map: HashMap<String, TrianglelistDedupedTextureProperty> =
            HashMap::new();

        for trianglelist_texture_file_name in trianglelist_texture_file_name_list {
            let hash =
                SSMTStringUtils::get_file_hash_from_file_name(&trianglelist_texture_file_name);

            let fa_log_deduped_filename = {
                let deduped = self
                    .fa
                    .log
                    .get_deduped_filename(&trianglelist_texture_file_name);
                if deduped.trim().is_empty() {
                    String::new()
                } else {
                    format!("{}_{}", hash, deduped)
                }
            };

            let fa_data_deduped_filename = {
                let mut out = String::new();
                let deduped_dir = Path::new(&self.fa.folder_path).join("deduped");
                if deduped_dir.exists() {
                    if let Ok(entries) = fs::read_dir(&deduped_dir) {
                        for entry in entries.flatten() {
                            let file_name = entry.file_name();
                            if let Some(file_name_str) = file_name.to_str() {
                                if file_name_str.contains(&hash) {
                                    out = format!("{}_{}", hash, file_name_str);
                                    break;
                                }
                            }
                        }
                    }
                }
                out
            };

            trianglelist_deduped_map.insert(
                trianglelist_texture_file_name,
                TrianglelistDedupedTextureProperty {
                    fa_log_deduped_file_name: fa_log_deduped_filename.clone(),
                    fa_data_deduped_file_name: fa_data_deduped_filename,
                    format: SSMTStringUtils::extract_texture_format_from_deduped_file_name(
                        &fa_log_deduped_filename,
                    ),
                },
            );
        }

        let trianglelist_json_path =
            get_workspace_trianglelist_deduped_filename_json_path(&self.workspace_path)?;
        let trianglelist_json = TrianglelistDedupedFileNameJson::from_map(trianglelist_deduped_map);
        if let Err(e) = trianglelist_json.save_to_file(&trianglelist_json_path) {
            crate::extract_error!(
                "Failed to write {}: {}",
                trianglelist_json_path.to_string_lossy(),
                e
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ZZMIDX12NewExtractor;
    use crate::extract_new::extract_services::FullExtractDataTypeFilter;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn extract_sample_to_workspace(sample: &Path, name: &str) -> PathBuf {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tmp")
            .join(name);
        if workspace.exists() {
            fs::remove_dir_all(&workspace).unwrap();
        }
        fs::create_dir_all(&workspace).unwrap();

        let sample_path = sample.to_string_lossy().to_string();
        let workspace_path = workspace.to_string_lossy().to_string();
        let mut extractor =
            ZZMIDX12NewExtractor::new_dx12(&sample_path, &workspace_path, true).unwrap();
        extractor
            .run_extract(FullExtractDataTypeFilter::All)
            .unwrap();

        workspace
    }

    #[test]
    fn extracts_dx12_schema3_sample_when_present() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-06-28-161958");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let workspace = extract_sample_to_workspace(sample, "zzmidx12-schema3-extract");

        fn count_json_files(path: &Path) -> usize {
            let Ok(entries) = fs::read_dir(path) else {
                return 0;
            };
            entries
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        count_json_files(&path)
                    } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                        1
                    } else {
                        0
                    }
                })
                .sum()
        }

        let exported_json_count = count_json_files(&workspace);

        assert!(exported_json_count > 0);
    }

    #[test]
    fn extracts_dx12_latest_schema3_full_view_sample_when_present() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-07-07-094622");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let workspace = extract_sample_to_workspace(sample, "zzmidx12-latest-full-view-extract");

        fn count_json_files(path: &Path) -> usize {
            let Ok(entries) = fs::read_dir(path) else {
                return 0;
            };
            entries
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        count_json_files(&path)
                    } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                        1
                    } else {
                        0
                    }
                })
                .sum()
        }

        let exported_json_count = count_json_files(&workspace);

        assert!(exported_json_count > 0);

        let deduped_texture_count = fs::read_dir(workspace.join("DedupedTextures"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().is_file())
            .count();
        assert!(deduped_texture_count > 0);

        let texture_json_path = workspace.join("TrianglelistDedupedFileName.json");
        let texture_json: Value =
            serde_json::from_str(&fs::read_to_string(texture_json_path).unwrap()).unwrap();
        let texture_map = texture_json.as_object().unwrap();
        assert!(texture_map.keys().any(|key| key.contains("-ps-t")));
    }

    #[test]
    fn extracts_dx12_latest_dump_20260708_when_present() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-07-08-123605");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let workspace = extract_sample_to_workspace(sample, "zzmidx12-latest-20260708-extract");

        fn count_json_files(path: &Path) -> usize {
            let Ok(entries) = fs::read_dir(path) else {
                return 0;
            };
            entries
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        count_json_files(&path)
                    } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                        1
                    } else {
                        0
                    }
                })
                .sum()
        }

        let exported_json_count = count_json_files(&workspace);
        assert!(exported_json_count > 0);

        let texture_json_path = workspace.join("TrianglelistDedupedFileName.json");
        let texture_json: Value =
            serde_json::from_str(&fs::read_to_string(texture_json_path).unwrap()).unwrap();
        let texture_map = texture_json.as_object().unwrap();
        assert!(texture_map.keys().any(|key| key.contains("-ps-t")));
    }

    #[test]
    fn does_not_divide_dx12_draw_slice_ib_when_sample_dump_exists() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-06-28-172703");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let workspace = extract_sample_to_workspace(sample, "zzmidx12-draw-slice-extract");
        let ib_path = workspace
            .join("f28d4cfa-31062-6630")
            .join("TYPE_GPU_P12_N12_TA16_C4_T4_T1-4_T2-4_T3-4_BW16_BI16_")
            .join("f28d4cfa-31062-6630.ib");
        let position_path = workspace
            .join("f28d4cfa-31062-6630")
            .join("TYPE_GPU_P12_N12_TA16_C4_T4_T1-4_T2-4_T3-4_BW16_BI16_")
            .join("f28d4cfa-31062-6630-Position.buf");
        let json_path = workspace
            .join("f28d4cfa-31062-6630")
            .join("TYPE_GPU_P12_N12_TA16_C4_T4_T1-4_T2-4_T3-4_BW16_BI16_")
            .join("f28d4cfa-31062-6630.json");

        let ib_bytes = fs::read(&ib_path).unwrap();
        assert_eq!(ib_bytes.len() / 4, 31062);
        let max_index = ib_bytes
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .max()
            .unwrap();
        let position_vertex_count = fs::metadata(&position_path).unwrap().len() / 40;
        assert_eq!(position_vertex_count, 8691);
        assert!(u64::from(max_index) < position_vertex_count);

        let json: Value = serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(json["VertexOffset"].as_i64().unwrap(), 2224);
        assert_eq!(json["VertexCount"].as_i64().unwrap(), 8691);
    }

    #[test]
    fn matches_dx12_stride76_cpu_ia_buffer_when_sample_dump_exists() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-06-28-180634");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let sample_path = sample.to_string_lossy().to_string();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tmp")
            .join("zzmidx12-stride76-match");
        let workspace_path = workspace.to_string_lossy().to_string();
        let extractor =
            ZZMIDX12NewExtractor::new_dx12(&sample_path, &workspace_path, true).unwrap();
        let log = extractor.dx12_log.as_ref().unwrap();
        let draw_609 = log
            .draw_calls
            .iter()
            .find(|draw| draw.call_index == 609)
            .expect("call 609 should be parsed from stride76 sample");

        assert_eq!(draw_609.vertex_bindings.get(&0).unwrap().stride, 76);

        let matched = extractor
            .dx12_matching_gametypes(draw_609, FullExtractDataTypeFilter::All)
            .unwrap();
        assert!(matched
            .iter()
            .any(|gt| gt.game_type_name == "CPU_P12_N12_TA16_T4_T1-8_T2-8_T3-8_T4-8_"));
    }

    #[test]
    fn matches_dx12_gpu_preskinning_by_resource_fallback_when_sample_dump_exists() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-06-30-165238");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let sample_path = sample.to_string_lossy().to_string();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tmp")
            .join("zzmidx12-resource-fallback-match");
        let workspace_path = workspace.to_string_lossy().to_string();
        let extractor =
            ZZMIDX12NewExtractor::new_dx12(&sample_path, &workspace_path, true).unwrap();
        let log = extractor.dx12_log.as_ref().unwrap();
        let draw_3497 = log
            .draw_calls
            .iter()
            .find(|draw| draw.call_index == 3497)
            .expect("call 3497 should be parsed from resource fallback sample");

        let source_sets = extractor.dx12_gpu_source_sets_for_draw(draw_3497);
        assert!(source_sets.iter().any(|source| {
            source.source_label == "dispatch-resource"
                && source.producer_call_index == 1697
                && source.match_cs == "93db774c5ca9a3ea"
                && source.candidates.iter().any(|binding| {
                    binding.kind == "SRV" && binding.reg == 0 && binding.stride == 40
                })
                && source.candidates.iter().any(|binding| {
                    binding.kind == "SRV" && binding.reg == 1 && binding.stride == 32
                })
        }));

        let matched = extractor
            .dx12_matching_gametypes(draw_3497, FullExtractDataTypeFilter::All)
            .unwrap();
        assert!(matched.iter().any(|gt| gt.gpu_pre_skinning));
    }

    #[test]
    fn explains_unmatched_body_draw_when_metadata_only_sample_exists() {
        let sample =
            Path::new("D:/SSMTCacheFolder/3Dmigoto/ZZMIDX12/FrameAnalysis-2026-07-01-205407");
        if !sample.join("log.jsonl").exists() {
            return;
        }

        let sample_path = sample.to_string_lossy().to_string();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tmp")
            .join("zzmidx12-metadata-body-diagnose");
        let workspace_path = workspace.to_string_lossy().to_string();
        let extractor =
            ZZMIDX12NewExtractor::new_dx12(&sample_path, &workspace_path, true).unwrap();
        let log = extractor.dx12_log.as_ref().unwrap();
        let draw_195 = log
            .draw_calls
            .iter()
            .find(|draw| draw.call_index == 195)
            .expect("call 195 should be parsed from draw-metadata sample");

        let matched = extractor
            .dx12_matching_gametypes(draw_195, FullExtractDataTypeFilter::All)
            .unwrap();
        assert!(matched.is_empty());

        let reason = extractor.dx12_explain_unmatched_draw(draw_195);
        assert!(reason.contains("missing dumped IB file"));
        assert!(reason.contains("no IA vertex bindings"));
        assert!(reason.contains("fb1678c0-buffer.buf"));
    }
}
