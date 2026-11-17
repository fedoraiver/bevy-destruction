"""Geometry exchanged between the authoring stages, in Blender meters/Z-up."""

from dataclasses import dataclass, field
from typing import Any


@dataclass
class Part:
    id: str
    kind: str
    material: str
    obj: Any


@dataclass
class Chunk:
    id: str
    part_id: str
    kind: str
    material: str
    obj: Any
    collider: Any
    volume: float
    center: tuple[float, float, float]
    inertia: list[list[float]]
    colliders: list[Any] = field(default_factory=list)

    @property
    def collision_objects(self):
        return self.colliders or [self.collider]
