use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::shader_resource::ShaderResource;
use crate::common::shape_keys::{NativeMorphKey, ShapeKeys};
use half::f16;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

/// ZZMI morph 结构化记录：vertex_id(u32) + position/normal/tangent 三组 float3。
const MORPH_RECORD_STRIDE: usize = 40;
const MORPH_VERTEX_ATTRIBUTE_SCALAR_COUNT: usize = 9;
const SHAPE_KEY_OFFSET_COUNT: usize = 128;
const MAX_SHAPE_KEYS: usize = SHAPE_KEY_OFFSET_COUNT - 1;

struct ShapeKeyBuildOutput {
    offsets: Vec<u8>,
    vertex_ids: Vec<u8>,
    vertex_offsets: Vec<u8>,
    vertex_attributes: Vec<u8>,
    native_morph_keys: Vec<NativeMorphKey>,
}

#[derive(Clone, Debug)]
pub(crate) struct ZZMIMorphExtraction {
    pub base_position_buf: Vec<u8>,
    pub shape_key_offset_buf: Vec<u8>,
    pub shape_key_vertex_id_buf: Vec<u8>,
    pub shape_key_vertex_offset_buf: Vec<u8>,
    pub shape_key_vertex_attribute_offset_buf: Vec<u8>,
    pub shape_keys_info: ShapeKeys,
}

#[derive(Clone, Debug)]
struct CopyOperation {
    draw: u32,
    line_index: usize,
    source_hash: String,
    destination_hash: String,
}

#[derive(Clone, Debug)]
struct RawMorphDispatch {
    draw: u32,
    shader_hash: String,
    source_hash: String,
    constant_buffer_hash: String,
}

#[derive(Clone, Debug)]
struct MorphDispatch {
    shader_hash: String,
    source_hash: String,
    source_path: PathBuf,
    constant_buffer_hash: String,
    offset: u32,
    count: u32,
    weight: f32,
}

#[derive(Clone, Copy, Debug)]
struct PositionChannelOffsets {
    position: usize,
    normal: Option<usize>,
    tangent: Option<usize>,
}

#[derive(Clone, Copy)]
enum ComputeBinding {
    ShaderResource,
    UnorderedAccess,
    ConstantBuffer,
}

#[derive(Default)]
struct ComputeState {
    shader_resources: HashMap<u32, String>,
    unordered_access_views: HashMap<u32, String>,
    constant_buffers: HashMap<u32, String>,
}

/// 从预蒙皮 Position 的 CopyResource 终点向前还原 morph 链。
///
/// 这里不依赖 shader hash、角色 hash 或固定 draw 编号，只依赖：
/// seed CopyResource -> 同一 UAV 的连续 CS Dispatch -> Position CopyResource。
pub(crate) fn extract(
    frame_analysis: &FrameAnalysis,
    pointlist_index: &str,
    final_position_buf: &[u8],
    position_stride: usize,
    d3d11_game_type: &D3D11GameType,
) -> Result<Option<ZZMIMorphExtraction>, String> {
    let pointlist_draw = pointlist_index
        .parse::<u32>()
        .map_err(|error| format!("Pointlist draw index is invalid: {pointlist_index}: {error}"))?;
    let position_hash = frame_analysis
        .log
        .get_ia_vertex_buffer_hash_map(pointlist_index)
        .get("0")
        .cloned()
        .unwrap_or_default();
    if position_hash.is_empty() {
        return Ok(None);
    }

    let copy_operations = collect_copy_operations(&frame_analysis.log.lines);
    let Some(final_copy) = copy_operations.iter().rev().find(|operation| {
        operation.draw <= pointlist_draw && operation.destination_hash == position_hash
    }) else {
        return Ok(None);
    };
    let uav_hash = final_copy.source_hash.as_str();
    if uav_hash.is_empty() {
        return Ok(None);
    }
    let Some(seed_copy) = copy_operations.iter().rev().find(|operation| {
        operation.line_index < final_copy.line_index && operation.destination_hash == uav_hash
    }) else {
        return Ok(None);
    };

    let raw_dispatches = collect_morph_dispatches(
        &frame_analysis.log.lines,
        seed_copy.line_index,
        final_copy.line_index,
        uav_hash,
    );
    if raw_dispatches.is_empty() {
        return Ok(None);
    }

    let mut dispatches = Vec::new();
    let mut source_cache: HashMap<PathBuf, Vec<u8>> = HashMap::new();
    for raw_dispatch in raw_dispatches {
        let Some(constant_buffer_path) = find_binding_file(
            frame_analysis,
            raw_dispatch.draw,
            "cs-cb0",
            &raw_dispatch.constant_buffer_hash,
            ".buf",
        ) else {
            return Err(format!(
                "ZZMI morph dispatch {} is missing cs-cb0 {}",
                raw_dispatch.draw, raw_dispatch.constant_buffer_hash
            ));
        };

        let constant_buffer = fs::read(&constant_buffer_path).map_err(|error| {
            format!(
                "Failed to read ZZMI morph cb0 {}: {error}",
                constant_buffer_path.display()
            )
        })?;
        if constant_buffer.len() < 16 {
            return Err(format!(
                "ZZMI morph dispatch {} has a cb0 shorter than 16 bytes",
                raw_dispatch.draw
            ));
        }
        // cb0[0] 依次是源记录起点、记录数、本帧权重。
        let offset = read_u32(&constant_buffer, 0);
        let count = read_u32(&constant_buffer, 4);
        let weight = read_f32(&constant_buffer, 8);
        if count == 0 {
            continue;
        }
        if !weight.is_finite() {
            return Err(format!(
                "ZZMI morph dispatch {} has a non-finite weight",
                raw_dispatch.draw
            ));
        }

        let Some(source_path) = find_binding_file(
            frame_analysis,
            raw_dispatch.draw,
            "cs-t0",
            &raw_dispatch.source_hash,
            ".buf",
        ) else {
            return Err(format!(
                "ZZMI morph dispatch {} is missing cs-t0 {}",
                raw_dispatch.draw, raw_dispatch.source_hash
            ));
        };
        if !source_cache.contains_key(&source_path) {
            let source = fs::read(&source_path).map_err(|error| {
                format!(
                    "Failed to read ZZMI morph cs-t0 {}: {error}",
                    source_path.display()
                )
            })?;
            source_cache.insert(source_path.clone(), source);
        }
        let source = source_cache
            .get(&source_path)
            .ok_or_else(|| format!("Missing cached morph source: {}", source_path.display()))?;
        let source_record_count = source.len() / MORPH_RECORD_STRIDE;
        if source.is_empty() || !source.len().is_multiple_of(MORPH_RECORD_STRIDE) {
            return Err(format!(
                "ZZMI morph dispatch {} cs-t0 length {} is not aligned to {}",
                raw_dispatch.draw,
                source.len(),
                MORPH_RECORD_STRIDE,
            ));
        }
        let end_record = u64::from(offset) + u64::from(count);
        if end_record > source_record_count as u64 {
            return Err(format!(
                "ZZMI morph dispatch {} range exceeds cs-t0 records: offset={} count={} records={}",
                raw_dispatch.draw, offset, count, source_record_count
            ));
        }

        dispatches.push(MorphDispatch {
            shader_hash: raw_dispatch.shader_hash,
            source_hash: raw_dispatch.source_hash,
            source_path,
            constant_buffer_hash: raw_dispatch.constant_buffer_hash,
            offset,
            count,
            weight,
        });
    }

    if dispatches.is_empty() {
        return Ok(None);
    }
    if position_stride == 0
        || final_position_buf.is_empty()
        || !final_position_buf.len().is_multiple_of(position_stride)
    {
        crate::extract_log!(
            "ZZMI morph 跳过：Position stride={} 无法整除 buffer_len={}",
            position_stride,
            final_position_buf.len()
        );
        return Ok(None);
    }
    let channel_offsets = resolve_position_channel_offsets(d3d11_game_type, position_stride)?;

    let vertex_count = final_position_buf.len() / position_stride;
    for dispatch in &dispatches {
        let source = source_cache.get(&dispatch.source_path).ok_or_else(|| {
            format!(
                "Missing cached morph source: {}",
                dispatch.source_path.display()
            )
        })?;
        for record_index in dispatch.offset..dispatch.offset + dispatch.count {
            let record_offset = record_index as usize * MORPH_RECORD_STRIDE;
            let vertex_id = read_u32(source, record_offset) as usize;
            if vertex_id >= vertex_count {
                return Err(format!(
                    "ZZMI morph record vertex id {} exceeds Position vertex count {}",
                    vertex_id, vertex_count
                ));
            }
        }
    }

    let mut base_position_buf = final_position_buf.to_vec();
    for dispatch in dispatches.iter().rev() {
        let source = source_cache.get(&dispatch.source_path).ok_or_else(|| {
            format!(
                "Missing cached morph source: {}",
                dispatch.source_path.display()
            )
        })?;
        for record_index in (dispatch.offset..dispatch.offset + dispatch.count).rev() {
            subtract_morph_record(
                &mut base_position_buf,
                position_stride,
                source,
                record_index as usize,
                dispatch.weight,
                channel_offsets,
            );
        }
    }

    // Only the actual dispatches of this frame participate in undoing its pose.
    // Inferred ranges extend the editable key table after the neutral basis is restored.
    append_inactive_morph_dispatches(&mut dispatches, &source_cache, vertex_count);
    let shape_key_buffers = build_shape_key_buffers(&dispatches, &source_cache, vertex_count)?;
    if shape_key_buffers.vertex_ids.is_empty() {
        return Ok(None);
    }

    let offset_values: Vec<u32> = shape_key_buffers
        .offsets
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect();
    let shape_key_vertex_count = offset_values[SHAPE_KEY_OFFSET_COUNT - 1] as i32;
    let shape_keys_info = ShapeKeys {
        offsets_hash: stable_buffer_hash(&shape_key_buffers.offsets),
        scale_hash: String::new(),
        vertex_ids_hash: stable_buffer_hash(&shape_key_buffers.vertex_ids),
        vertex_offsets_hash: stable_buffer_hash(&shape_key_buffers.vertex_offsets),
        vertex_attribute_offsets_hash: stable_buffer_hash(&shape_key_buffers.vertex_attributes),
        vertex_attribute_channel_count: MORPH_VERTEX_ATTRIBUTE_SCALAR_COUNT as u32,
        vertex_count: shape_key_vertex_count,
        dispatch_y: (shape_key_vertex_count + 31) / 32,
        checksum: offset_values
            .iter()
            .take(4)
            .fold(0u32, |sum, value| sum.wrapping_add(*value)) as i32,
        native_morph_keys: shape_key_buffers.native_morph_keys,
    };

    Ok(Some(ZZMIMorphExtraction {
        base_position_buf,
        shape_key_offset_buf: shape_key_buffers.offsets,
        shape_key_vertex_id_buf: shape_key_buffers.vertex_ids,
        shape_key_vertex_offset_buf: shape_key_buffers.vertex_offsets,
        shape_key_vertex_attribute_offset_buf: shape_key_buffers.vertex_attributes,
        shape_keys_info,
    }))
}

fn collect_copy_operations(lines: &[String]) -> Vec<CopyOperation> {
    let mut operations = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let Some(draw) = draw_number(line) else {
            continue;
        };
        if !line.contains("CopyResource(") {
            continue;
        }

        let mut source_hash = String::new();
        let mut destination_hash = String::new();
        for detail in lines.iter().skip(line_index + 1).take(4) {
            let trimmed = detail.trim();
            if trimmed.starts_with("Src:") {
                source_hash = hash_from_line(trimmed).unwrap_or_default().to_string();
            } else if trimmed.starts_with("Dst:") {
                destination_hash = hash_from_line(trimmed).unwrap_or_default().to_string();
            }
            if !source_hash.is_empty() && !destination_hash.is_empty() {
                break;
            }
        }
        if !source_hash.is_empty() && !destination_hash.is_empty() {
            operations.push(CopyOperation {
                draw,
                line_index,
                source_hash,
                destination_hash,
            });
        }
    }
    operations
}

fn collect_morph_dispatches(
    lines: &[String],
    start_line: usize,
    end_line: usize,
    target_uav_hash: &str,
) -> Vec<RawMorphDispatch> {
    let mut state = ComputeState::default();
    let mut pending_binding = None;
    let mut current_shader_hash = String::new();
    let mut dispatches = Vec::new();

    for line in lines.iter().take(end_line + 1).skip(start_line) {
        if let Some(draw) = draw_number(line) {
            let payload = draw_payload(line);
            if payload.starts_with("CSSetShaderResources(") {
                clear_binding_slots(&mut state.shader_resources, payload, "NumViews:");
                pending_binding = Some(ComputeBinding::ShaderResource);
            } else if payload.starts_with("CSSetUnorderedAccessViews(") {
                clear_binding_slots(&mut state.unordered_access_views, payload, "NumUAVs:");
                pending_binding = Some(ComputeBinding::UnorderedAccess);
            } else if payload.starts_with("CSSetConstantBuffers") {
                clear_binding_slots(&mut state.constant_buffers, payload, "NumBuffers:");
                pending_binding = Some(ComputeBinding::ConstantBuffer);
            } else if payload.starts_with("CSSetShader(") {
                current_shader_hash = hash_from_line(payload).unwrap_or_default().to_string();
                pending_binding = None;
            } else if payload.starts_with("Dispatch(") {
                let source_hash = state.shader_resources.get(&0).cloned().unwrap_or_default();
                let constant_buffer_hash =
                    state.constant_buffers.get(&0).cloned().unwrap_or_default();
                let writes_target_uav = state
                    .unordered_access_views
                    .values()
                    .any(|hash| hash == target_uav_hash);
                if writes_target_uav
                    && !source_hash.is_empty()
                    && !constant_buffer_hash.is_empty()
                    && !current_shader_hash.is_empty()
                {
                    dispatches.push(RawMorphDispatch {
                        draw,
                        shader_hash: current_shader_hash.clone(),
                        source_hash,
                        constant_buffer_hash,
                    });
                }
                pending_binding = None;
            } else {
                pending_binding = None;
            }
            continue;
        }

        let Some(binding) = pending_binding else {
            continue;
        };
        let resource = ShaderResource::new(line.trim());
        let Ok(slot) = resource.index.parse::<u32>() else {
            continue;
        };
        if resource.hash.is_empty() {
            continue;
        }
        match binding {
            ComputeBinding::ShaderResource => {
                state.shader_resources.insert(slot, resource.hash.clone());
            }
            ComputeBinding::UnorderedAccess => {
                state
                    .unordered_access_views
                    .insert(slot, resource.hash.clone());
            }
            ComputeBinding::ConstantBuffer => {
                state.constant_buffers.insert(slot, resource.hash.clone());
            }
        }
    }
    dispatches
}

fn clear_binding_slots(bindings: &mut HashMap<u32, String>, payload: &str, count_key: &str) {
    let Some(start_slot) = number_after(payload, "StartSlot:") else {
        return;
    };
    let Some(count) = number_after(payload, count_key) else {
        return;
    };
    for slot in start_slot..start_slot.saturating_add(count) {
        bindings.remove(&slot);
    }
}

fn find_binding_file(
    frame_analysis: &FrameAnalysis,
    draw: u32,
    binding: &str,
    hash: &str,
    suffix: &str,
) -> Option<PathBuf> {
    let exact = format!("{draw:06}-{binding}={hash}");
    let dynamic = format!("{draw:06}-{binding}-");
    let file_name = frame_analysis
        .data
        .files
        .iter()
        .find(|file_name| file_name.contains(&exact) && file_name.ends_with(suffix))
        .or_else(|| {
            frame_analysis
                .data
                .files
                .iter()
                .find(|file_name| file_name.contains(&dynamic) && file_name.ends_with(suffix))
        })?;
    frame_analysis.log.resolve_dump_filepath(file_name).ok()
}

fn subtract_morph_record(
    position_buf: &mut [u8],
    position_stride: usize,
    source: &[u8],
    record_index: usize,
    weight: f32,
    channel_offsets: PositionChannelOffsets,
) {
    if weight == 0.0 {
        return;
    }
    let record_offset = record_index * MORPH_RECORD_STRIDE;
    let vertex_id = read_u32(source, record_offset) as usize;
    let vertex_offset = vertex_id * position_stride;

    for channel in 0..3 {
        let position_offset = vertex_offset + channel_offsets.position + channel * 4;
        let delta = read_f32(source, record_offset + 4 + channel * 4);
        write_f32(
            position_buf,
            position_offset,
            read_f32(position_buf, position_offset) - delta * weight,
        );
    }
    for channel in 0..3 {
        let Some(base_offset) = channel_offsets.normal else {
            break;
        };
        let normal_offset = vertex_offset + base_offset + channel * 4;
        let delta = read_f32(source, record_offset + 16 + channel * 4);
        write_f32(
            position_buf,
            normal_offset,
            read_f32(position_buf, normal_offset) - delta * weight,
        );
    }
    for channel in 0..3 {
        let Some(base_offset) = channel_offsets.tangent else {
            break;
        };
        let tangent_offset = vertex_offset + base_offset + channel * 4;
        let delta = read_f32(source, record_offset + 28 + channel * 4);
        write_f32(
            position_buf,
            tangent_offset,
            read_f32(position_buf, tangent_offset) - delta * weight,
        );
    }
}

fn resolve_position_channel_offsets(
    d3d11_game_type: &D3D11GameType,
    position_stride: usize,
) -> Result<PositionChannelOffsets, String> {
    let mut byte_offset = 0usize;
    let mut position = None;
    let mut normal = None;
    let mut tangent = None;
    for element in &d3d11_game_type.d3d11_element_list {
        if !element.category.eq_ignore_ascii_case("Position") {
            continue;
        }
        let semantic = element.semantic_name.to_ascii_uppercase();
        let byte_width = usize::try_from(element.byte_width_int()).unwrap_or(0);
        if byte_width >= 12 {
            match semantic.as_str() {
                "POSITION" => position = Some(byte_offset),
                "NORMAL" => normal = Some(byte_offset),
                "TANGENT" => tangent = Some(byte_offset),
                _ => {}
            }
        }
        byte_offset = byte_offset.saturating_add(byte_width);
    }

    let position = position.ok_or_else(|| {
        "ZZMI morph Position channel is missing or not float3-compatible".to_string()
    })?;
    for (label, offset) in [
        ("Position", Some(position)),
        ("Normal", normal),
        ("Tangent", tangent),
    ] {
        if let Some(offset) = offset {
            if offset + 12 > position_stride {
                return Err(format!(
                    "ZZMI morph {label} channel exceeds Position stride: offset={offset}, stride={position_stride}"
                ));
            }
        }
    }
    Ok(PositionChannelOffsets {
        position,
        normal,
        tangent,
    })
}

fn infer_morph_ranges(
    source: &[u8],
    observed: &[(u32, u32)],
    vertex_count: usize,
) -> Vec<(u32, u32)> {
    let observed: HashSet<_> = observed.iter().copied().collect();
    if observed.len() < 2 || !source.len().is_multiple_of(MORPH_RECORD_STRIDE) {
        return Vec::new();
    }
    let Ok(record_count) = u32::try_from(source.len() / MORPH_RECORD_STRIDE) else {
        return Vec::new();
    };
    let mut anchors: Vec<_> = observed.iter().copied().collect();
    anchors.sort_unstable();
    let mut confirmed_edges = HashSet::new();
    let mut last_end = 0;
    for (offset, count) in anchors {
        let Some(end) = offset.checked_add(count) else {
            return Vec::new();
        };
        if count == 0 || offset < last_end || end > record_count {
            return Vec::new();
        }
        confirmed_edges.extend([offset, end]);
        last_end = end;
    }

    // A real dispatch can start above the preceding vertex ID. Confirmed
    // offsets and endpoints still delimit it; ID resets only reveal the
    // remaining boundaries for this ascending source layout.
    let mut start = 0;
    let mut previous = None;
    let mut ranges = Vec::new();
    for (index, record) in (0..record_count).zip(source.chunks_exact(MORPH_RECORD_STRIDE)) {
        let vertex = read_u32(record, 0);
        if vertex as usize >= vertex_count {
            return Vec::new();
        }
        if index != start
            && (confirmed_edges.contains(&index) || previous.is_some_and(|old| vertex <= old))
        {
            ranges.push((start, index - start));
            start = index;
        }
        previous = Some(vertex);
    }
    if start < record_count {
        ranges.push((start, record_count - start));
    }
    let boundary_set: HashSet<_> = ranges.iter().copied().collect();
    if !observed.is_subset(&boundary_set) {
        return Vec::new();
    }

    ranges
        .into_iter()
        .filter(|&(offset, count)| {
            if observed.contains(&(offset, count)) {
                return false;
            }
            let first = offset as usize * MORPH_RECORD_STRIDE;
            let end = (offset + count) as usize * MORPH_RECORD_STRIDE;
            source[first..end]
                .chunks_exact(MORPH_RECORD_STRIDE)
                .all(|row| {
                    (0..MORPH_VERTEX_ATTRIBUTE_SCALAR_COUNT).all(|channel| {
                        let value = read_f32(row, 4 + channel * 4);
                        value.is_finite() && (channel >= 3 || f16::from_f32(value).is_finite())
                    })
                })
        })
        .collect()
}

fn append_inactive_morph_dispatches(
    dispatches: &mut Vec<MorphDispatch>,
    source_cache: &HashMap<PathBuf, Vec<u8>>,
    vertex_count: usize,
) {
    let mut by_source = BTreeMap::<&str, Vec<&MorphDispatch>>::new();
    for dispatch in dispatches.iter() {
        by_source
            .entry(&dispatch.source_hash)
            .or_default()
            .push(dispatch);
    }
    let mut added = Vec::new();
    for group in by_source.into_values() {
        let template = group[0];
        let Some(source) = source_cache.get(&template.source_path) else {
            continue;
        };
        if !group.iter().all(|dispatch| {
            dispatch.shader_hash == template.shader_hash
                && dispatch.constant_buffer_hash == template.constant_buffer_hash
                && source_cache.get(&dispatch.source_path) == Some(source)
        }) {
            continue;
        }
        let observed: Vec<_> = group
            .iter()
            .map(|dispatch| (dispatch.offset, dispatch.count))
            .collect();
        for (offset, count) in infer_morph_ranges(source, &observed, vertex_count) {
            added.push(MorphDispatch {
                offset,
                count,
                weight: 0.0,
                ..template.clone()
            });
        }
    }
    if !added.is_empty() {
        crate::extract_log!("ZZMI morph: 从单帧源缓冲补出 {} 个候选形态键", added.len());
        dispatches.extend(added);
    }
}

fn build_shape_key_buffers(
    dispatches: &[MorphDispatch],
    source_cache: &HashMap<PathBuf, Vec<u8>>,
    vertex_count: usize,
) -> Result<ShapeKeyBuildOutput, String> {
    let mut seen_ranges = HashSet::new();
    let mut offsets = vec![0u32; SHAPE_KEY_OFFSET_COUNT];
    let mut vertex_ids = Vec::new();
    let mut vertex_offsets = Vec::new();
    let mut vertex_attributes = Vec::new();
    let mut native_morph_keys = Vec::new();
    let mut shape_key_count = 0usize;

    for dispatch in dispatches {
        let range_key = (
            dispatch.source_hash.clone(),
            dispatch.offset,
            dispatch.count,
        );
        if !seen_ranges.insert(range_key) {
            continue;
        }
        if shape_key_count >= MAX_SHAPE_KEYS {
            crate::extract_log!(
                "ZZMI morph 检测到超过 {} 个形态键，后续键已截断",
                MAX_SHAPE_KEYS
            );
            break;
        }

        let source = source_cache.get(&dispatch.source_path).ok_or_else(|| {
            format!(
                "Missing cached morph source: {}",
                dispatch.source_path.display()
            )
        })?;
        for record_index in dispatch.offset..dispatch.offset + dispatch.count {
            let record_offset = record_index as usize * MORPH_RECORD_STRIDE;
            let vertex_id = read_u32(source, record_offset);
            if vertex_id as usize >= vertex_count {
                return Err(format!(
                    "ZZMI morph record vertex id {} exceeds Position vertex count {}",
                    vertex_id, vertex_count
                ));
            }
            vertex_ids.extend_from_slice(&vertex_id.to_le_bytes());
            for channel in 0..3 {
                let delta = read_f32(source, record_offset + 4 + channel * 4);
                if !delta.is_finite() {
                    return Err(format!(
                        "ZZMI morph delta at record {} is not finite",
                        record_index
                    ));
                }
                let encoded = f16::from_f32(delta);
                if encoded.is_infinite() {
                    return Err(format!(
                        "ZZMI morph delta {} cannot be represented as float16",
                        delta
                    ));
                }
                vertex_offsets.extend_from_slice(&encoded.to_bits().to_le_bytes());
            }
            for scalar_index in 0..MORPH_VERTEX_ATTRIBUTE_SCALAR_COUNT {
                let delta = read_f32(source, record_offset + 4 + scalar_index * 4);
                if !delta.is_finite() {
                    return Err(format!(
                        "ZZMI morph attribute delta at record {} is not finite",
                        record_index
                    ));
                }
                vertex_attributes.extend_from_slice(&delta.to_le_bytes());
            }
        }
        shape_key_count += 1;
        offsets[shape_key_count] = (vertex_ids.len() / 4) as u32;
        native_morph_keys.push(NativeMorphKey {
            shader_hash: dispatch.shader_hash.clone(),
            source_hash: dispatch.source_hash.clone(),
            constant_buffer_hash: dispatch.constant_buffer_hash.clone(),
            offset: dispatch.offset,
            count: dispatch.count,
            shape_key_index: (shape_key_count - 1) as u32,
        });
    }

    let total_entries = (vertex_ids.len() / 4) as u32;
    for offset in offsets
        .iter_mut()
        .take(SHAPE_KEY_OFFSET_COUNT)
        .skip(shape_key_count + 1)
    {
        *offset = total_entries;
    }

    let offset_buf = offsets
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<u8>>();
    Ok(ShapeKeyBuildOutput {
        offsets: offset_buf,
        vertex_ids,
        vertex_offsets,
        vertex_attributes,
        native_morph_keys,
    })
}

fn stable_buffer_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn draw_number(line: &str) -> Option<u32> {
    let prefix = line.get(..6)?;
    if line.as_bytes().get(6) != Some(&b' ') || !prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    prefix.parse().ok()
}

fn draw_payload(line: &str) -> &str {
    line.get(7..).unwrap_or_default()
}

fn hash_from_line(line: &str) -> Option<&str> {
    let value = line.split_once("hash=")?.1;
    let end = value
        .find(|character: char| !character.is_ascii_hexdigit())
        .unwrap_or(value.len());
    (end > 0).then_some(&value[..end])
}

fn number_after(text: &str, prefix: &str) -> Option<u32> {
    let value = text.split_once(prefix)?.1;
    let end = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    if end == 0 {
        None
    } else {
        value[..end].parse().ok()
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("u32 byte range"),
    )
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("f32 byte range"),
    )
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn restores_inactive_sorted_morph_ranges_without_changing_existing_range_contracts() {
        let ids = [0, 2, 1, 3, 0, 1, 4];
        let source: Vec<u8> = ids
            .into_iter()
            .flat_map(|id: u32| {
                let mut row = [0; MORPH_RECORD_STRIDE];
                row[..4].copy_from_slice(&id.to_le_bytes());
                row
            })
            .collect();
        assert_eq!(
            infer_morph_ranges(&source, &[(0, 2), (4, 3)], 5),
            vec![(2, 2)],
        );
        assert_eq!(
            infer_morph_ranges(&source, &[(0, 2), (2, 1)], 5),
            vec![(3, 1), (4, 3)],
        );
        assert!(infer_morph_ranges(&source, &[(0, 3), (4, 3)], 5).is_empty());
        assert!(infer_morph_ranges(&source, &[(0, 2), (1, 1)], 5).is_empty());
        assert!(infer_morph_ranges(&source, &[(0, 2)], 5).is_empty());
    }

    fn create_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ssmt4-zzmi-morph-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn write_position_vertex(
        bytes: &mut [u8],
        vertex_index: usize,
        stride: usize,
        position: [f32; 3],
        normal: [f32; 3],
        tangent: [f32; 4],
    ) {
        let offset = vertex_index * stride;
        for (channel, value) in position.into_iter().enumerate() {
            bytes[offset + channel * 4..offset + channel * 4 + 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        for (channel, value) in normal.into_iter().enumerate() {
            let start = offset + 12 + channel * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (channel, value) in tangent.into_iter().enumerate() {
            let start = offset + 24 + channel * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    fn write_morph_record(
        bytes: &mut Vec<u8>,
        vertex_id: u32,
        position_delta: [f32; 3],
        normal_delta: [f32; 3],
        tangent_delta: [f32; 3],
    ) {
        bytes.extend_from_slice(&vertex_id.to_le_bytes());
        for value in position_delta
            .into_iter()
            .chain(normal_delta)
            .chain(tangent_delta)
        {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn write_cb0(path: &Path, offset: u32, count: u32, weight: f32) {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&weight.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        fs::write(path, bytes).expect("failed to write cb0");
    }

    fn read_f32(bytes: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("f32 bytes"))
    }

    fn f16_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x03ff) as f32;
        match exponent {
            0 => sign * 2f32.powi(-14) * (mantissa / 1024.0),
            0x1f if mantissa == 0.0 => sign * f32::INFINITY,
            0x1f => f32::NAN,
            _ => sign * 2f32.powi(exponent - 15) * (1.0 + mantissa / 1024.0),
        }
    }

    fn test_game_type() -> D3D11GameType {
        use crate::common::d3d11_element::D3D11Element;
        D3D11GameType::from_parts(
            "ZZMI_TEST",
            vec![
                D3D11Element::new(
                    "POSITION",
                    "R32G32B32_FLOAT",
                    "vb0",
                    "pointlist",
                    "Position",
                    "Position",
                    "12",
                ),
                D3D11Element::new(
                    "NORMAL",
                    "R32G32B32_FLOAT",
                    "vb0",
                    "pointlist",
                    "Position",
                    "Position",
                    "12",
                ),
                D3D11Element::new(
                    "TANGENT",
                    "R32G32B32A32_FLOAT",
                    "vb0",
                    "pointlist",
                    "Position",
                    "Position",
                    "16",
                ),
            ],
        )
    }

    #[test]
    fn reconstructs_morph_from_resource_relationships_without_known_hashes() {
        let dir = create_temp_dir("generic-chain");
        let final_hash = "bbbbbbbb";
        let uav_hash = "aaaaaaaa";
        let source_hash = "cccccccc";
        let cb0_hash = "dddddddd";
        let shader_hash = "eeeeeeeeeeeeeeee";

        let mut records = Vec::new();
        write_morph_record(
            &mut records,
            1,
            [2.0, 4.0, 6.0],
            [0.0, 0.0, 2.0],
            [2.0, 0.0, 0.0],
        );
        write_morph_record(
            &mut records,
            2,
            [-2.0, -4.0, -6.0],
            [0.0, 0.0, -2.0],
            [-2.0, 0.0, 0.0],
        );
        fs::write(
            dir.join(format!(
                "000002-cs-t0={source_hash}-cs=eeeeeeeeeeeeeeee.buf"
            )),
            records,
        )
        .expect("write morph source");
        write_cb0(
            &dir.join(format!("000002-cs-cb0={cb0_hash}-cs=eeeeeeeeeeeeeeee.buf")),
            0,
            2,
            0.5,
        );
        let mut unrelated_records = Vec::new();
        write_morph_record(
            &mut unrelated_records,
            0,
            [100.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        );
        fs::write(
            dir.join("000003-cs-t0=eeeeeeee-cs=eeeeeeeeeeeeeeee.buf"),
            unrelated_records,
        )
        .expect("write unrelated morph source");
        write_cb0(
            &dir.join("000003-cs-cb0=ffffffff-cs=eeeeeeeeeeeeeeee.buf"),
            0,
            1,
            1.0,
        );

        let stride = 40;
        let mut position = vec![0u8; stride * 3];
        write_position_vertex(
            &mut position,
            0,
            stride,
            [1.0, 2.0, 3.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        );
        write_position_vertex(
            &mut position,
            1,
            stride,
            [5.0, 7.0, 9.0],
            [0.0, 1.0, 1.0],
            [2.0, 0.0, 0.0, 1.0],
        );
        write_position_vertex(
            &mut position,
            2,
            stride,
            [6.0, 6.0, 6.0],
            [0.0, 1.0, -1.0],
            [-1.0, 0.0, 0.0, 1.0],
        );

        let log = format!(
            "\
000002 CopyResource(pDstResource:0x1, pSrcResource:0x2)\n\
       Src: resource=0x2 hash=99999999\n\
       Dst: resource=0x1 hash={uav_hash}\n\
000002 CSSetConstantBuffers(StartSlot:0, NumBuffers:1, ppConstantBuffers:0x1)\n\
       0: resource=0x3 hash={cb0_hash}\n\
000002 CSSetShaderResources(StartSlot:0, NumViews:1, ppShaderResourceViews:0x1)\n\
       0: view=0x4 resource=0x5 hash={source_hash}\n\
000002 CSSetUnorderedAccessViews(StartSlot:0, NumUAVs:1, ppUnorderedAccessViews:0x1)\n\
       0: view=0x6 resource=0x1 hash={uav_hash}\n\
000002 CSSetShader(pComputeShader:0x7, ppClassInstances:0x0, NumClassInstances:0) hash={shader_hash}\n\
000002 Dispatch(ThreadGroupCountX:1, ThreadGroupCountY:1, ThreadGroupCountZ:1)\n\
000003 CSSetUnorderedAccessViews(StartSlot:0, NumUAVs:1, ppUnorderedAccessViews:0x0)\n\
000003 CSSetConstantBuffers(StartSlot:0, NumBuffers:1, ppConstantBuffers:0x1)\n\
       0: resource=0x7 hash=ffffffff\n\
000003 CSSetShaderResources(StartSlot:0, NumViews:1, ppShaderResourceViews:0x1)\n\
       0: view=0x8 resource=0x9 hash=eeeeeeee\n\
000003 Dispatch(ThreadGroupCountX:1, ThreadGroupCountY:1, ThreadGroupCountZ:1)\n\
000009 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)\n\
       0: resource=0x1 hash={final_hash}\n\
000009 Draw(VertexCount:3, StartVertexLocation:0)\n\
000009 CopyResource(pDstResource:0x1, pSrcResource:0x2)\n\
       Src: resource=0x2 hash={uav_hash}\n\
       Dst: resource=0x1 hash={final_hash}\n"
        );
        fs::write(dir.join("log.txt"), log).expect("write log");

        let fa = FrameAnalysis::new(dir.to_string_lossy().as_ref()).expect("frame analysis");
        let game_type = test_game_type();
        let result = extract(&fa, "000009", &position, stride, &game_type)
            .expect("morph extraction")
            .expect("morph chain should be found");

        assert_eq!(read_f32(&result.base_position_buf, 0), 1.0);
        assert_eq!(read_f32(&result.base_position_buf, 4), 2.0);
        assert_eq!(read_f32(&result.base_position_buf, 8), 3.0);
        assert_eq!(read_f32(&result.base_position_buf, 40), 4.0);
        assert_eq!(read_f32(&result.base_position_buf, 44), 5.0);
        assert_eq!(read_f32(&result.base_position_buf, 48), 6.0);
        assert_eq!(read_f32(&result.base_position_buf, 80), 7.0);
        assert_eq!(read_f32(&result.base_position_buf, 84), 8.0);
        assert_eq!(read_f32(&result.base_position_buf, 88), 9.0);
        assert_eq!(read_f32(&result.base_position_buf, 52), 0.0);
        assert_eq!(read_f32(&result.base_position_buf, 56), 1.0);
        assert_eq!(read_f32(&result.base_position_buf, 60), 0.0);

        assert_eq!(result.shape_key_offset_buf.len(), 128 * 4);
        let offsets: Vec<u32> = result
            .shape_key_offset_buf
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        assert_eq!(&offsets[..3], &[0, 2, 2]);
        assert_eq!(offsets[127], 2);

        let vertex_ids: Vec<u32> = result
            .shape_key_vertex_id_buf
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        assert_eq!(vertex_ids, vec![1, 2]);
        assert_eq!(result.shape_key_vertex_offset_buf.len(), 2 * 6);
        assert_eq!(
            result.shape_key_vertex_attribute_offset_buf.len(),
            2 * MORPH_VERTEX_ATTRIBUTE_SCALAR_COUNT * 4
        );
        assert_eq!(
            read_f32(&result.shape_key_vertex_attribute_offset_buf, 4),
            4.0
        );
        assert_eq!(
            read_f32(&result.shape_key_vertex_attribute_offset_buf, 20),
            2.0
        );
        assert_eq!(
            read_f32(&result.shape_key_vertex_attribute_offset_buf, 24),
            2.0
        );
        let first_delta = u16::from_le_bytes(
            result.shape_key_vertex_offset_buf[0..2]
                .try_into()
                .expect("first delta"),
        );
        assert_eq!(f16_to_f32(first_delta), 2.0);
        assert_eq!(result.shape_keys_info.vertex_count, 2);
        assert_eq!(result.shape_keys_info.dispatch_y, 1);
        assert_eq!(result.shape_keys_info.checksum, 6);
        assert_eq!(
            result.shape_keys_info.native_morph_keys,
            vec![NativeMorphKey {
                shader_hash: shader_hash.to_string(),
                source_hash: source_hash.to_string(),
                constant_buffer_hash: cb0_hash.to_string(),
                offset: 0,
                count: 2,
                shape_key_index: 0,
            }]
        );

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn returns_none_when_the_pointlist_position_has_no_morph_copy_chain() {
        let dir = create_temp_dir("no-chain");
        let log = "\
000009 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)\n\
       0: resource=0x1 hash=bbbbbbbb\n\
000009 Draw(VertexCount:3, StartVertexLocation:0)\n";
        fs::write(dir.join("log.txt"), log).expect("write log");
        let fa = FrameAnalysis::new(dir.to_string_lossy().as_ref()).expect("frame analysis");

        let game_type = test_game_type();
        let result = extract(&fa, "000009", &[0u8; 120], 40, &game_type).expect("morph extraction");
        assert!(result.is_none());

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn rejects_partial_morph_chains_when_a_candidate_dump_is_missing() {
        let dir = create_temp_dir("partial-chain");
        let mut records = Vec::new();
        write_morph_record(
            &mut records,
            0,
            [2.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        );
        fs::write(
            dir.join("000002-cs-t0=11111111-cs=2222222222222222.buf"),
            records,
        )
        .expect("write morph source");
        write_cb0(
            &dir.join("000002-cs-cb0=33333333-cs=2222222222222222.buf"),
            0,
            1,
            1.0,
        );

        let log = "\
000002 CopyResource(pDstResource:0x1, pSrcResource:0x2)\n\
       Src: resource=0x2 hash=44444444\n\
       Dst: resource=0x1 hash=aaaaaaaa\n\
000002 CSSetConstantBuffers(StartSlot:0, NumBuffers:1, ppConstantBuffers:0x1)\n\
       0: resource=0x3 hash=33333333\n\
000002 CSSetShaderResources(StartSlot:0, NumViews:1, ppShaderResourceViews:0x1)\n\
       0: view=0x4 resource=0x5 hash=11111111\n\
000002 CSSetUnorderedAccessViews(StartSlot:0, NumUAVs:1, ppUnorderedAccessViews:0x1)\n\
       0: view=0x6 resource=0x1 hash=aaaaaaaa\n\
000002 CSSetShader(pComputeShader:0x7, ppClassInstances:0x0, NumClassInstances:0) hash=2222222222222222\n\
000002 Dispatch(ThreadGroupCountX:1, ThreadGroupCountY:1, ThreadGroupCountZ:1)\n\
000003 CSSetConstantBuffers(StartSlot:0, NumBuffers:1, ppConstantBuffers:0x1)\n\
       0: resource=0x7 hash=55555555\n\
000003 CSSetShaderResources(StartSlot:0, NumViews:1, ppShaderResourceViews:0x1)\n\
       0: view=0x8 resource=0x9 hash=66666666\n\
000003 Dispatch(ThreadGroupCountX:1, ThreadGroupCountY:1, ThreadGroupCountZ:1)\n\
000009 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)\n\
       0: resource=0x1 hash=bbbbbbbb\n\
000009 CopyResource(pDstResource:0x1, pSrcResource:0x2)\n\
       Src: resource=0x2 hash=aaaaaaaa\n\
       Dst: resource=0x1 hash=bbbbbbbb\n\
000009 Draw(VertexCount:1, StartVertexLocation:0)\n";
        fs::write(dir.join("log.txt"), log).expect("write log");
        let fa = FrameAnalysis::new(dir.to_string_lossy().as_ref()).expect("frame analysis");

        let game_type = test_game_type();
        let result = extract(&fa, "000009", &[0u8; 40], 40, &game_type);
        assert!(result.is_err());

        fs::remove_dir_all(dir).expect("remove temp dir");
    }
}
