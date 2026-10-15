"""检查导出器对损坏输入的拒绝，以及新旧身份信息和索引格式兼容性。"""
import copy
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from export_static_fixtures import build_reference, build_skin


def fixture():
    matrix = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
    reference = {'complete': True, 'actor_name': 'Actor', 'animator_native': '0x10', 'rig': [
        {'index': 0, 'parent': -1, 'name': 'Actor', 'static_model_rotation': None},
        {'index': 1, 'parent': 0, 'name': 'Joint', 'static_reference': True,
         'static_local_position': [0, 0, 0], 'static_model_rotation': [0, 0, 0, 1]}],
        'meshes': [{'name': 'Body', 'mesh_native': '0x20', 'selected_body': True, 'valid': True,
                    'bones': [{'rig_index': 1, 'name': 'Joint', 'matrix_column_major': matrix}]}]}
    header = bytearray(0x1D8)
    header[0x10:0x18] = bytes((0, 0, 0, 3, 0, 12, 0, 3))
    header[0x3A] = 24
    vertices = b''.join(struct.pack('<6f', *p, 0, 0, 1) for p in ((0, 0, 0), (1, 0, 0), (0, 1, 0)))
    submesh = bytearray(64)
    struct.pack_into('<4I', submesh, 0x28, 0, 3, 0, 0)
    mesh = {'name': 'Body', 'mesh_native': '0x20', 'complete': True, 'vertex_count': 3,
            'bone_weight_count': 3, 'bone_weight_layout': '4xfloat32_4xint32',
            'shared_data_header_hex': header.hex(), 'cpu_vertex_bytes_hex': vertices.hex(),
            'vertex_bytes': len(vertices), 'bone_weight_bytes_hex': (struct.pack('<4f4i', 1, 0, 0, 0, 0, 0, 0, 0)*3).hex(),
            'index_format': 0, 'submesh_count': 1, 'submesh_bytes_hex': submesh.hex(),
            'opaque_index_bytes_hex': struct.pack('<3H', 0, 1, 2).hex()}
    return reference, {'complete': True, 'animator_native': '0x10', 'static_mesh_sources': [mesh]}


class StaticFixturesTests(unittest.TestCase):
    def test_valid_and_index_width_equivalence(self):
        reference, capture = fixture()
        avatar, body, mesh = build_reference(reference)
        self.assertEqual(struct.unpack_from('<I', avatar)[0], 2)
        self.assertEqual(len(body), 328)
        skin16, details = build_skin(reference, capture, mesh)
        self.assertEqual(details['triangles'], 1)
        capture['static_mesh_sources'][0].update(index_format=1, opaque_index_bytes_hex=struct.pack('<3I', 0, 1, 2).hex())
        self.assertEqual(build_skin(reference, capture, mesh)[0], skin16)

    def test_cycle_and_missing_static(self):
        reference, _ = fixture()
        reference['rig'][1]['parent'] = 1
        with self.assertRaisesRegex(ValueError, '父链'):
            build_reference(reference)
        reference['rig'][1].update(parent=0, static_reference=False)
        with self.assertRaisesRegex(ValueError, 'STATIC'):
            build_reference(reference)

    def test_identity_and_legacy_evidence(self):
        reference, capture = fixture()
        body = reference['meshes'][0]
        capture['animator_native'] = '0x30'
        with self.assertRaisesRegex(ValueError, 'Animator'):
            build_skin(reference, capture, body)
        capture['animator_native'] = '0x10'
        body['mesh_native'] = '0x30'
        with self.assertRaisesRegex(ValueError, '身份'):
            build_skin(reference, capture, body)
        del body['mesh_native']
        with self.assertRaisesRegex(ValueError, '交叉核验'):
            build_skin(reference, capture, body)
        capture['static_mesh_sources'][0]['source'] = 'EXTERNAL_READONLY_STATIC_BODY_BIND_CROSSCHECKED'
        self.assertEqual(build_skin(reference, capture, body)[1]['identity'], 'LEGACY_EXTERNAL_CROSSCHECK_DECLARED')

    def test_corrupt_weights_indices_and_ranges(self):
        reference, original = fixture()
        cases = [
            {'bone_weight_bytes_hex': (struct.pack('<4f4i', float('nan'), 0, 0, 0, 0, 0, 0, 0)*3).hex()},
            {'bone_weight_bytes_hex': (struct.pack('<4f4i', 1, 0, 0, 0, -1, 0, 0, 0)*3).hex()},
            {'opaque_index_bytes_hex': struct.pack('<3H', 0, 1, 3).hex()},
            {'opaque_index_bytes_hex': struct.pack('<H', 0).hex()},
            {'vertex_bytes': 12},
        ]
        for changed in cases:
            with self.subTest(fields=list(changed)):
                capture = copy.deepcopy(original)
                capture['static_mesh_sources'][0].update(changed)
                with self.assertRaises(ValueError):
                    build_skin(reference, capture, reference['meshes'][0])


if __name__ == '__main__':
    unittest.main()
