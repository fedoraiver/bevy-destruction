"""Build, validate, and export a self-contained destructible building asset."""

import hashlib
import json
import math
from pathlib import Path
import struct
import tempfile
import uuid

import bpy
from mathutils import Matrix, Vector

from .bonds import disconnected_chunks, generate_bonds
from .natural import fracture_parts, SAMPLING_FACTOR, MAX_PROXIES_PER_CHUNK
from .geometry import generate_building
from .surfaces import classify_bond_surfaces
from .contact_export import correct_contacts, read_triangles

OWNER = "bevy_destruction_asset_tool"
VERSION = "0.3.0"
ROTATION = Matrix(((1, 0, 0), (0, 0, 1), (0, -1, 0)))


def _vector(value):
    return [_number(v) for v in ROTATION @ Vector(value)]


def _number(value):
    return float(format(float(value), ".12g"))


def _canonical(value):
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    )


def _geometry_digest(obj):
    materials = []
    for material in obj.data.materials:
        if material is None:
            materials.append(None)
            continue
        nodes = []
        links = []
        if material.use_nodes:
            for node in material.node_tree.nodes:
                inputs = []
                for socket in node.inputs:
                    if hasattr(socket, "default_value"):
                        value = socket.default_value
                        if not isinstance(value, (int, float, str, bool)):
                            value = list(value)
                        inputs.append((socket.identifier, value))
                nodes.append((node.type, inputs))
            links = [
                (
                    link.from_node.type,
                    link.from_socket.identifier,
                    link.to_node.type,
                    link.to_socket.identifier,
                )
                for link in material.node_tree.links
            ]
        materials.append(
            {
                "color": list(material.diffuse_color),
                "nodes": nodes,
                "links": links,
                "surface_role": material.get("bd_surface_role"),
                "material_id": material.get("bd_material_id"),
            }
        )
    geometry = {
        "vertices": [[_number(c) for c in v.co] for v in obj.data.vertices],
        "faces": [list(p.vertices) for p in obj.data.polygons],
        "face_materials": [p.material_index for p in obj.data.polygons],
        "materials": materials,
        "matrix": [[_number(c) for c in row] for row in obj.matrix_world],
    }
    return hashlib.sha256(_canonical(geometry).encode()).hexdigest()


def _remove_collection(collection):
    for child in list(collection.children):
        if child.name not in bpy.context.scene.collection.children:
            bpy.context.scene.collection.children.link(child)
        collection.children.unlink(child)
    for obj in list(collection.objects):
        if obj.get("bd_owner") != OWNER:
            if len(obj.users_collection) == 1:
                bpy.context.scene.collection.objects.link(obj)
            continue
        if len(obj.users_collection) > 1:
            collection.objects.unlink(obj)
            continue
        mesh = obj.data if obj.type == "MESH" else None
        bpy.data.objects.remove(obj, do_unlink=True)
        if mesh is not None and mesh.users == 0:
            bpy.data.meshes.remove(mesh)
    bpy.data.collections.remove(collection)


def _mark_owned(collection):
    for obj in collection.objects:
        obj["bd_owner"] = OWNER


def _f32(value):
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def _cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _patch_metrics(vertices, allow_edge_contact=False, edge_audit=None):
    import math

    vertices = list(dict.fromkeys(tuple(point) for point in vertices))
    if len(vertices) < 3:
        if allow_edge_contact:
            return None
        raise ValueError(
            "Baked patch has fewer than three distinct serialized vertices"
        )

    origin = vertices[0]
    shifted = [tuple(p[i] - origin[i] for i in range(3)) for p in vertices]
    raw = tuple(
        math.fsum(_cross(a, b)[i] for a, b in zip(shifted, shifted[1:] + shifted[:1]))
        for i in range(3)
    )
    length = math.sqrt(math.fsum(v * v for v in raw))
    if length <= 2e-12:
        if allow_edge_contact:
            return None
        raise ValueError("Baked patch collapsed after float32 serialization")
    normal = tuple(v / length for v in raw)
    # Clipping is convex before serialization. Rebuild its outline using a subset
    # of the actual float32 vertices: rounding can reverse microscopic edges.
    tangent = max(shifted, key=lambda p: sum(v * v for v in p))
    tangent_length = math.sqrt(sum(v * v for v in tangent))
    tangent = tuple(v / tangent_length for v in tangent)
    bitangent = _cross(normal, tangent)
    projected = {
        (
            sum(p[i] * tangent[i] for i in range(3)),
            sum(p[i] * bitangent[i] for i in range(3)),
        ): vertex
        for p, vertex in zip(shifted, vertices)
    }
    points = sorted(projected)
    outline = []
    for values in (points, list(reversed(points))):
        chain = []
        for point in values:
            while (
                len(chain) > 1
                and (
                    (chain[-1][0] - chain[-2][0]) * (point[1] - chain[-1][1])
                    - (chain[-1][1] - chain[-2][1]) * (point[0] - chain[-1][0])
                )
                <= 0
            ):
                chain.pop()
            chain.append(point)
        outline.extend(chain[:-1])
    vertices = [projected[p] for p in outline]
    if len(vertices) < 3:
        if allow_edge_contact:
            return None
        raise ValueError("Baked patch outline collapsed after float32 serialization")
    origin = vertices[0]
    shifted = [tuple(p[i] - origin[i] for i in range(3)) for p in vertices]
    raw = tuple(
        math.fsum(_cross(a, b)[i] for a, b in zip(shifted, shifted[1:] + shifted[:1]))
        for i in range(3)
    )
    length = math.sqrt(math.fsum(v * v for v in raw))
    normal = tuple(v / length for v in raw)
    triangles = []
    for a, b in zip(vertices[1:-1], vertices[2:]):
        cross = _cross(
            tuple(a[i] - origin[i] for i in range(3)),
            tuple(b[i] - origin[i] for i in range(3)),
        )
        area = math.fsum(cross[i] * normal[i] for i in range(3)) * 0.5
        if area > 0:
            triangles.append((area, (origin, a, b)))
    area = math.fsum(a for a, _ in triangles)
    center = tuple(
        math.fsum(a * sum(p[i] for p in tri) / 3 for a, tri in triangles) / area
        for i in range(3)
    )
    q = [
        [
            math.fsum(
                a
                * (
                    sum(p[i] - center[i] for p in tri)
                    * sum(p[j] - center[j] for p in tri)
                    + sum((p[i] - center[i]) * (p[j] - center[j]) for p in tri)
                )
                / 12
                for a, tri in triangles
            )
            for j in range(3)
        ]
        for i in range(3)
    ]
    # Match the consumer's double basis applied to the serialized float32 tensor.
    sign = math.copysign(1.0, normal[2])
    factor = -1.0 / (sign + normal[2])
    tangent = (
        normal[0] * normal[1] * factor,
        sign + normal[1] ** 2 * factor,
        -normal[1],
    )
    bitangent = _cross(normal, tangent)
    projected = [
        (
            math.fsum(p[i] * tangent[i] for i in range(3)),
            math.fsum(p[i] * bitangent[i] for i in range(3)),
        )
        for p in shifted
    ]
    from .bonds import _minimum_width

    width = _minimum_width(projected)
    if width <= 2e-5:
        if not allow_edge_contact:
            raise ValueError(
                f"Baked patch is edge-only contact below minimum bearing width (area={area:g}, width={width:g})"
            )
        if edge_audit is not None:
            edge_audit["edge_contact_area_m2"] += area
            edge_audit["maximum_edge_contact_width_m"] = max(
                edge_audit["maximum_edge_contact_width_m"], width
            )
        return None
    quantized_q = [[_f32(_number(value)) for value in row] for row in q]

    def quadratic(a, b):
        return sum(a[i] * quantized_q[i][j] * b[j] for i in range(3) for j in range(3))

    qt, qb, qtb = (
        quadratic(tangent, tangent),
        quadratic(bitangent, bitangent),
        quadratic(tangent, bitangent),
    )
    if qt <= 0 or qb <= 0 or qt * qb - qtb * qtb <= max(qt, qb) ** 2 * 1e-12:
        raise ValueError(
            f"Baked patch has unresolved float32 sectional moments (area={area:g}, width={width:g})"
        )
    return {
        "area": _number(area),
        "center": [_number(v) for v in center],
        "normal": [_number(v) for v in normal],
        "area_covariance": [[_number(v) for v in row] for row in q],
        "vertices": vertices,
    }


def _aggregate_patches(patches):
    import math

    area = math.fsum(p["area"] for p in patches)
    center = [
        math.fsum(p["area"] * p["center"][i] for p in patches) / area for i in range(3)
    ]
    normal = [math.fsum(p["area"] * p["normal"][i] for p in patches) for i in range(3)]
    length = math.sqrt(math.fsum(v * v for v in normal))
    if length <= area * 1e-8:
        raise ValueError("Baked interface normals nearly cancel")
    q = [
        [
            math.fsum(
                p["area_covariance"][i][j]
                + p["area"]
                * (p["center"][i] - center[i])
                * (p["center"][j] - center[j])
                for p in patches
            )
            for j in range(3)
        ]
        for i in range(3)
    ]
    return {
        "area": _number(area),
        "center": [_number(v) for v in center],
        "normal": [_number(v / length) for v in normal],
        "area_covariance": [[_number(v) for v in row] for row in q],
    }


def _manifest(spec, chunks, bonds, audit=None):
    materials = spec.to_dict()["materials"]
    anchors = []
    records = []
    for chunk in sorted(chunks, key=lambda item: item.id):
        density = materials[chunk.material]["density"]
        inertia = ROTATION @ Matrix(chunk.inertia) @ ROTATION.transposed()
        record = {
            "id": chunk.id,
            "part_id": chunk.part_id,
            "kind": chunk.kind,
            "material": chunk.material,
            "node": chunk.id,
            "position": _vector(chunk.center),
            "rotation": [0, 0, 0, 1],
            "volume": _number(chunk.volume),
            "mass": _number(chunk.volume * density),
            "center_of_mass": [0, 0, 0],
            "inertia": [
                [_number(inertia[i][j] * density) for j in range(3)] for i in range(3)
            ],
            "colliders": [
                {
                    "type": "convex_hull",
                    "vertices": [
                        _vector(v.co - Vector(chunk.center))
                        for v in collider.data.vertices
                    ],
                }
                for collider in chunk.collision_objects
            ],
        }
        records.append(record)
        if min(v.co.z for v in chunk.obj.data.vertices) <= 2e-5:
            anchors.append(
                {
                    "chunk": chunk.id,
                    "kind": "fixed",
                    "position": _vector((chunk.center[0], chunk.center[1], 0)),
                }
            )
    converted_bonds = []
    contact_audit = {
        "collapsed_or_edge_subfacets": 0,
        "edge_only_pairs": 0,
        "minimum_bearing_width_m": 2e-5,
        "edge_contact_area_m2": 0.0,
        "maximum_edge_contact_width_m": 0.0,
    }
    for bond in bonds:
        patches = []
        for polygon in bond["patch_vertices"]:
            patch = _patch_metrics(
                [[_f32(v) for v in _vector(point)] for point in polygon],
                allow_edge_contact=True,
                edge_audit=contact_audit,
            )
            if patch is None:
                contact_audit["collapsed_or_edge_subfacets"] += 1
            else:
                patches.append(patch)
        if not patches:
            contact_audit["edge_only_pairs"] += 1
            continue
        parent = _aggregate_patches(patches)
        converted_bonds.append(
            {
                "id": bond["id"],
                "a": bond["a"],
                "b": bond["b"],
                **parent,
                "patches": patches,
                "state": "rigid",
            }
        )
    unsupported = disconnected_chunks(
        chunks, converted_bonds, [a["chunk"] for a in anchors]
    )
    if unsupported:
        raise ValueError(
            f"{len(unsupported)} chunks have no bond path to the ground: {unsupported[:5]}"
        )
    manifest = {
        "schema": "bevy_destruction_asset",
        "schema_version": 2,
        "generator": {
            "name": OWNER,
            "version": VERSION,
            "surface_policy": "source_mesh_with_contact_overlays",
            "fracture": "shared_natural_profile_v2",
            "sampling_factor": SAMPLING_FACTOR,
            "max_proxies_per_chunk": MAX_PROXIES_PER_CHUNK,
        },
        "name": spec.name,
        "coordinates": {"units": "meters", "up": "+Y", "handedness": "right"},
        "specification": spec.to_dict(),
        "materials": materials,
        "chunks": records,
        "bonds": converted_bonds,
        "anchors": anchors,
    }
    if audit is not None:
        manifest["authoring_audit"] = audit
    manifest["contact_quantization_audit"] = contact_audit
    return _finalize_surface_audit(manifest, chunks)


def _finalize_surface_audit(manifest, chunks):
    surface_audits = [
        json.loads(chunk.obj.get("bd_surface_partition_audit", "{}"))
        for chunk in chunks
    ]
    manifest["surface_classification_audit"] = {
        "representation": "source_mesh_with_contact_overlays",
        "geometry_preserved_chunks": sum(
            bool(audit.get("source_geometry_preserved", False))
            for audit in surface_audits
        ),
        "uniform_interior_faces": sum(
            audit.get("uniform_interior_faces", 0) for audit in surface_audits
        ),
        "unmatched_geometric_edges": 0,
        "maximum_source_boundary_distance_m": max(
            (
                audit.get("maximum_original_vertex_surface_distance_m", 0.0)
                for audit in surface_audits
            ),
            default=0.0,
        ),
        "maximum_final_role_area_drift_m2": max(
            (
                audit.get("final_surface_role_area_drift_m2", 0.0)
                for audit in surface_audits
            ),
            default=0.0,
        ),
        "final_role_area_m2": [
            math.fsum(
                audit.get("final_surface_role_area_m2", [0.0, 0.0])[role]
                for audit in surface_audits
            )
            for role in range(2)
        ],
    }
    manifest.pop("asset_id", None)
    manifest["asset_id"] = hashlib.sha256(_canonical(manifest).encode()).hexdigest()
    return manifest


def _place_chunks(chunks):
    for chunk in chunks:
        for obj, role in [(chunk.obj, "chunk")] + [
            (collider, "collider") for collider in chunk.collision_objects
        ]:
            for vertex in obj.data.vertices:
                vertex.co -= Vector(chunk.center)
            obj.location = chunk.center
            obj["bd_owner"] = OWNER
            obj["bd_role"] = role
            obj["bd_chunk_id"] = chunk.id
            obj["bd_part_id"] = chunk.part_id
            obj.data.update()
        for collider in chunk.collision_objects:
            collider.hide_render = True
            collider.hide_set(True)
            collider.display_type = "WIRE"
    bpy.context.view_layer.update()
    for chunk in chunks:
        for obj in (chunk.obj, *chunk.collision_objects):
            obj["bd_geometry_digest"] = _geometry_digest(obj)


def _result(collection, manifest):
    return {
        "asset_id": manifest["asset_id"],
        "collection_name": collection.name,
        "chunk_count": len(manifest["chunks"]),
        "collider_count": sum(len(chunk["colliders"]) for chunk in manifest["chunks"]),
        "bond_count": len(manifest["bonds"]),
        "anchor_count": len(manifest["anchors"]),
        "warnings": [],
        "glb_path": None,
        "manifest_path": None,
    }


def build_asset(spec, *, output_dir=None, preview=True):
    if bpy.context.mode != "OBJECT":
        raise ValueError("Switch to Object Mode before generating a building")
    name = "BD::" + spec.name
    previous = next(
        (
            collection
            for collection in bpy.context.scene.collection.children_recursive
            if collection.get("bd_owner") == OWNER
            and collection.get("bd_asset_name") == spec.name
        ),
        None,
    )
    staging = bpy.data.collections.new("BD staging " + uuid.uuid4().hex[:8])
    staging["bd_owner"] = OWNER
    staging["bd_asset_name"] = spec.name
    bpy.context.scene.collection.children.link(staging)
    try:
        parts = generate_building(spec, staging)
        _mark_owned(staging)
        chunks = fracture_parts(
            parts,
            seed=spec.seed,
            target_size=spec.fracture_size,
            collection=staging,
            spec=spec,
        )
        _mark_owned(staging)
        bonds = generate_bonds(chunks, detailed=True)
        manifest = _manifest(
            spec, chunks, bonds, json.loads(staging["bd_natural_audit"])
        )
        classify_bond_surfaces(chunks, manifest["bonds"])
        _finalize_surface_audit(manifest, chunks)
        displays = {chunk.obj for chunk in chunks}
        colliders = {
            collider for chunk in chunks for collider in chunk.collision_objects
        }
        for obj in list(staging.objects):
            if obj not in displays | colliders:
                mesh = obj.data if obj.type == "MESH" else None
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh is not None and mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
        _place_chunks(chunks)
        staging["bd_manifest_json"] = _canonical(manifest)
        result = (
            export_asset(staging, output_dir)
            if output_dir is not None
            else _result(staging, manifest)
        )
    except Exception:
        _mark_owned(staging)
        _remove_collection(staging)
        raise
    if previous is not None:
        _remove_collection(previous)
    staging.name = name
    result["collection_name"] = staging.name
    if preview:
        for obj in bpy.context.selected_objects:
            obj.select_set(False)
        for chunk in chunks:
            chunk.obj.select_set(True)
        bpy.context.view_layer.objects.active = chunks[0].obj
    return result


def _normalize_glb(path, expected_ids):
    data = path.read_bytes()
    magic, version, length = struct.unpack_from("<III", data)
    if magic != 0x46546C67 or version != 2 or length != len(data):
        raise ValueError("Exporter did not produce a valid GLB 2.0 file")
    json_length, json_type = struct.unpack_from("<II", data, 12)
    if json_type != 0x4E4F534A:
        raise ValueError("GLB has no JSON chunk")
    document = json.loads(data[20 : 20 + json_length])
    found = set()
    for node in document.get("nodes", []):
        identifier = node.get("extras", {}).get("bd_chunk_id")
        if "mesh" in node and identifier not in expected_ids:
            raise ValueError("GLB contains a mesh outside this building")
        if identifier is not None:
            if identifier in found:
                raise ValueError("GLB contains a duplicate chunk ID")
            found.add(identifier)
            node["name"] = identifier
            document["meshes"][node["mesh"]]["name"] = identifier
    if found != expected_ids:
        raise ValueError("GLB chunk IDs do not match the structure manifest")
    payload = _canonical(document).encode()
    payload += b" " * (-len(payload) % 4)
    remaining = data[20 + json_length :]
    path.write_bytes(
        struct.pack("<III", magic, version, 20 + len(payload) + len(remaining))
        + struct.pack("<II", len(payload), json_type)
        + payload
        + remaining
    )


def _finalize_export_contacts(manifest, path):
    previous_audit = manifest.get("glb_contact_audit")
    audit = correct_contacts(
        manifest, read_triangles(path), _patch_metrics, _aggregate_patches
    )
    if previous_audit is not None and audit["corrected_patches"] == 0:
        # A second export revalidates the already corrected footprint while
        # retaining the original correction provenance and stable identity.
        manifest["glb_contact_audit"] = previous_audit
    manifest["generator"]["contact_export_policy"] = "actual_glb_footprint"
    manifest.pop("asset_id", None)
    structural = {
        key: value
        for key, value in manifest.items()
        if key not in {"mesh_file", "mesh_sha256"}
    }
    manifest["asset_id"] = hashlib.sha256(_canonical(structural).encode()).hexdigest()
    return manifest


def export_asset(collection, output_dir):
    if collection.get("bd_owner") != OWNER or "bd_manifest_json" not in collection:
        raise ValueError("Select a successfully generated building before exporting")
    manifest = json.loads(collection["bd_manifest_json"])
    objects = [obj for obj in collection.objects if obj.get("bd_role") == "chunk"]
    expected_ids = {chunk["id"] for chunk in manifest["chunks"]}
    colliders = [obj for obj in collection.objects if obj.get("bd_role") == "collider"]
    expected_colliders = sum(len(chunk["colliders"]) for chunk in manifest["chunks"])
    collider_counts = {
        identifier: sum(obj.get("bd_chunk_id") == identifier for obj in colliders)
        for identifier in expected_ids
    }
    if (
        len(objects) != len(expected_ids)
        or len(colliders) != expected_colliders
        or {obj.get("bd_chunk_id") for obj in objects} != expected_ids
        or {obj.get("bd_chunk_id") for obj in colliders} != expected_ids
        or any(
            collider_counts[chunk["id"]] != len(chunk["colliders"])
            for chunk in manifest["chunks"]
        )
    ):
        raise ValueError("Generated objects were changed; regenerate before exporting")
    bpy.context.view_layer.update()
    for obj in collection.objects:
        if obj.get("bd_role") in {"chunk", "collider"}:
            if (
                obj.get("bd_geometry_digest") != _geometry_digest(obj)
                or obj.modifiers
                or obj.data.shape_keys is not None
            ):
                raise ValueError(
                    f"Generated geometry {obj.name!r} was edited; regenerate before exporting"
                )
    target = Path(output_dir).expanduser().resolve()
    target.mkdir(parents=True, exist_ok=True)
    selected = list(bpy.context.selected_objects)
    active = bpy.context.view_layer.objects.active
    try:
        for obj in selected:
            obj.select_set(False)
        for obj in objects:
            obj.select_set(True)
        bpy.context.view_layer.objects.active = objects[0]
        with tempfile.TemporaryDirectory(prefix="bd_export_", dir=target) as directory:
            staging = Path(directory)
            glb = staging / "asset.glb"
            status = bpy.ops.export_scene.gltf(
                filepath=str(glb),
                export_format="GLB",
                use_selection=True,
                use_active_scene=True,
                export_yup=True,
                export_extras=True,
                export_animations=False,
                export_cameras=False,
                export_lights=False,
                export_apply=True,
            )
            if "FINISHED" not in status:
                raise ValueError("Blender glTF export was cancelled")
            _normalize_glb(glb, expected_ids)
            _finalize_export_contacts(manifest, glb)
            mesh_digest = hashlib.sha256(glb.read_bytes()).hexdigest()
            mesh_filename = manifest["name"] + "." + mesh_digest + ".glb"
            exported = {
                **manifest,
                "mesh_file": mesh_filename,
                "mesh_sha256": mesh_digest,
            }
            sidecar = staging / (manifest["name"] + ".destruction.json")
            sidecar.write_text(
                json.dumps(
                    exported,
                    indent=2,
                    sort_keys=True,
                    ensure_ascii=False,
                    allow_nan=False,
                )
                + "\n",
                encoding="utf-8",
            )
            final_glb = target / mesh_filename
            final_json = target / sidecar.name
            glb.replace(final_glb)
            sidecar.replace(final_json)
            collection["bd_manifest_json"] = _canonical(exported)
    finally:
        for obj in objects:
            obj.select_set(False)
        for obj in selected:
            if bpy.data.objects.get(obj.name) == obj:
                obj.select_set(True)
        if active is not None and bpy.data.objects.get(active.name) == active:
            bpy.context.view_layer.objects.active = active
    return {
        **_result(collection, manifest),
        "glb_path": str(final_glb),
        "manifest_path": str(final_json),
    }
