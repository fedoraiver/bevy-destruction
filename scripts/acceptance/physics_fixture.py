"""Prepare and observe physical scenes through native Bevy Remote Protocol."""

from contextlib import contextmanager
import json
import math
from pathlib import Path

from bevy_acceptance.driver import assert_that

from runtime import capture, capture_scene


ROOT = Path(__file__).resolve().parents[2]
GAME = "bevy_destruction"
IDENTITY = f"{GAME}::scene::ChunkIdentity"
BOND = f"{GAME}::scene::BondInstance"
CHUNK = f"{GAME}::scene::ChunkInstance"
REMOVED = f"{GAME}::scene::RemovedChunk"
CLUSTER = f"{GAME}::physics::Cluster"
REBUILD = f"{GAME}::physics::NeedsClusterRebuild"
STRESS = f"{GAME}::stress_runtime::StressState"
STRESS_SETTINGS = f"{GAME}::stress_runtime::StressSettings"
IMPACTS = f"{GAME}::projectiles::ImpactStats"
PROJECTILE = f"{GAME}::projectiles::RigidProjectile"
CHILD = "bevy_ecs::hierarchy::ChildOf"
VISIBILITY = "bevy_camera::visibility::Visibility"
POSITION = "avian3d::physics_transform::transform::Position"
ROTATION = "avian3d::physics_transform::transform::Rotation"
RIGID = "avian3d::dynamics::rigid_body::RigidBody"
LINEAR = "avian3d::dynamics::rigid_body::LinearVelocity"
ANGULAR = "avian3d::dynamics::rigid_body::AngularVelocity"
MASS = "avian3d::dynamics::rigid_body::mass_properties::components::computed::ComputedMass"
CENTER = "avian3d::dynamics::rigid_body::mass_properties::components::computed::ComputedCenterOfMass"
SLEEPING = "avian3d::dynamics::rigid_body::sleeping::Sleeping"
GRAVITY = "avian3d::dynamics::integrator::Gravity"
VIRTUAL_TIME = "bevy_time::time::Time<bevy_time::virt::Virtual>"


def component(game, entity, kind):
    return game.get(entity, kind)[kind]


def insert(game, entity, components):
    return game.call("world.insert_components", {"entity": entity, "components": components})


def mutate(game, entity, kind, path, value):
    return game.call("world.mutate_components",
                     {"entity": entity, "component": kind, "path": path, "value": value})


def mutate_resource(game, kind, path, value):
    return game.call("world.mutate_resources", {"resource": kind, "path": path, "value": value})


@contextmanager
def paused_physics(game):
    # The fixed schedules stop while native BRP requests and rendering continue.
    mutate_resource(game, VIRTUAL_TIME, "context.paused", True)
    try:
        yield
    finally:
        mutate_resource(game, VIRTUAL_TIME, "context.paused", False)


def building_geometry(game, root, manifest):
    chunks = {identity["id"]: row["entity"] for row in game.query(IDENTITY)
              if (identity := row["components"][IDENTITY])["building"] == root}
    entities = set(chunks.values())
    bonds = {bond["id"]: row["entity"] for row in game.query(BOND)
             if set((bond := row["components"][BOND])["endpoints"]) <= entities}
    assert_that(chunks.keys() == {chunk["id"] for chunk in manifest["chunks"]},
                "The loaded building does not match the fixture manifest")
    assert_that(bonds.keys() == {bond["id"] for bond in manifest["bonds"]},
                "The loaded building does not have the expected structural bonds")
    vertices = [rest_point(chunk, vertex) for chunk in manifest["chunks"]
                for collider in chunk["colliders"] for vertex in collider["vertices"]]
    axes = list(zip(*vertices, strict=True))
    return {"chunks": chunks, "bonds": bonds,
            "bounds": {"min": [min(axis) for axis in axes], "max": [max(axis) for axis in axes]}}


def load_manifest(scenario):
    return json.loads((ROOT / "assets" / scenario.asset_path).read_text(encoding="utf-8"))


def rest_point(chunk, point=None):
    local = chunk["center_of_mass"] if point is None else point
    return add(chunk["position"], rotate(chunk["rotation"], local))


def add(left, right):
    return [a + b for a, b in zip(left, right, strict=True)]


def scale(vector, factor):
    return [value * factor for value in vector]


def cross(left, right):
    x, y, z = left
    a, b, c = right
    return [y * c - z * b, z * a - x * c, x * b - y * a]


def rotate(rotation, vector):
    xyz = rotation[:3]
    uv = scale(cross(xyz, vector), 2.0)
    return add(vector, add(scale(uv, rotation[3]), cross(xyz, uv)))


def point_center(points):
    return [sum(axis) / len(points) for axis in zip(*points, strict=True)]


def close_mass(actual, expected, tolerance=1e-4):
    assert_that(math.isclose(actual, expected, rel_tol=tolerance),
                f"Expected {expected:.6g} kg, observed {actual:.6g} kg")


def chunk_mass(game, building, identifier):
    return component(game, building["chunks"][identifier], CHUNK)["mass_properties"]["mass"]


def mass_from(component_value):
    inverse = component_value["inverse"]
    assert_that(isinstance(inverse, (int, float)) and math.isfinite(inverse) and inverse > 0,
                f"Invalid inverse mass {inverse!r}")
    return 1.0 / inverse


def bodies(game, root):
    result = []
    for row in game.query(CLUSTER, CHILD, RIGID, POSITION, ROTATION, LINEAR, ANGULAR, MASS, CENTER):
        values = row["components"]
        if values[CHILD] != root:
            continue
        body = values[CLUSTER].copy()
        body.update(
            entity=row["entity"], kind=values[RIGID], position=values[POSITION],
            rotation=values[ROTATION], velocity=values[LINEAR], angular=values[ANGULAR],
            mass=mass_from(values[MASS]),
            center=add(values[POSITION], rotate(values[ROTATION], values[CENTER])),
        )
        assert_that(all(isinstance(value, (int, float)) and math.isfinite(value)
                        for key in ("position", "rotation", "velocity", "angular", "center")
                        for value in body[key]), f"Non-finite body state {body}")
        result.append(body)
    return result


def sleeping_entities(game):
    return {row["entity"] for row in game.query(with_=(CLUSTER, SLEEPING))}


def cut(game, root, building, manifest, *, removed=(), broken=()):
    removed = set(removed)
    broken = set(broken)
    for identifier in sorted(removed):
        entity = building["chunks"][identifier]
        mutate(game, entity, CHUNK, "alive", False)
        insert(game, entity, {REMOVED: {}, VISIBILITY: "Hidden"})
    for bond in manifest["bonds"]:
        if bond["id"] in broken or bond["a"] in removed or bond["b"] in removed:
            mutate(game, building["bonds"][bond["id"]], BOND, "active", False)
    insert(game, root, {REBUILD: {}})


def captures_at_rest(game, filename, *, require_scene=False):
    # Let the renderer present the requested stage without advancing physics.
    with paused_physics(game):
        return capture_scene(game, filename) if require_scene else capture(game, filename)


def expect_stress(game, root):
    def solved():
        value = component(game, root, STRESS)
        return value if value["solves"] else None

    return game.wait(solved, "a production self-weight solve")
