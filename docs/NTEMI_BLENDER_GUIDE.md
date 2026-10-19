# NTEMI 模型导入导出指南

基于 `mod_importer` 参考实现——适配 SSMT4 的原始 Buffer + SubMeshJson 格式。

---

## 1. 整体流程

```
FrameAnalysis dump → SSMT4 提取 (.buf + .ib + .json)
                            ↓
                    Blender 导入 (本指南)
                            ↓
                    编辑模型 / 纹理
                            ↓
                    Blender 导出 → .buf + .ib
                            ↓
                    打包 Mod (.zip)
```

---

## 2. SubMeshJson 结构说明

每次提取生成一组文件，每 submesh 一个 `.json`：

```
{draw_ib}-{index_count}-{first_index}/
├── TYPE_GPU_P12_BI8_BW8_T8_T1-8_TA4_N4_/
│   ├── {name}.json          ← SubMeshJson
│   ├── {name}.ib            ← Index Buffer (R32_UINT)
│   ├── {name}-Blend.buf     ← Blend indices + weights
│   ├── {name}-Normal.buf    ← Tangent frame (pre-CS)
│   ├── {name}-Position.buf  ← Vertex positions
│   ├── {name}-Texcoord.buf  ← Packed UV (4 sets)
│   ├── {name}-Color.buf     ← Outline params (optional)
│   └── {name}-BoneMatrix.buf ← Bone matrices
```

### 2.1 JSON 关键字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `GamePreset` | string | `"NTEMI"` |
| `WorkGameType` | string | 数据类型名称，如 `"GPU_P12_BI8_BW8_T8_T1-8_TA4_N4_"` |
| `GPU-PreSkinning` | bool | `true` = GPU skinning (含 CS stage，需 Blend Category) |
| `VertexLimitVB` | string | VB0(Position) 的 hash |
| `VertexOffset` | i64 | submesh 在完整 VB 中的起始顶点索引 (= IB first_index) |
| `VertexCount` | i64 | 完整 VB 的顶点总数 |
| `IndexOffset` | i64 | IB 在全量 IB 中的起始位置 (= match_first_index) |
| `IndexCount` | i64 | IB 元素数量 (三角形数 × 3) |
| `VGCount` | i32 | submesh 使用的骨骼数 |
| `VGOffset` | i32 | submesh 在合并骨架中的 VG 起始偏移 |
| `VGMap` | dict | local VG id → merged VG id |
| `CB4Hash` | string | vs-cb4 skeleton buffer 的 hash |
| `BoneMatrixFileName` | string | 骨骼矩阵 buf 文件名 |
| `IndexBufferList` | array | IB 文件描述 |
| `CategoryBufferList` | array | 各类别 buffer 描述 |
| `CategoryHash` | dict | CategoryName → buffer hash，附加: `IB`, `DrawCallIndex`, `VertexMin`, `VertexMax`, `VertexUnique` |
| `CategoryDrawCategoryMap` | dict | CategoryName → DrawCategory |
| `TextureMarkUpInfoList` | array | 纹理绑定列表，每个元素包含 hash/slot/文件名 |
| `ShapeKeysInfo` | object | ShapeKey 元数据 (vertex_count / dispatch_y / checksum / buffer hashes) |

### 2.2 D3D11Element 描述

`CategoryBufferList` 中每个 buffer 的 `D3D11ElementList` 描述其内部布局：

| 字段 | 说明 |
|------|------|
| `SemanticName` | 语义名: `POSITION`, `TEXCOORD`, `BLENDINDICES`, `BLENDWEIGHTS`, `TANGENT`, `NORMAL`, `COLOR` |
| `SemanticIndex` | 同语义序号 (0-based) |
| `Format` | DXGI 格式: `R32G32B32_FLOAT`, `R16G16_FLOAT`, `R8G8B8A8_UINT` 等 |
| `ByteWidth` | 单个元素字节宽度 |
| `ExtractSlot` | 3DMigoto dump slot: `cs-t1`, `cs-t2`, `cs-t3`, `vs-t5`, `vs-t8` 等 |
| `ExtractTechnique` | `pointlist` 或 `trianglelist` |
| `Category` | 所属 Category: `Blend`, `Normal`, `Position`, `Texcoord`, `Color` |

### 2.3 TextureMarkUpInfoList 纹理绑定

从 FrameAnalysis 的 `ps-t*` dump 文件提取的纹理绑定信息。每个条目:

| 字段 | 类型 | 说明 |
|------|------|------|
| `MarkName` | string | 纹理槽位名，如 `"ps-t0"`, `"ps-t5"` |
| `MarkHash` | string | 纹理文件 hash (8 位 hex) |
| `MarkSlot` | string | 槽位编号，如 `"0"`, `"5"` |
| `MarkType` | string | 纹理格式，`"dds"` 或 `"jpg"` |
| `MarkFileName` | string | 去重后的原始纹理文件名 |

导入时按 slot 映射:
- `ps-t0` / `ps-t1` / `ps-t2`: 辅助纹理 (mask, detail 等)
- `ps-t5`: 法线贴图 (normal map, BC5)
- `ps-t7`: 基础颜色贴图 (albedo/base color, BC7)

### 2.4 ShapeKeysInfo 形态键元数据

从 CS(cs-cb0, cs-t0, cs-t1) buffer 提取的 ShapeKey/Morph 参数。所有 submesh 共享同一组数据。

| 字段 | 类型 | 说明 |
|------|------|------|
| `vertex_count` | i32 | 受 ShapeKey 影响的顶点总数 (= cb0_u32[127]) |
| `dispatch_y` | i32 | CS dispatch Y 维度 = ceil(vertex_count / 32) |
| `checksum` | i32 | cb0 前 4 个 u32 之和 |
| `offsets_hash` | string | cs-cb0 buffer 的 8 位 hash (ShapeKey dispatch params) |
| `scale_hash` | string | 预留，NTEMI 暂空 |
| `vertex_ids_hash` | string | cs-t0 buffer 的 8 位 hash (ShapeKeyVertexId) |
| `vertex_offsets_hash` | string | cs-t1 buffer 的 8 位 hash (ShapeKeyVertexOffset) |

ShapeKey 相关 buffer 解析:
- **cs-cb0**: 前 128 个 u32 = shapekey offsets 数组
- **cs-t0**: ShapeKeyVertexId — 每个受影响的顶点在 VB 中的索引 (每项 4 字节)
- **cs-t1**: ShapeKeyVertexOffset — 每项 12 字节 (3 x float delta position)

### 2.5 CategoryHash 附加字段

除各 Category buffer 的 hash 外，`CategoryHash` dict 还包含:

| Key | 说明 |
|-----|------|
| `IB` | Index Buffer 文件的 8 位 hash |
| `DrawCallIndex` | 3DMigoto dump 索引号 (6 位数字) |
| `VertexMin` | IB 中的最小顶点索引 |
| `VertexMax` | IB 中的最大顶点索引 |
| `VertexUnique` | IB 中的唯一顶点数 |

---

## 3. Buffer 文件格式详解

所有 buffer 均为 little-endian。每条记录连续排列，无 padding，stride = 各元素 ByteWidth 之和。

### 3.1 Position Buffer (`{name}-Position.buf`)

```
Format:      R32G32B32_FLOAT
Stride:      12 bytes/vertex
Count:       VertexCount 条记录
Layout:      每顶点: float x, float y, float z (各 4 字节 LE)
```

Python 读取:
```python
import struct
def read_positions(path):
    data = Path(path).read_bytes()
    return [struct.unpack_from("<3f", data, i * 12) for i in range(len(data) // 12)]
```

### 3.2 Blend Buffer (`{name}-Blend.buf`)

```
Format:      R8G8B8A8_UINT (indices) + R8G8B8A8_UNORM (weights)
Stride:      8 bytes/vertex
Count:       VertexCount 条记录
Layout:      [indices: u8×4][weights: u8×4]
              = [i0,i1,i2,i3,w0,w1,w2,w3] 共 8 字节
```

- `indices`: 骨骼索引 (0-255)，仅低 8 位有效
- `weights`: 归一化权重 (0-255 → 0.0-1.0)。

Python 读取:
```python
def read_blend_weights(path):
    data = Path(path).read_bytes()
    count = len(data) // 8
    indices = []
    weights = []
    for i in range(count):
        off = i * 8
        idx = struct.unpack_from("<4B", data, off)
        wt  = struct.unpack_from("<4B", data, off + 4)
        indices.append(idx)
        weights.append(tuple(w / 255.0 for w in wt))
    return indices, weights
```

### 3.3 Normal / Tangent Frame Buffer (`{name}-Normal.buf`)

```
Format:      R8G8B8A8_SNORM × 2
Stride:      8 bytes/vertex
Count:       VertexCount 条记录
Layout:      [FrameA: snorm8×4][FrameB: snorm8×4]
              FrameA.xyz = tangent (game space)
              FrameA.w   = 1.0 (reserved)
              FrameB.xyz = normal (game space)
              FrameB.w   = bitangent_sign (+1.0 / -1.0)
```

SNORM8 解码: `value = max(-1.0, (u8 as signed) / 127.0)`, 其中 -128 → -1.0。

Python 读取:
```python
def snorm8_to_float(b):
    signed = b if b < 128 else b - 256
    if signed == -128: return -1.0
    return max(-1.0, signed / 127.0)

def read_tangent_frames(path):
    data = Path(path).read_bytes()
    count = len(data) // 8
    tangents, normals, signs = [], [], []
    for i in range(count):
        off = i * 8
        a = struct.unpack_from("<4B", data, off)
        b = struct.unpack_from("<4B", data, off + 4)
        tangent = tuple(snorm8_to_float(a[j]) for j in range(3))
        normal  = tuple(snorm8_to_float(b[j]) for j in range(3))
        sign    = 1.0 if snorm8_to_float(b[3]) >= 0.0 else -1.0
        tangents.append(tangent)
        normals.append(normal)
        signs.append(sign)
    return tangents, normals, signs
```

### 3.4 Texcoord Buffer (`{name}-Texcoord.buf`)

```
Format:      R16G16_FLOAT × 8 (= 4 组 half2)
Stride:      16 bytes/vertex
Count:       VertexCount 条记录
Layout:      [UV0: half2][UV1: half2][UV2: half2][UV3: half2]
              每组 4 字节 = 2 个 half-float LE
```

Python 读取:
```python
def read_uvs(path):
    data = Path(path).read_bytes()
    count = len(data) // 16
    uvs = []
    for i in range(count):
        off = i * 16
        vals = struct.unpack_from("<8e", data, off)  # half-float
        uv_sets = [
            (float(vals[0]), float(vals[1])),
            (float(vals[2]), float(vals[3])),
            (float(vals[4]), float(vals[5])),
            (float(vals[6]), float(vals[7])),
        ]
        uvs.append(uv_sets)
    return uvs
```

### 3.5 Color / Outline Buffer (`{name}-Color.buf`)

```
Format:      R8G8B8A8_UNORM
Stride:      4 bytes/vertex
Count:       VertexCount 条记录
Layout:      [R: u8][G: u8][B: u8][A: u8]
```

Python 读取:
```python
def read_color(path):
    data = Path(path).read_bytes()
    return [struct.unpack_from("<4B", data, i * 4) for i in range(len(data) // 4)]
```

### 3.6 Index Buffer (`{name}.ib`)

```
Format:      R32_UINT (DXGI_FORMAT_R32_UINT)
Layout:      连续 uint32 LE，每 3 个一组构成三角形
Count:       IndexCount 个元素
```

Python 读取:
```python
def read_ib(path):
    data = Path(path).read_bytes()
    indices = [struct.unpack_from("<I", data, i * 4)[0] for i in range(len(data) // 4)]
    triangles = [(indices[i], indices[i+1], indices[i+2]) for i in range(0, len(indices), 3)]
    return triangles
```

### 3.7 Bone Matrix Buffer (`{name}-BoneMatrix.buf`)

```
Format:      float3x4 per bone
Stride:      48 bytes/bone
Layout:      [row0: float4][row1: float4][row2: float4]
              每行 16 字节, row.xyz = 旋转/缩放, row.w = 平移
Count:       由文件大小 / 48 决定
```

---

## 4. 坐标系统转换

NTEMI (异环) 使用 Unity-like 坐标系:

| | 游戏空间 | Blender 空间 |
|---|---|---|
| X | 右 (Right) | **-X** (左) |
| Y | 上 (Up)   | **-Y** (下) |
| Z | 前 (Forward) | Z (前) |
| 缩放 | 1 unit = 1m | 1 unit = 1m → 0.01 scale |

转换公式:
```python
def game_to_blender_position(pos):
    return (-pos[0] * 0.01, -pos[1] * 0.01, pos[2] * 0.01)

def blender_to_game_position(pos):
    return (-pos[0] * 100.0, -pos[1] * 100.0, pos[2] * 100.0)

def game_to_blender_direction(d):
    return (-d[0], -d[1], d[2])

def blender_to_game_direction(d):
    return (-d[0], -d[1], d[2])
```

方向向量仅翻转轴符号，不做缩放。法线、切线、bitangent 都走方向转换。

---

## 5. Blender 导入流程

### 5.1 读入 SubMeshJson

```python
import json

with open("path/to/submesh.json") as f:
    j = json.load(f)

# 获取目录
import os
json_dir = os.path.dirname(json_path)
```

### 5.2 构建顶点数据

```python
# 1. 读所有 Category Buffer
for buf_info in j["CategoryBufferList"]:
    buf_path = os.path.join(json_dir, buf_info["FileName"])
    for elem in buf_info["D3D11ElementList"]:
      # 按 SemanticName 路由到对应解析函数

# 2. 读 IB
ib_path = os.path.join(json_dir, j["IndexBufferList"][0]["FileName"])
triangles = read_ib(ib_path)
```

### 5.3 Compact Geometry

IB 中的顶点索引引用完整 VB 中的位置。需要将实际使用的顶点压缩到稠密范围：

```python
def compact_geometry(positions, triangles, uv_data):
    remap = {}
    orig_ids = []
    new_positions = []
    new_uvs = []
    new_triangles = []

    for tri in triangles:
        new_tri = []
        for v_orig in tri:
            if v_orig not in remap:
                remap[v_orig] = len(orig_ids)
                orig_ids.append(v_orig)
                new_positions.append(positions[v_orig])
                new_uvs.append(uv_data[v_orig])
            new_tri.append(remap[v_orig])
        new_triangles.append(tuple(new_tri))

    return new_positions, new_triangles, new_uvs, orig_ids
```

### 5.4 创建 Blender Mesh

```python
import bpy
from mathutils import Vector

# 转换坐标
blender_positions = [game_to_blender_position(p) for p in compact_positions]

# 翻转三角形朝向 (Blender 正面 = CCW, DX 正面 = CW)
blender_triangles = [(t[0], t[2], t[1]) for t in compact_triangles]

# 创建 mesh
mesh = bpy.data.meshes.new("NTEMI_Mesh")
mesh.from_pydata(blender_positions, [], blender_triangles)
mesh.validate()
mesh.update()

obj = bpy.data.objects.new("NTEMI_Mesh", mesh)
bpy.context.collection.objects.link(obj)
```

### 5.5 应用 UV

```python
# 每个 UV set 创建一个 UV layer
for uv_idx in range(4):
    uv_layer = mesh.uv_layers.new(name=f"UV{uv_idx}")
    for poly in mesh.polygons:
        for loop_idx in poly.loop_indices:
            v_idx = mesh.loops[loop_idx].vertex_index
            u, v = compact_uvs[v_idx][uv_idx]
            uv_layer.data[loop_idx].uv = (u, 1.0 - v)  # V 翻转
    if uv_idx == 0:
        mesh.uv_layers.active = uv_layer
```

### 5.6 应用骨骼权重 (仅 GPU-PreSkinning)

```python
if j["GPU-PreSkinning"]:
    indices, weights = read_blend_weights(blend_buf_path)

    # 按 compact 后的顶点重排
    compact_indices = [indices[orig_id] for orig_id in orig_ids]
    compact_weights = [weights[orig_id] for orig_id in orig_ids]

    # 创建 vertex groups
    # 注意：VGMap 用于 local → merged bone index 映射
    vg_map = j.get("VGMap", {})
    vg_offset = j.get("VGOffset", 0)

    for v_idx, (idx_record, wt_record) in enumerate(zip(compact_indices, compact_weights)):
        for slot in range(4):
            local_bone = idx_record[slot]
            weight = wt_record[slot]
            if weight <= 0:
                continue
            # 通过 VGMap 映射到 merged bone index
            merged_bone = vg_map.get(str(local_bone), vg_offset + local_bone)
            vg = obj.vertex_groups.get(str(merged_bone))
            if vg is None:
                vg = obj.vertex_groups.new(name=str(merged_bone))
            vg.add([v_idx], weight, "ADD")
```

### 5.7 应用自定义法线 (Tangent Frame)

```python
# 从 pre-CS Normal buffer 解码
tangents, normals, signs = read_tangent_frames(normal_buf_path)

# compact
compact_normals = [normals[orig_id] for orig_id in orig_ids]
compact_tangents = [tangents[orig_id] for orig_id in orig_ids]
compact_signs = [signs[orig_id] for orig_id in orig_ids]

# 转换到 Blender 空间
blender_normals = [game_to_blender_direction(n) for n in compact_normals]

# 应用自定义法线
mesh.use_auto_smooth = True
mesh.normals_split_custom_set_from_vertices(blender_normals)

# 存储 tangent / bitangent_sign 作为 vertex attributes (供导出使用)
store_vector_attribute(mesh, "modimp_tangent", [
    game_to_blender_direction(t) for t in compact_tangents
])
store_vector_attribute(mesh, "modimp_normal", blender_normals)
store_float_attribute(mesh, "modimp_bitangent_sign", compact_signs)
```

### 5.8 应用 Outline 参数

```python
if color_exists:
    outline_data = read_color(color_buf_path)
    compact_outline = [outline_data[orig_id] for orig_id in orig_ids]

    # 存储为顶点属性
    for ch_idx, ch_name in enumerate(['r', 'g', 'b', 'a']):
        store_int_attribute(mesh, f"modimp_outline_{ch_name}",
            [record[ch_idx] for record in compact_outline])

    # 同时写入 vertex color layer
    color_attr = mesh.color_attributes.new(
        name="NTMI_OutlineParam", type="BYTE_COLOR", domain="POINT"
    )
    for item, record in zip(color_attr.data, compact_outline):
        item.color = tuple(c / 255.0 for c in record)
```

### 5.9 存储元数据 (供导出使用)

```python
obj["modimp_profile_id"] = "yihuan"
obj["modimp_ib_hash"] = j.get("CategoryHash", {}).get("IB", "")
obj["modimp_region_hash"] = j.get("VertexLimitVB", "")
obj["modimp_region_index_count"] = int(j.get("IndexCount", 0))
obj["modimp_region_first_index"] = int(j.get("IndexOffset", 0))
obj["modimp_first_index"] = int(j.get("VertexOffset", 0))
obj["modimp_index_count"] = int(j.get("IndexCount", 0))
obj["modimp_slice_order"] = int(j.get("VertexOffset", 0))
obj["modimp_import_variant"] = "pre_cs" if j.get("GPU-PreSkinning") else "post_cs"
obj["modimp_mirror_flip"] = False
obj["modimp_match_vs_position_hash"] = j.get("CategoryHash", {}).get("POSITION", "")
obj["modimp_match_vs_texcoord_hash"] = j.get("CategoryHash", {}).get("TEXCOORD", "")
obj["modimp_match_vs_outline_hash"] = j.get("CategoryHash", {}).get("COLOR", "")
# 纹理绑定 hash (供导出时匹配原始纹理)
obj["modimp_texture_hash_dict"] = json.dumps({
    item["MarkName"]: item["MarkHash"]
    for item in j.get("TextureMarkUpInfoList", [])
})
# ShapeKey 信息
obj["modimp_shape_keys"] = json.dumps(j.get("ShapeKeysInfo", {}))
# IK 骨骼合并图 (所有 submesh 合并后写入 collection)
collection["modimp_bone_merge_map_text"] = build_bone_merge_map(all_submeshes)
```

### 5.10 应用纹理 (从 TextureMarkUpInfoList)

```python
texture_marks = j.get("TextureMarkUpInfoList", [])
for tex_info in texture_marks:
    tex_path = resolve_deduped_texture_path(tex_info["MarkFileName"])
    if tex_info["MarkSlot"] == "7":  # ps-t7 = base color
        apply_texture(obj, tex_path, "BaseColor", color_space='sRGB')
    elif tex_info["MarkSlot"] == "5":  # ps-t5 = normal map
        apply_texture(obj, tex_path, "NormalMap", color_space='Non-Color')
```

---

## 5bis. Post-CS (CPU Pre-Skinning) 导入流程

当 `GPU-PreSkinning` = `false` 时，使用 Post-CS 流程。模型已完成 CPU skinning，buffer 均为 trianglelist，无需 Blend/CB4/VGMap。

### 5bis.1 Post-CS Buffer 布局

```
CategoryBufferList (全部 trianglelist):
  Position  → vb0:  R32G32B32_FLOAT (12 bytes/vertex, 已蒙皮)
  Texcoord  → vb1:  R16G16_FLOAT × 8 (16 bytes/vertex, 4 UV sets)
  Normal    → vs-t7: R8G8B8A8_SNORM × 2 (8 bytes/vertex, tangent frame post-CS)
  Color     → vs-t8: R8G8B8A8_UNORM (4 bytes/vertex, outline, optional)
```

### 5bis.2 差异点

| | Pre-CS (GPU skinning) | Post-CS (CPU skinning) |
|---|---|---|
| Position slot | `cs-t3` (pointlist, 未蒙皮) | `vb0` (trianglelist, 已蒙皮) |
| Tangent frame slot | `cs-t2` (pointlist, 未变换) | `vs-t7` (trianglelist, 已变换) |
| Blend weights | `cs-t1` (pointlist, R8G8B8A8) | 无 |
| Texcoord slot | `vs-t5` | `vb1` |
| BoneMatrix / VGMap | 有 | 无 |
| ShapeKeys | 有 (CS dispatch) | 无 |

### 5bis.3 导入 Post-CS Mesh

```python
# Post-CS 简化流程: 无骨骼/无 VGMap，position 已蒙皮
# 1. 按 D3D11ElementList 顺序读各 buffer
# 2. Position 直接使用 (已 skinned)
# 3. Tangent frame 从 vs-t7 解码 (同 snorm8 解码逻辑)
# 4. 无 blend / VGMap / bone matrix 步骤
# 5. 其余 (UV, color, IB, 坐标转换) 与 pre-CS 相同
```

---

## 6. Blender 导出流程 (生成 Mod)

### 6.1 读取编辑后的 Mesh

```python
def export_mesh_to_buffers(obj, output_dir):
    # 获取 evaluated mesh (应用所有 modifier + shape key)
    depsgraph = bpy.context.evaluated_depsgraph_get()
    eval_obj = obj.evaluated_get(depsgraph)
    mesh = bpy.data.meshes.new_from_object(eval_obj)

    # 应用 world matrix
    mesh.transform(eval_obj.matrix_world)
    mesh.update()

    # 三角化
    import bmesh
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    mesh.calc_loop_triangles()
```

### 6.2 Write Position Buffer

```python
def write_positions(mesh, output_path):
    buf = bytearray()
    for v in mesh.vertices:
        game_pos = blender_to_game_position((v.co.x, v.co.y, v.co.z))
        buf.extend(struct.pack("<3f", *game_pos))
    Path(output_path).write_bytes(buf)
```

### 6.3 Write Index Buffer

```python
def write_ib(mesh, output_path, use_16bit=False):
    buf = bytearray()
    for tri in mesh.loop_triangles:
        # 翻转回 DX 正面
        if use_16bit:
            buf.extend(struct.pack("<HHH", tri.vertices[0], tri.vertices[2], tri.vertices[1]))
        else:
            buf.extend(struct.pack("<III", tri.vertices[0], tri.vertices[2], tri.vertices[1]))
    Path(output_path).write_bytes(buf)
```

### 6.4 Write UV Buffer

```python
def write_uvs(mesh, output_path):
    uv_layers = [mesh.uv_layers.get(f"UV{i}") for i in range(4)]
    buf = bytearray()
    for v_idx in range(len(mesh.vertices)):
        for uv_layer in uv_layers:
            # 找到该顶点的第一个 loop 获取 UV
            found = False
            for poly in mesh.polygons:
                for loop_idx in poly.loop_indices:
                    if mesh.loops[loop_idx].vertex_index == v_idx:
                        u, v = uv_layer.data[loop_idx].uv
                        buf.extend(struct.pack("<2e", u, 1.0 - v))  # V 翻转回
                        found = True
                        break
                if found:
                    break
            if not found:
                buf.extend(struct.pack("<2e", 0.0, 0.0))
    Path(output_path).write_bytes(buf)
```

### 6.5 Write Blend Buffer

从 Blender vertex groups 重建 blend indices + weights:

```python
def write_blend_weights(obj, mesh, output_path, vg_map_reverse):
    buf = bytearray()
    for v_idx, v in enumerate(mesh.vertices):
        # 取顶点前 4 个 VG (按 weight 排序)
        vgs = sorted(
            [(g.group, g.weight) for g in v.groups],
            key=lambda x: -x[1]
        )[:4]
        # 补齐到 4
        while len(vgs) < 4:
            vgs.append((0, 0.0))

        indices = [vg_map_reverse.get(gid, gid) for gid, _ in vgs]
        weights = [min(255, max(0, int(w * 255))) for _, w in vgs]

        buf.extend(struct.pack("<4B", *indices))
        buf.extend(struct.pack("<4B", *weights))
    Path(output_path).write_bytes(buf)
```

### 6.6 Write Tangent Frame Buffer

从 vertex attributes 重建 snorm8 编码的 tangent frame:

```python
def snorm8_from_float(v):
    clamped = max(-1.0, min(1.0, v))
    if clamped <= -1.0:
        signed = -128
    else:
        signed = int(round(clamped * 127.0))
        signed = max(-127, min(127, signed))
    return signed & 0xFF

def write_tangent_frames(mesh, output_path):
    tangents = read_vector_attr(mesh, "modimp_tangent")
    normals  = read_vector_attr(mesh, "modimp_normal")
    signs    = read_float_attr(mesh, "modimp_bitangent_sign")

    buf = bytearray()
    for t_bl, n_bl, sign in zip(tangents, normals, signs):
        t_game = blender_to_game_direction(t_bl)
        n_game = blender_to_game_direction(n_bl)

        # Frame A: tangent.xyz + 1.0
        buf.extend(bytes([
            snorm8_from_float(t_game[0]),
            snorm8_from_float(t_game[1]),
            snorm8_from_float(t_game[2]),
            127,  # 1.0 in snorm8
        ]))
        # Frame B: normal.xyz + bitangent_sign
        buf.extend(bytes([
            snorm8_from_float(n_game[0]),
            snorm8_from_float(n_game[1]),
            snorm8_from_float(n_game[2]),
            snorm8_from_float(sign),
        ]))

    Path(output_path).write_bytes(buf)
```

### 6.7 Write Outline/Color Buffer

```python
def write_color_buffer(mesh, output_path):
    color_attr = mesh.color_attributes.get("NTMI_OutlineParam")
    if color_attr is None:
        return  # optional

    buf = bytearray()
    for item in color_attr.data:
        buf.extend(struct.pack("<4B", *[
            max(0, min(255, int(c * 255))) for c in item.color[:4]
        ]))
    Path(output_path).write_bytes(buf)
```

---

## 7. Shape Keys (Morph Targets)

### 7.1 解析 cs-cb0 (ShapeKey dispatch params)

```python
def read_shape_key_cb0(cb0_path):
    """cs-cb0 buffer: 前 128 个 u32 为 shapekey offset 数组"""
    data = Path(cb0_path).read_bytes()
    count = min(len(data) // 4, 128)
    offsets = struct.unpack(f"<{count}I", data[:count * 4])

    vertex_count = offsets[127]  # 受影响的顶点总数
    checksum = offsets[0] + offsets[1] + offsets[2] + offsets[3]
    dispatch_y = (vertex_count + 31) // 32

    return {
        "offsets": offsets,
        "vertex_count": vertex_count,
        "checksum": checksum,
        "dispatch_y": dispatch_y,
    }
```

### 7.2 解析 cs-t0 (ShapeKeyVertexId)

```python
def read_shape_key_vertex_ids(t0_path, vertex_count):
    """cs-t0: 每个受影响的顶点在 VB 中的索引 (R32_UINT)"""
    data = Path(t0_path).read_bytes()
    entry_count = len(data) // 4  # u32 per vertex
    return struct.unpack(f"<{entry_count}I", data[:entry_count * 4])
```

### 7.3 解析 cs-t1 (ShapeKeyVertexOffset)

```python
def read_shape_key_vertex_offsets(t1_path):
    """cs-t1: delta positions (3 x float per vertex, 12 bytes/entry)"""
    data = Path(t1_path).read_bytes()
    entry_count = len(data) // 12
    offsets = []
    for i in range(entry_count):
        off = i * 12
        delta = struct.unpack_from("<3f", data, off)
        offsets.append(delta)
    return offsets
```

### 7.4 导入 ShapeKeys 到 Blender

```python
shape_info = j.get("ShapeKeysInfo", {})
if shape_info.get("vertex_count", 0) > 0:
    # 1. 定位 cs-cb0/cs-t0/cs-t1 buffer 文件
    #    (文件名格式: {pointlist_index}-cs-t0=XXXXXXXX.buf)
    #    通过 ShapeKeysInfo.hashes 关联到 deduped 文件

    # 2. 读取 vertex_ids (cs-t0) → 哪些顶点受影响
    vertex_ids = read_shape_key_vertex_ids(t0_path, shape_info["vertex_count"])

    # 3. 读取 all_offsets (cs-t1) → delta 位置
    all_offsets = read_shape_key_vertex_offsets(t1_path)

    # 4. 读取 cb0 offsets 数组 → 每个 shape key 在 cs-t1 中的 [start, end)
    cb0 = read_shape_key_cb0(cb0_path)
    key_count = cb0["offsets"][0]  # 第一个 offset 值 = shape key 数量

    # 5. 为每个 shape key 创建 Blender shape key
    for key_idx in range(key_count):
        start = cb0["offsets"][key_idx]
        end = cb0["offsets"][key_idx + 1]
        key_vertex_ids = vertex_ids[start:end]
        key_offsets = all_offsets[start:end]

        # 创建 shape key 并设置 delta
        shape_key = obj.shape_key_add(name=f"Shape_{key_idx}")
        for vid, delta in zip(key_vertex_ids, key_offsets):
            # delta 是 game-space offset，需转换
            delta_bl = game_to_blender_direction(delta)
            shape_key.data[vid].co += Vector(delta_bl)
```

### 7.5 导出 ShapeKeys

```python
# Blender shape keys → NTEMI compute shader input buffers
# 1. 收集所有 mesh objects 的所有 shape key names
# 2. 对每个 shape key，evaluate delta vs basis
# 3. 收集受影响的顶点 ID 和 delta offsets
# 4. 写入 cs-t0 (R32_UINT vertex IDs)
# 5. 写入 cs-t1 (R32G32B32_FLOAT vertex offsets)
# 6. 写入 cs-cb0 (128 个 u32 offset 数组)
```

---

## 8. 纹理处理

### 8.1 纹理来源

纹理绑定信息存储在 `TextureMarkUpInfoList` 中。每条记录包含:
- `MarkName`: slot 名 (`ps-t0` ~ `ps-t7`)
- `MarkHash`: 纹理文件 8 位 hash (用于去重路径查找)
- `MarkSlot`: slot 编号
- `MarkFileName`: 去重后的原始文件名

常见 NTEMI 纹理槽位:
- `ps-t5` = normal map (BC5_UNORM)
- `ps-t7` = base color / albedo (BC7_UNORM_SRGB)

格式一般为 `.dds` (BC7/BC5)，偶有 `.jpg`。

纹理文件位于 FrameAnalysis `dedicated/` 目录下，文件名与 `MarkFileName` 一致。

### 8.2 Blender 中应用

```python
import bpy

def apply_texture(obj, texture_path, image_node_label, *, color_space='sRGB'):
    mat = bpy.data.materials.new(f"{obj.name}_Material")
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    links = mat.node_tree.links

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    output = nodes.new("ShaderNodeOutputMaterial")
    links.new(bsdf.outputs["BSDF"], output.inputs["Surface"])

    tex_node = nodes.new("ShaderNodeTexImage")
    tex_node.label = image_node_label
    img = bpy.data.images.load(texture_path)
    img.colorspace_settings.name = color_space
    tex_node.image = img

    # 连接到 Base Color
    links.new(tex_node.outputs["Color"], bsdf.inputs["Base Color"])

    obj.data.materials.append(mat)
```

### 8.3 导出纹理

```python
# 导出时使用 texconv 转换为 DDS
# BC7_UNORM_SRGB for base color
# BC5_UNORM for normal maps
```

---

## 9. Bone Merge Map (BMC)

当多个 submesh 共享骨骼矩阵时，需要生成 bone merge map：

```python
# 从 BoneMatrix.buf 读取各 submesh 的骨骼矩阵
# 比较矩阵内容识别重复骨骼
# 生成映射表: global_bone_index → local_submesh_bone_index
# 存储在 collection 属性 modimp_bone_merge_map_text
```

每个 submesh 的 `VGMap` 已记录了 local VG → merged VG 的映射。合并所有 submesh 的 VGMap 即可得到完整的 bone merge map。

---

## 10. 目录结构约定

### 10.1 Pre-CS (GPU skinning) 布局

```
FrameAnalysis/
├── FrameAnalysisLog.txt       ← 3DMigoto 抓帧日志
├── dedicated/                  ← 去重后的 dump 文件
│   ├── *-cs-t1=*.buf          ← Blend indices+weights
│   ├── *-cs-t2=*.buf          ← Tangent frame pre-CS
│   ├── *-cs-t3=*.buf          ← Position pre-CS
│   ├── *-vs-t5=*.buf          ← Texcoord UV sets
│   ├── *-vs-t8=*.buf          ← Color/Outline
│   ├── *-vs-cb4=*.buf         ← Bone matrices
│   ├── *-cs-cb0=*.buf         ← ShapeKey dispatch params
│   ├── *-cs-t0=*.buf          ← ShapeKey vertex IDs
│   ├── *-cs-t1=*.buf          ← ShapeKey vertex offsets
│   ├── *-ps-t*.dds            ← 纹理
│   └── *.txt                  ← 3DMigoto metadata
└── ...

Workspace/
├── drawib.ini                  ← DrawIB 配置文件
├── {draw_ib}-{count}-{first}/
│   └── TYPE_GPU_.../
│       ├── {name}.json
│       ├── {name}.ib
│       ├── {name}-Blend.buf
│       ├── {name}-Normal.buf
│       ├── {name}-Position.buf
│       ├── {name}-Texcoord.buf
│       ├── {name}-Color.buf
│       └── {name}-BoneMatrix.buf
├── textures/
│   └── *.dds
└── ...
```

### 10.2 Post-CS (CPU skinning) 布局

```
FrameAnalysis/
├── dedicated/
│   ├── *-vb0=*.buf            ← Skinned Position
│   ├── *-vb1=*.buf            ← Texcoord UV sets
│   ├── *-vs-t7=*.buf          ← Tangent frame post-CS
│   ├── *-vs-t8=*.buf          ← Color/Outline (optional)
│   └── ...
└── ...

Workspace/
├── {draw_ib}-{count}-{first}/
│   └── TYPE_CPU_.../
│       ├── {name}.json
│       ├── {name}.ib
│       ├── {name}-Position.buf
│       ├── {name}-Texcoord.buf
│       ├── {name}-Normal.buf
│       └── {name}-Color.buf    ← optional
└── ...
```

---

## 11. 完整导入脚本骨架

参考 `mod_importer/tools/import_frameanalysis_mesh_to_blender.py` 获取可运行的完整实现。核心流程：

**Pre-CS (GPU-PreSkinning = true):**
```
1. 加载 SubMeshJson → 获取 buffer 文件列表
2. 读取所有 Category buffer → 构建顶点数据
3. 读取 IB → 构建三角形索引
4. Compact geometry → 压缩顶点范围
5. 转换坐标 → game space → Blender space
6. 创建 mesh → from_pydata()
7. 应用 UV → 4 个 UV layers
8. 应用骨骼 → Read Blend → VGMap 映射 → vertex groups
9. 读取 BoneMatrix → 应用骨骼矩阵
10. 应用法线 → Read Normal buffer → custom split normals + tangent attr
11. 应用材质 → 从 TextureMarkUpInfoList 获取纹理绑定
12. 应用 ShapeKeys → 从 ShapeKeysInfo 定位 cs-t0/cs-t1 → 解析 delta
13. 应用 Outline → 从 Color buffer 读取 → vertex color layer
14. 存储元数据 → object properties (hash, index info, VGMap, texture hashes)
```

**Post-CS (GPU-PreSkinning = false):**
```
1-7.  同上
8.   应用法线 → Read Normal buffer (vs-t7, post-CS tangent frame)
9.   应用材质 → 从 TextureMarkUpInfoList 获取纹理绑定
10.  应用 Outline → 从 Color buffer 读取 (if present)
11.  存储元数据 → object properties (无 VGMap/Blend/BoneMatrix)
```

---

## 12. 参考

- `mod_importer` (NTEMI 参考实现): https://github.com/ssice-a/mod_importer
- SSMT4 提取代码: `src-tauri/src/extract_new/ntemi.rs`
- 数据类型定义: `src-tauri/resources/GameType/NTEMI/*.json`
  （运行时直接读这一份：类型库随程序发布，用户配置目录下不再有副本）
- SubMeshJson 结构: `src-tauri/src/workspace/submesh_json.rs`
