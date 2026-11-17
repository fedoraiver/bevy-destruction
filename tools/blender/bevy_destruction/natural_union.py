"""Boundary extraction and exact convex merging for nonoverlapping convex cells.

A cell is a list of outward-wound convex face polygons in Blender meters.
Inputs must cover their source domain without overlapping interiors. Partial
coplanar contacts are supported; edge/point contact never connects two cells.
"""

import collections
import heapq
import itertools
import math
import statistics

import bmesh


def _sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def _add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def _scale(a, k):
    return tuple(x * k for x in a)


def _dot(a, b):
    return math.fsum(x * y for x, y in zip(a, b))


def _cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _normal(face):
    origin = face[0]
    vectors = [_sub(p, origin) for p in face]
    crosses = [_cross(a, b) for a, b in zip(vectors, vectors[1:] + vectors[:1])]
    cross = tuple(math.fsum(c[i] for c in crosses) for i in range(3))
    length = math.sqrt(_dot(cross, cross))
    if length <= 1e-16:
        raise ValueError("Degenerate convex cell face")
    return _scale(cross, 1 / length)


def _frame(face):
    normal = _normal(face)
    axis = (1, 0, 0) if abs(normal[0]) < 0.8 else (0, 1, 0)
    tangent = _cross(normal, axis)
    tangent = _scale(tangent, 1 / math.sqrt(_dot(tangent, tangent)))
    return face[0], tangent, _cross(normal, tangent), normal


def _project(point, frame):
    delta = _sub(point, frame[0])
    return (_dot(delta, frame[1]), _dot(delta, frame[2]))


def _unproject(point, frame):
    return _add(frame[0], _add(_scale(frame[1], point[0]), _scale(frame[2], point[1])))


def _cross2(a, b):
    return a[0] * b[1] - a[1] * b[0]


def _area(poly):
    return 0.5 * math.fsum(_cross2(a, b) for a, b in zip(poly, poly[1:] + poly[:1]))


def _clean(poly, tolerance):
    result = []
    for point in poly:
        if not result or math.dist(point, result[-1]) > tolerance:
            result.append(tuple(point))
    if len(result) > 1 and math.dist(result[0], result[-1]) <= tolerance:
        result.pop()
    return result


def _clip(poly, a, b, inside=True):
    result = []
    sign = 1 if inside else -1
    edge = _sub(b, a)
    for p, q in zip(poly, poly[1:] + poly[:1]):
        dp = sign * _cross2(edge, _sub(p, a))
        dq = sign * _cross2(edge, _sub(q, a))
        if dp >= 0:
            result.append(p)
        if (dp >= 0) != (dq >= 0):
            t = dp / (dp - dq)
            result.append(_add(p, _scale(_sub(q, p), t)))
    return _clean(result, 1e-10)


def _intersection(a, b):
    if _area(b) < 0:
        b = list(reversed(b))
    for p, q in zip(b, b[1:] + b[:1]):
        a = _clip(a, p, q)
        if len(a) < 3:
            return []
    return a


def _subtract(poly, cutter, tolerance):
    if _area(cutter) < 0:
        cutter = list(reversed(cutter))
    result = []
    for a, b in zip(cutter, cutter[1:] + cutter[:1]):
        outside = _clip(poly, a, b, False)
        if len(outside) >= 3 and abs(_area(outside)) > tolerance**2 * 0.01:
            result.append(outside)
        poly = _clip(poly, a, b)
        if len(poly) < 3:
            break
    return result


def _width(poly):
    widths = []
    for a, b in zip(poly, poly[1:] + poly[:1]):
        edge = _sub(b, a)
        length = math.dist(a, b)
        if length > 1e-12:
            distances = [_cross2(edge, _sub(p, a)) / length for p in poly]
            widths.append(max(distances) - min(distances))
    return min(widths, default=0)


def _face_records(cells):
    records = []
    for cell_id, cell in enumerate(cells):
        for face_id, face in enumerate(cell):
            low = tuple(min(p[i] for p in face) for i in range(3))
            high = tuple(max(p[i] for p in face) for i in range(3))
            records.append((cell_id, face_id, face, low, high, _normal(face)))
    return records


def _face_contacts(cells, tolerance):
    records = _face_records(cells)
    if not records:
        return
    extent = max(
        max(r[4][i] for r in records) - min(r[3][i] for r in records) for i in range(3)
    )
    spacing = max(
        statistics.median(max(b - a for a, b in zip(r[3], r[4])) for r in records),
        extent / 64,
        tolerance * 16,
    )
    buckets = collections.defaultdict(list)
    for index, record in enumerate(records):
        cell, _, face, low, high, normal = record
        keys = list(
            itertools.product(
                *(
                    range(
                        math.floor((low[i] - tolerance) / spacing),
                        math.floor((high[i] + tolerance) / spacing) + 1,
                    )
                    for i in range(3)
                )
            )
        )
        candidates = set(itertools.chain.from_iterable(buckets[key] for key in keys))
        for other in sorted(candidates):
            previous = records[other]
            if previous[0] == cell or _dot(normal, previous[5]) > -1 + 1e-8:
                continue
            if any(
                previous[4][i] < low[i] - tolerance
                or high[i] < previous[3][i] - tolerance
                for i in range(3)
            ):
                continue
            if any(
                abs(_dot(normal, _sub(p, face[0]))) > tolerance for p in previous[2]
            ):
                continue
            frame = _frame(previous[2])
            polygon = _intersection(
                [_project(p, frame) for p in previous[2]],
                [_project(p, frame) for p in face],
            )
            if (
                len(polygon) >= 3
                and abs(_area(polygon)) > tolerance**2
                and _width(polygon) > tolerance
            ):
                yield other, index, [_unproject(p, frame) for p in polygon], records
        for key in keys:
            buckets[key].append(index)


def cell_contacts(cells, tolerance=2e-6):
    """Yield (i, j, overlap), winding from i to j, for finite face contacts."""
    return [
        (records[a][0], records[b][0], polygon)
        for a, b, polygon, records in _face_contacts(cells, tolerance)
    ]


def boundary_faces(cells, tolerance=2e-6):
    """Remove even partial interior subfaces, preserving all exterior material."""
    contacts = collections.defaultdict(list)
    for a, b, polygon, records in _face_contacts(cells, tolerance):
        contacts[(records[a][0], records[a][1])].append(polygon)
        contacts[(records[b][0], records[b][1])].append(polygon)
    result = []
    for cell_id, cell in enumerate(cells):
        for face_id, face in enumerate(cell):
            cutters = contacts[(cell_id, face_id)]
            if not cutters:
                result.append(list(face))
                continue
            frame = _frame(face)
            polygons = [[_project(p, frame) for p in face]]
            for cutter in cutters:
                projected = [_project(p, frame) for p in cutter]
                polygons = [
                    part
                    for poly in polygons
                    for part in _subtract(poly, projected, tolerance)
                ]
            result.extend([_unproject(p, frame) for p in poly] for poly in polygons)
    return result


def mesh_data(faces, tolerance=2e-6):
    """Weld shared vertices and split boundary edges at all T-junctions."""
    vertices, polygons = [], []
    buckets = collections.defaultdict(list)
    weld = tolerance * 0.25
    for face in faces:
        polygon = []
        for point in face:
            key = tuple(math.floor(p / weld) for p in point)
            candidates = itertools.chain.from_iterable(
                buckets[tuple(a + b for a, b in zip(key, offset))]
                for offset in itertools.product((-1, 0, 1), repeat=3)
            )
            index = next(
                (i for i in candidates if math.dist(vertices[i], point) <= weld), None
            )
            if index is None:
                index = len(vertices)
                vertices.append(tuple(point))
                buckets[key].append(index)
            if not polygon or polygon[-1] != index:
                polygon.append(index)
        if len(polygon) > 1 and polygon[0] == polygon[-1]:
            polygon.pop()
        if len(set(polygon)) >= 3:
            polygons.append(polygon)
    stitched = []
    for polygon in polygons:
        boundary = []
        for a, b in zip(polygon, polygon[1:] + polygon[:1]):
            start, finish = vertices[a], vertices[b]
            direction = _sub(finish, start)
            length2 = _dot(direction, direction)
            splits = [(0.0, a)]
            for i, point in enumerate(vertices):
                if i in (a, b) or any(
                    point[k] < min(start[k], finish[k]) - weld
                    or point[k] > max(start[k], finish[k]) + weld
                    for k in range(3)
                ):
                    continue
                t = _dot(_sub(point, start), direction) / length2
                if (
                    0 < t < 1
                    and math.dist(point, _add(start, _scale(direction, t))) <= weld
                ):
                    splits.append((t, i))
            boundary.extend(i for _, i in sorted(splits))
        if len(boundary) != len(set(boundary)):
            edges = collections.Counter()
            for a, b in zip(boundary, boundary[1:] + boundary[:1]):
                if edges[b, a]:
                    edges[b, a] -= 1
                else:
                    edges[a, b] += 1
            remaining = [
                (a, b) for (a, b), count in edges.items() for _ in range(count)
            ]
            if not remaining:
                continue
            links = dict(remaining)
            start = min(links)
            boundary = [start]
            while links.get(boundary[-1]) != start:
                following = links.get(boundary[-1])
                if following is None or following in boundary:
                    raise ValueError(
                        "Union boundary has a non-simple contour after welding"
                    )
                boundary.append(following)
            if len(boundary) != len(remaining):
                raise ValueError("Union face welding produced multiple contours")
        stitched.append(boundary)
    used = sorted({i for polygon in stitched for i in polygon})
    indices = {old: new for new, old in enumerate(used)}
    return [vertices[i] for i in used], [
        [indices[i] for i in face] for face in stitched
    ]


def boundary_mesh(cells, tolerance=2e-6):
    return mesh_data(boundary_faces(cells, tolerance), tolerance)


def volume(cell):
    origin = cell[0][0]
    return math.fsum(
        _dot(
            _sub(face[0], origin),
            _cross(_sub(face[i], origin), _sub(face[i + 1], origin)),
        )
        / 6
        for face in cell
        for i in range(1, len(face) - 1)
    )


def _hull(cells):
    bm = bmesh.new()
    try:
        points = sorted(set(tuple(p) for cell in cells for face in cell for p in face))
        origin = tuple(math.fsum(p[i] for p in points) / len(points) for i in range(3))
        scale = max(
            max(p[i] for p in points) - min(p[i] for p in points) for i in range(3)
        )
        if scale <= 0:
            raise ValueError("Convex hull has zero extent")
        originals = {}
        for point in points:
            originals[bm.verts.new(_scale(_sub(point, origin), 1 / scale))] = point
        result = bmesh.ops.convex_hull(
            bm, input=list(bm.verts), use_existing_faces=False
        )
        unused = {
            item
            for key in ("geom_unused", "geom_interior")
            for item in result.get(key, [])
            if isinstance(item, bmesh.types.BMVert)
        }
        if unused:
            bmesh.ops.delete(bm, geom=list(unused), context="VERTS")
        bm.normal_update()
        bmesh.ops.dissolve_limit(
            bm,
            angle_limit=1e-6,
            verts=list(bm.verts),
            edges=list(bm.edges),
            use_dissolve_boundaries=False,
        )
        # Hull topology selects input vertices; retain their canonical double coordinates.
        faces = [[originals[v] for v in face.verts] for face in bm.faces]
        if any(
            any(_dot(_normal(face), _sub(p, face[0])) > scale * 1e-10 for p in points)
            for face in faces
        ):
            return _precise_hull(points)
        return faces
    finally:
        bm.free()


def convex_hull(cells):
    """Compute hull topology near the origin, preserving supplied vertex coordinates."""
    return _hull(cells)


def _precise_hull(originals):
    """Incremental double-precision hull for near-coplanar float32 hull failures."""
    origin = originals[0]
    scale = max(math.dist(p, origin) for p in originals)
    points = [_scale(_sub(p, origin), 1 / scale) for p in originals]
    first = 0
    second = max(range(len(points)), key=lambda i: _dot(points[i], points[i]))
    line = points[second]
    third = max(
        range(len(points)),
        key=lambda i: _dot(_cross(line, points[i]), _cross(line, points[i])),
    )
    normal = _cross(line, points[third])
    fourth = max(range(len(points)), key=lambda i: abs(_dot(normal, points[i])))
    if abs(_dot(normal, points[fourth])) < 1e-15:
        raise ValueError("Convex hull is not a resolvable three-dimensional solid")
    tetra = (first, second, third, fourth)
    interior = tuple(math.fsum(points[i][axis] for i in tetra) / 4 for axis in range(3))

    def face(a, b, c):
        cross = _cross(_sub(points[b], points[a]), _sub(points[c], points[a]))
        length = math.sqrt(_dot(cross, cross))
        if length < 1e-14:
            return None
        normal = _scale(cross, 1 / length)
        if _dot(normal, _sub(interior, points[a])) > 0:
            b, c = c, b
            normal = _scale(normal, -1)
        return a, b, c, normal

    hull = [face(*indices) for indices in itertools.combinations(tetra, 3)]
    for index, point in enumerate(points):
        if index in tetra:
            continue
        visible = [f for f in hull if _dot(f[3], _sub(point, points[f[0]])) > 1e-12]
        if not visible:
            continue
        horizon = set()
        for a, b, c, _ in visible:
            for edge in ((a, b), (b, c), (c, a)):
                reverse = tuple(reversed(edge))
                if reverse in horizon:
                    horizon.remove(reverse)
                else:
                    horizon.add(edge)
        hull = [f for f in hull if f not in visible]
        hull.extend(
            f for a, b in sorted(horizon) if (f := face(a, b, index)) is not None
        )
    result = [[originals[i] for i in f[:3]] for f in hull]
    if any(any(_dot(f[3], _sub(p, points[f[0]])) > 1e-9 for p in points) for f in hull):
        raise ValueError("Double-precision convex hull has an unsupported face")
    return result


def merge_convex_cells(cells, tolerance=2e-6):
    """Merge face-adjacent cells only when their hull adds no measurable volume."""
    cells = list(cells)
    active = dict(enumerate(cells))
    volumes = {i: volume(cell) for i, cell in active.items()}
    neighbors = collections.defaultdict(set)
    areas = collections.defaultdict(float)
    for a, b, face in cell_contacts(cells, tolerance):
        neighbors[a].add(b)
        neighbors[b].add(a)
        frame = _frame(face)
        areas[a, b] += abs(_area([_project(p, frame) for p in face]))
    queue = [(-area, a, b) for (a, b), area in areas.items()]
    heapq.heapify(queue)
    serial = len(active)
    while queue:
        _, a, b = heapq.heappop(queue)
        if a not in active or b not in active:
            continue
        points = [p for cell in (active[a], active[b]) for face in cell for p in face]
        boundary = boundary_faces([active[a], active[b]], tolerance)
        convex = True
        for face in boundary:
            normal = _normal(face)
            if any(_dot(normal, _sub(p, face[0])) > tolerance * 0.25 for p in points):
                convex = False
                break
        if not convex:
            continue
        expected = volumes[a] + volumes[b]
        hull = _hull([active[a], active[b]])
        error = abs(volume(hull) - expected)
        # BMesh coordinates are float32; bound relative and absolute error.
        if error > max(expected * 3e-6, tolerance**3):
            continue
        adjacent = (neighbors.pop(a) | neighbors.pop(b)) - {a, b}
        del active[a], active[b]
        active[serial] = hull
        volumes[serial] = expected
        for other in sorted(adjacent):
            area = sum(areas.get(tuple(sorted((old, other))), 0.0) for old in (a, b))
            neighbors[other].difference_update((a, b))
            neighbors[other].add(serial)
            neighbors[serial].add(other)
            areas[other, serial] = area
            heapq.heappush(queue, (-area, other, serial))
        serial += 1
    return list(active.values())
