"""Build, validate, and export a self-contained destructible building asset."""

import hashlib
import json
from pathlib import Path
import struct
import tempfile
import uuid

import bpy
from mathutils import Matrix, Vector

from .bonds import disconnected_chunks, generate_bonds
from .fracture import fracture_parts
from .geometry import generate_building


OWNER = "bevy_destruction_asset_tool"
VERSION = "0.1.0"
ROTATION = Matrix(((1, 0, 0), (0, 0, 1), (0, -1, 0)))


def _vector(value):
    return [_number(v) for v in ROTATION @ Vector(value)]


def _number(value):
    return float(format(float(value), ".12g"))


def _canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


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
            links = [(link.from_node.type, link.from_socket.identifier,
                      link.to_node.type, link.to_socket.identifier) for link in material.node_tree.links]
        materials.append({"color": list(material.diffuse_color), "nodes": nodes, "links": links})
    geometry = {"vertices": [[_number(c) for c in v.co] for v in obj.data.vertices],
                "faces": [list(p.vertices) for p in obj.data.polygons],
                "face_materials": [p.material_index for p in obj.data.polygons], "materials": materials,
                "matrix": [[_number(c) for c in row] for row in obj.matrix_world]}
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


def _manifest(spec, chunks, bonds):
    materials = spec.to_dict()["materials"]
    anchors = []
    records = []
    for chunk in sorted(chunks, key=lambda item: item.id):
        density = materials[chunk.material]["density"]
        inertia = ROTATION @ Matrix(chunk.inertia) @ ROTATION.transposed()
        record = {
            "id": chunk.id, "part_id": chunk.part_id, "kind": chunk.kind,
            "material": chunk.material, "node": chunk.id,
            "position": _vector(chunk.center), "rotation": [0, 0, 0, 1],
            "volume": _number(chunk.volume), "mass": _number(chunk.volume * density),
            "center_of_mass": [0, 0, 0],
            "inertia": [[_number(inertia[i][j] * density) for j in range(3)] for i in range(3)],
            "colliders": [{"type": "convex_hull", "vertices": [
                _vector(v.co - Vector(chunk.center)) for v in chunk.collider.data.vertices]}],
        }
        records.append(record)
        if min(v.co.z for v in chunk.obj.data.vertices) <= 2e-5:
            anchors.append({"chunk": chunk.id, "kind": "fixed",
                            "position": _vector((chunk.center[0], chunk.center[1], 0))})
    converted_bonds = []
    for bond in bonds:
        covariance = ROTATION @ Matrix(bond["area_covariance"]) @ ROTATION.transposed()
        converted_bonds.append({**bond, "area": _number(bond["area"]),
                                "center": _vector(bond["center"]), "normal": _vector(bond["normal"]),
                                "area_covariance": [[_number(v) for v in row] for row in covariance],
                                "state": "rigid"})
    unsupported = disconnected_chunks(chunks, bonds, [a["chunk"] for a in anchors])
    if unsupported:
        raise ValueError(f"{len(unsupported)} chunks have no bond path to the ground: {unsupported[:5]}")
    manifest = {
        "schema": "bevy_destruction_asset", "schema_version": 1,
        "generator": {"name": OWNER, "version": VERSION},
        "name": spec.name, "coordinates": {"units": "meters", "up": "+Y", "handedness": "right"},
        "specification": spec.to_dict(), "materials": materials,
        "chunks": records, "bonds": converted_bonds, "anchors": anchors,
    }
    manifest["asset_id"] = hashlib.sha256(_canonical(manifest).encode()).hexdigest()
    return manifest


def _place_chunks(chunks):
    for chunk in chunks:
        for obj, role in ((chunk.obj, "chunk"), (chunk.collider, "collider")):
            for vertex in obj.data.vertices:
                vertex.co -= Vector(chunk.center)
            obj.location = chunk.center
            obj["bd_owner"] = OWNER
            obj["bd_role"] = role
            obj["bd_chunk_id"] = chunk.id
            obj["bd_part_id"] = chunk.part_id
            obj.data.update()
        chunk.collider.hide_render = True
        chunk.collider.hide_set(True)
        chunk.collider.display_type = "WIRE"
    bpy.context.view_layer.update()
    for chunk in chunks:
        for obj in (chunk.obj, chunk.collider):
            obj["bd_geometry_digest"] = _geometry_digest(obj)


def _result(collection, manifest):
    return {"asset_id": manifest["asset_id"], "collection_name": collection.name,
            "chunk_count": len(manifest["chunks"]), "collider_count": len(manifest["chunks"]),
            "bond_count": len(manifest["bonds"]), "anchor_count": len(manifest["anchors"]),
            "warnings": [], "glb_path": None, "manifest_path": None}


def build_asset(spec, *, output_dir=None, preview=True):
    if bpy.context.mode != "OBJECT":
        raise ValueError("Switch to Object Mode before generating a building")
    name = "BD::" + spec.name
    previous = next((collection for collection in bpy.context.scene.collection.children_recursive
                     if collection.get("bd_owner") == OWNER and collection.get("bd_asset_name") == spec.name), None)
    staging = bpy.data.collections.new("BD staging " + uuid.uuid4().hex[:8])
    staging["bd_owner"] = OWNER
    staging["bd_asset_name"] = spec.name
    bpy.context.scene.collection.children.link(staging)
    try:
        parts = generate_building(spec, staging)
        _mark_owned(staging)
        chunks = fracture_parts(parts, seed=spec.seed, target_size=spec.fracture_size, collection=staging)
        _mark_owned(staging)
        bonds = generate_bonds(chunks)
        manifest = _manifest(spec, chunks, bonds)
        displays = {chunk.obj for chunk in chunks}
        colliders = {chunk.collider for chunk in chunks}
        for obj in list(staging.objects):
            if obj not in displays | colliders:
                mesh = obj.data if obj.type == "MESH" else None
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh is not None and mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
        _place_chunks(chunks)
        staging["bd_manifest_json"] = _canonical(manifest)
        result = export_asset(staging, output_dir) if output_dir is not None else _result(staging, manifest)
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
    document = json.loads(data[20:20 + json_length])
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
    remaining = data[20 + json_length:]
    path.write_bytes(struct.pack("<III", magic, version, 20 + len(payload) + len(remaining)) +
                     struct.pack("<II", len(payload), json_type) + payload + remaining)


def export_asset(collection, output_dir):
    if collection.get("bd_owner") != OWNER or "bd_manifest_json" not in collection:
        raise ValueError("Select a successfully generated building before exporting")
    manifest = json.loads(collection["bd_manifest_json"])
    objects = [obj for obj in collection.objects if obj.get("bd_role") == "chunk"]
    expected_ids = {chunk["id"] for chunk in manifest["chunks"]}
    colliders = [obj for obj in collection.objects if obj.get("bd_role") == "collider"]
    if (len(objects) != len(expected_ids) or len(colliders) != len(expected_ids) or
            {obj.get("bd_chunk_id") for obj in objects} != expected_ids or
            {obj.get("bd_chunk_id") for obj in colliders} != expected_ids):
        raise ValueError("Generated objects were changed; regenerate before exporting")
    bpy.context.view_layer.update()
    for obj in collection.objects:
        if obj.get("bd_role") in {"chunk", "collider"}:
            if (obj.get("bd_geometry_digest") != _geometry_digest(obj) or obj.modifiers or
                    obj.data.shape_keys is not None):
                raise ValueError(f"Generated geometry {obj.name!r} was edited; regenerate before exporting")
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
                filepath=str(glb), export_format="GLB", use_selection=True,
                use_active_scene=True,
                export_yup=True, export_extras=True, export_animations=False,
                export_cameras=False, export_lights=False, export_apply=True)
            if "FINISHED" not in status:
                raise ValueError("Blender glTF export was cancelled")
            _normalize_glb(glb, expected_ids)
            mesh_digest = hashlib.sha256(glb.read_bytes()).hexdigest()
            mesh_filename = manifest["name"] + "." + mesh_digest + ".glb"
            exported = {**manifest, "mesh_file": mesh_filename, "mesh_sha256": mesh_digest}
            sidecar = staging / (manifest["name"] + ".destruction.json")
            sidecar.write_text(json.dumps(exported, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
            final_glb = target / mesh_filename
            final_json = target / sidecar.name
            glb.replace(final_glb)
            sidecar.replace(final_json)
    finally:
        for obj in objects:
            obj.select_set(False)
        for obj in selected:
            if bpy.data.objects.get(obj.name) == obj:
                obj.select_set(True)
        if active is not None and bpy.data.objects.get(active.name) == active:
            bpy.context.view_layer.objects.active = active
    return {**_result(collection, manifest), "glb_path": str(final_glb), "manifest_path": str(final_json)}
