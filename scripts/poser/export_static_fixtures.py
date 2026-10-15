"""从现有只读 JSON 导出 Poser 离线 STATIC 夹具；不访问游戏进程。"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys
import zlib


def require(condition, message):
    if not condition:
        raise ValueError(message)


def finite(values, count):
    require(len(values) == count and all(math.isfinite(x) for x in values), '静态向量或矩阵无效')
    return values


def multiply(a, b):
    x, y, z, w = a
    X, Y, Z, W = b
    return (w*X+x*W+y*Z-z*Y, w*Y-x*Z+y*W+z*X,
            w*Z+x*Y-y*X+z*W, w*W-x*X-y*Y-z*Z)


def build_reference(reference):
    require(reference.get('complete'), 'STATIC 参考未完成')
    nodes = reference['rig']
    rig = {b['index']: b for b in nodes}
    require(0 < len(rig) <= 256 and len(rig) == len(nodes), '骨架数量或索引无效')
    actors = [b for b in nodes if b['name'] == reference['actor_name']]
    require(len(actors) == 1, 'Actor 不唯一')
    actor = actors[0]['index']
    order, visiting = [], set()

    def visit(index):
        require(index in rig and index not in visiting, 'STATIC 父链缺失或存在环')
        if index in order:
            return
        visiting.add(index)
        if index != actor:
            node = rig[index]
            require(node['static_reference'], '缺少 STATIC 骨骼参考')
            finite(node['static_local_position'], 3)
            rotation = finite(node['static_model_rotation'], 4)
            require(abs(sum(x*x for x in rotation)-1) < .001, 'STATIC 四元数非单位')
            visit(node['parent'])
        visiting.remove(index)
        order.append(index)

    for index in rig:
        visit(index)
    indices = {original: i for i, original in enumerate(order)}
    avatar = bytearray(struct.pack('<I', len(order)))
    for original in order:
        node = rig[original]
        if original == actor:
            parent, position, rotation = -1, (0, 0, 0), (0, 0, 0, 1)
        else:
            parent = indices[node['parent']]
            position = node['static_local_position']
            parent_rotation = rig[node['parent']]['static_model_rotation'] or (0, 0, 0, 1)
            inverse = (*(-x for x in parent_rotation[:3]), parent_rotation[3])
            rotation = multiply(inverse, node['static_model_rotation'])
        avatar += struct.pack('<ii12fII', parent, -1, *position, 0, *rotation,
                              1, 1, 1, 0, zlib.crc32(node['name'].encode('utf-8')), 0)
    bodies = [m for m in reference['meshes'] if m.get('selected_body') and m.get('valid')]
    require(len(bodies) == 1, '所选 STATIC Body 不唯一')
    body = bodies[0]
    require(0 < len(body['bones']) <= 256, 'Body bind 数量无效')
    binds = bytearray()
    for item in body['bones']:
        require(item['rig_index'] in indices, 'Body bind 不属于所选骨架')
        node = rig[item['rig_index']]
        require(node['index'] != actor and item['name'] == node['name'], 'Body bind 身份不一致')
        name = node['name'].encode('utf-16-le')
        require(len(name) < 256, '骨名超过夹具容量')
        binds += struct.pack('<ii', indices[node['index']], indices[node['parent']])
        binds += name + bytes(256-len(name))
        binds += struct.pack('<16f', *finite(item['matrix_column_major'], 16))
    return bytes(avatar), bytes(binds), body


def build_skin(reference, capture, body):
    require(capture.get('complete') and capture['animator_native'] == reference['animator_native'],
            '采集与 STATIC Animator 身份不一致')
    meshes = [m for m in capture['static_mesh_sources'] if m['name'] == body['name'] and m.get('complete')]
    require(len(meshes) == 1, '所选 Body 源网格不唯一或不完整')
    mesh = meshes[0]
    if 'mesh_native' in body:
        require(mesh['mesh_native'] == body['mesh_native'], '源 Mesh 与 STATIC bind 身份不一致')
        identity = 'ANIMATOR_AND_MESH_NATIVE'
    else:
        require(mesh.get('source') == 'EXTERNAL_READONLY_STATIC_BODY_BIND_CROSSCHECKED',
                '旧 STATIC 参考缺少 Mesh 身份，需使用已交叉核验的采集')
        identity = 'LEGACY_EXTERNAL_CROSSCHECK_DECLARED'
    require(mesh['bone_weight_layout'] == '4xfloat32_4xint32', '未经支持的权重布局')
    count = mesh['vertex_count']
    require(0 < count <= 200000 and mesh['bone_weight_count'] == count, '顶点或权重数量无效')
    header = bytes.fromhex(mesh['shared_data_header_hex'])
    buffer = bytes.fromhex(mesh['cpu_vertex_bytes_hex'])
    require(len(header) >= 0x1D8 and len(buffer) == mesh['vertex_bytes'] and len(buffer) <= 8*1024*1024,
            '源网格缓冲长度无效')

    def channel(index):
        stream, offset, fmt, dimension = struct.unpack_from('<4B', header, 0x10+index*4)
        require(stream < 4 and fmt == 0 and dimension == 3, '位置 / 法线须为 float32x3')
        base = struct.unpack_from('<I', header, 0x34+stream*12)[0]
        stride = header[0x3A+stream*12]
        require(stride >= offset+12 and base+(count-1)*stride+offset+12 <= len(buffer), '顶点通道越界')
        return [finite(struct.unpack_from('<3f', buffer, base+i*stride+offset), 3) for i in range(count)]

    positions, normals = channel(0), channel(1)
    weights = bytes.fromhex(mesh['bone_weight_bytes_hex'])
    require(len(weights) == count*32, '四权重缓冲长度无效')
    records = bytearray()
    for position, record in zip(positions, struct.iter_unpack('<4f4i', weights)):
        values, indices = record[:4], record[4:]
        require(all(math.isfinite(x) and 0 <= x <= 1.0001 for x in values)
                and abs(sum(values)-1) < .001, '蒙皮权重无效')
        require(all(0 <= x < len(body['bones']) for x in indices), '蒙皮骨索引越界')
        records += struct.pack('<3f4f4I', *position, *values, *indices)
    submeshes = bytes.fromhex(mesh['submesh_bytes_hex'])
    indices = bytes.fromhex(mesh['opaque_index_bytes_hex'])
    require(mesh['index_format'] in (0, 1) and 0 < mesh['submesh_count'] <= 128
            and len(submeshes) == mesh['submesh_count']*64, '子网格布局无效')
    width, fmt = (2, 'H') if mesh['index_format'] == 0 else (4, 'I')
    triangles = []
    for ordinal in range(mesh['submesh_count']):
        offset, size, topology, base = struct.unpack_from('<4I', submeshes, ordinal*64+0x28)
        require(topology == 0 and size % 3 == 0 and offset % width == 0
                and offset+size*width <= len(indices), '三角形索引范围无效')
        require(len(triangles)+size <= 400000*3, '三角形数量无效')
        for (value,) in struct.iter_unpack('<'+fmt, indices[offset:offset+size*width]):
            require(value+base < count, '三角形顶点索引越界')
            triangles.append(value+base)
    require(0 < len(triangles)//3 <= 400000, '三角形数量无效')
    skin = struct.pack('<8sII', b'SSMTSKN3', count, len(body['bones'])) + records
    skin += struct.pack('<I', len(triangles)//3) + struct.pack('<'+'I'*len(triangles), *triangles)
    skin += b''.join(struct.pack('<3f', *normal) for normal in normals)
    return bytes(skin), {'vertices': count, 'triangles': len(triangles)//3, 'identity': identity}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', required=True, type=Path)
    parser.add_argument('--capture', type=Path)
    parser.add_argument('--output-prefix', required=True, type=Path)
    args = parser.parse_args()
    reference_raw = args.reference.read_bytes()
    reference = json.loads(reference_raw.decode('utf-8-sig'))
    avatar, binds, body = build_reference(reference)
    outputs = {'.avatar-fixture': avatar, '.body-fixture': binds}
    manifest = {'format': 1, 'reference_sha256': hashlib.sha256(reference_raw).hexdigest(), 'body_binds': len(body['bones'])}
    if args.capture:
        capture_raw = args.capture.read_bytes()
        outputs['.skin-fixture'], details = build_skin(reference, json.loads(capture_raw.decode('utf-8-sig')), body)
        manifest.update(details, capture_sha256=hashlib.sha256(capture_raw).hexdigest())
    manifest['outputs'] = {suffix: hashlib.sha256(value).hexdigest() for suffix, value in outputs.items()}
    outputs['.manifest.json'] = (json.dumps(manifest, ensure_ascii=False, indent=2)+'\n').encode('utf-8')
    targets = {Path(str(args.output_prefix)+suffix): value for suffix, value in outputs.items()}
    require(not any(path.resolve() in {args.reference.resolve(), args.capture.resolve() if args.capture else None}
                    for path in targets), '输出不能覆盖输入')
    # 先核验所有目标，避免发现冲突前写出一部分夹具；相同内容允许重复导出。
    for path, value in targets.items():
        require(not path.exists() or path.read_bytes() == value, f'拒绝覆盖不同内容的夹具：{path}')
    for path, value in targets.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            with path.open('xb') as output:
                output.write(value)
    print(json.dumps(manifest, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, struct.error) as error:
        print(f'导出失败：{error}', file=sys.stderr)
        sys.exit(1)
