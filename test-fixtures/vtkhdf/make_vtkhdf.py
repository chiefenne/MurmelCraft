"""Write the VTKHDF test fixtures of MurmelCraft's field reader.

The files are written with h5py (BSD) on the HDF5 library, not with
MurmelCraft code, so the reader is checked against an independent writer.
The layout follows FoehnCraft's writer (crates/foehncraft-mesh/src/vtkhdf.rs,
FoehnCraft docs/interfaces.md, "Files of a run"): a
PartitionedDataSetCollection with the cells in block `internal`
(UnstructuredGrid; tetrahedra, hexahedra, prisms and pyramids as VTK's
linear cells, everything else as VTK_POLYHEDRON), one PolyData block per
boundary, groups that track and index the creation order of their links,
no object times, and the root attributes of a FoehnCraft run.

The fields are analytic functions of the cell centres, so the tests know
every value.

    python3 -I test-fixtures/vtkhdf/make_vtkhdf.py

writes channel_fields.vtkhdf, box_chunked.vtkhdf and box_latest.vtkhdf
beside this script.
"""

import math
import os
import sys
import xml.etree.ElementTree as ET

import h5py
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.dirname(HERE)

VTK_TETRA, VTK_HEXAHEDRON, VTK_WEDGE, VTK_PYRAMID, VTK_POLYHEDRON = 10, 12, 13, 14, 42


def read_vtu(path):
    """Points, volume cells (lists of faces) and boundary polygons of an
    ascii KoflCraft hand-off."""
    root = ET.parse(path).getroot()
    arrays = {}
    for sec in ("FieldData", "Points", "Cells", "CellData"):
        for el in root.iter(sec):
            for a in el.iter("DataArray"):
                arrays[(sec, a.get("Name", ""))] = a.text.split()
    pts = np.array(arrays[("Points", "")], dtype=float).reshape(-1, 3)
    conn = [int(v) for v in arrays[("Cells", "connectivity")]]
    offs = [int(v) for v in arrays[("Cells", "offsets")]]
    types = [int(v) for v in arrays[("Cells", "types")]]
    faces = [int(v) for v in arrays.get(("Cells", "faces"), [])]
    foffs = [int(v) for v in arrays.get(("Cells", "faceoffsets"), [])]
    region = [int(v) for v in arrays[("CellData", "region")]]

    def names(key):
        raw = bytes(int(v) for v in arrays.get(("FieldData", key), []))
        out = {}
        for s in raw.split(b"\0"):
            if s:
                i, n = s.decode().split("=", 1)
                out[int(i)] = n
        return out

    cells, cell_zone, bpolys, bpatch = [], [], [], []
    start, fstart = 0, 0
    for c, t in enumerate(types):
        nodes = conn[start:offs[c]]
        start = offs[c]
        if t == VTK_POLYHEDRON:
            stream = faces[fstart:foffs[c]]
            fstart = foffs[c]
            nf, k, fl = stream[0], 1, []
            for _ in range(nf):
                n = stream[k]
                fl.append(stream[k + 1:k + 1 + n])
                k += 1 + n
            cells.append(fl)
            cell_zone.append(region[c])
        elif t in (5, 7, 9):
            bpolys.append(nodes)
            bpatch.append(region[c])
        else:
            raise SystemExit(f"unexpected cell type {t}")
    return pts, cells, cell_zone, bpolys, bpatch, names("boundary_names")


def shape(faces):
    sizes = sorted(len(f) for f in faces)
    return {
        (3, 3, 3, 3): "tet",
        (3, 3, 3, 3, 4): "pyramid",
        (3, 3, 4, 4, 4): "prism",
        (4, 4, 4, 4, 4, 4): "hex",
    }.get(tuple(sizes), "poly")


def standard_cell(faces):
    """VTK type and node order of a tetrahedron, hexahedron, prism or
    pyramid given by outward faces, or None for a polyhedron. The first
    face (reversed, so its normal points into the cell) is the base; the
    other nodes follow along the side edges."""
    kind = shape(faces)
    if kind == "poly":
        return None

    def up(n, base):
        for f in faces:
            k = len(f)
            for i in range(k):
                if f[i] == n:
                    for m in (f[(i + 1) % k], f[(i + k - 1) % k]):
                        if m not in base:
                            return m
        raise SystemExit("standard cell without a side edge")

    if kind == "tet":
        t = faces[0]
        return VTK_TETRA, [t[0], t[2], t[1], up(t[0], t)]
    if kind == "hex":
        q = faces[0]
        base = [q[0], q[3], q[2], q[1]]
        return VTK_HEXAHEDRON, base + [up(n, base) for n in base]
    if kind == "prism":
        t = next(f for f in faces if len(f) == 3)
        base = [t[0], t[2], t[1]]
        return VTK_WEDGE, base + [up(n, base) for n in base]
    q = next(f for f in faces if len(f) == 4)
    base = [q[0], q[3], q[2], q[1]]
    return VTK_PYRAMID, base + [up(base[0], base)]


def group(parent, name):
    """A group that tracks and indexes the creation order of its links,
    without object times (as FoehnCraft's `group`)."""
    gcpl = h5py.h5p.create(h5py.h5p.GROUP_CREATE)
    gcpl.set_link_creation_order(h5py.h5p.CRT_ORDER_TRACKED | h5py.h5p.CRT_ORDER_INDEXED)
    gcpl.set_obj_track_times(False)
    return h5py.Group(h5py.h5g.create(parent.id, name.encode(), gcpl=gcpl))


def type_attr(g, t):
    g.attrs.create("Type", np.bytes_(t), dtype=h5py.string_dtype("ascii", len(t)))


def version_attr(g):
    g.attrs.create("Version", np.array([2, 4], dtype="<i8"))


class Writer:
    """How datasets are stored: contiguous (FoehnCraft), or chunked and
    compressed, or compact."""

    def __init__(self, chunks=None, compress=False, shuffle=False, special=None):
        self.chunks = chunks
        self.compress = compress
        self.shuffle = shuffle
        self.special = special or {}

    def ds(self, g, name, data):
        data = np.asarray(data)
        how = self.special.get(name)
        if how == "compact":
            space = h5py.h5s.create_simple(data.shape)
            dcpl = h5py.h5p.create(h5py.h5p.DATASET_CREATE)
            dcpl.set_layout(h5py.h5d.COMPACT)
            dcpl.set_obj_track_times(False)
            tid = h5py.h5t.py_create(data.dtype)
            dsid = h5py.h5d.create(g.id, name.encode(), tid, space, dcpl=dcpl)
            dsid.write(h5py.h5s.ALL, h5py.h5s.ALL, np.ascontiguousarray(data))
            return
        kw = {"track_times": False}
        if how == "single_chunk":
            kw.update(chunks=data.shape, compression="gzip", compression_opts=6)
        elif how == "big_endian":
            data = data.astype(">f8")
        elif self.chunks and data.size > 1:
            kw.update(chunks=(min(self.chunks, data.shape[0]),) + data.shape[1:])
            if self.compress:
                kw.update(compression="gzip", compression_opts=4)
            if self.shuffle:
                kw.update(shuffle=True)
        g.create_dataset(name, data=data, **kw)


def write(path, pts, cells, cell_zone, bpolys, bpatch, patch_names, fields, face_fields,
          attrs, writer, libver="earliest"):
    fcpl = h5py.h5p.create(h5py.h5p.FILE_CREATE)
    fcpl.set_obj_track_times(False)
    fapl = h5py.h5p.create(h5py.h5p.FILE_ACCESS)
    fapl.set_libver_bounds(
        h5py.h5f.LIBVER_EARLIEST if libver == "earliest" else h5py.h5f.LIBVER_LATEST,
        h5py.h5f.LIBVER_LATEST,
    )
    fid = h5py.h5f.create(path.encode(), h5py.h5f.ACC_TRUNC, fcpl=fcpl, fapl=fapl)
    f = h5py.File(fid)
    for name, value in attrs:
        if isinstance(value, str):
            f.attrs.create(name, value, dtype=h5py.string_dtype("utf-8"))
        elif isinstance(value, float):
            f.attrs.create(name, np.float64(value))
        else:
            f.attrs.create(name, np.uint64(value))
    root = group(f, "VTKHDF")
    type_attr(root, "PartitionedDataSetCollection")
    version_attr(root)

    internal = group(root, "internal")
    type_attr(internal, "UnstructuredGrid")
    version_attr(internal)
    types, offsets, conn = [], [0], []
    poly_offsets, poly_faces, face_offsets, face_conn = [0], [], [0], []
    for faces in cells:
        st = standard_cell(faces)
        if st:
            types.append(st[0])
            conn.extend(st[1])
        else:
            types.append(VTK_POLYHEDRON)
            seen = []
            for fc in faces:
                for n in fc:
                    if n not in seen:
                        seen.append(n)
            conn.extend(seen)
            for fc in faces:
                poly_faces.append(len(face_offsets) - 1)
                face_conn.extend(fc)
                face_offsets.append(len(face_conn))
        offsets.append(len(conn))
        poly_offsets.append(len(poly_faces))
    w = writer
    i64 = lambda v: np.array(v, dtype="<i8")
    w.ds(internal, "NumberOfPoints", i64([len(pts)]))
    w.ds(internal, "NumberOfCells", i64([len(cells)]))
    w.ds(internal, "NumberOfConnectivityIds", i64([len(conn)]))
    w.ds(internal, "Points", np.asarray(pts, dtype="<f8"))
    w.ds(internal, "Types", np.array(types, dtype="u1"))
    w.ds(internal, "Offsets", i64(offsets))
    w.ds(internal, "Connectivity", i64(conn))
    w.ds(internal, "NumberOfFaces", i64([len(face_offsets) - 1]))
    w.ds(internal, "NumberOfFaceConnectivityIds", i64([len(face_conn)]))
    w.ds(internal, "NumberOfPolyhedronToFaceIds", i64([len(poly_faces)]))
    w.ds(internal, "FaceOffsets", i64(face_offsets))
    w.ds(internal, "FaceConnectivity", i64(face_conn))
    w.ds(internal, "PolyhedronOffsets", i64(poly_offsets))
    w.ds(internal, "PolyhedronToFaces", i64(poly_faces))
    cd = group(internal, "CellData")
    for name, values in fields:
        w.ds(cd, name, np.asarray(values, dtype="<f8"))
    if not any(n == "zone" for n, _ in fields):
        w.ds(cd, "zone", np.asarray(cell_zone, dtype="<f8"))
    group(internal, "PointData")
    group(internal, "FieldData")
    internal.attrs.create("Index", np.int64(0))

    assembly = group(root, "Assembly")
    assembly["internal"] = h5py.SoftLink("/VTKHDF/internal")
    boundary = group(assembly, "boundary")
    for i in sorted(patch_names):
        name = patch_names[i]
        g = group(root, name)
        type_attr(g, "PolyData")
        version_attr(g)
        faces = [k for k, p in enumerate(bpatch) if p == i]
        pmap, ppts, pconn, poffs = {}, [], [], [0]
        for k in faces:
            for n in bpolys[k]:
                if n not in pmap:
                    pmap[n] = len(ppts)
                    ppts.append(pts[n])
                pconn.append(pmap[n])
            poffs.append(len(pconn))
        w.ds(g, "NumberOfPoints", i64([len(ppts)]))
        w.ds(g, "Points", np.asarray(ppts, dtype="<f8").reshape(-1, 3))
        for sub, n, o, c in (
            ("Vertices", 0, [0], []),
            ("Lines", 0, [0], []),
            ("Strips", 0, [0], []),
            ("Polygons", len(faces), poffs, pconn),
        ):
            sg = group(g, sub)
            w.ds(sg, "NumberOfCells", i64([n]))
            w.ds(sg, "NumberOfConnectivityIds", i64([len(c)]))
            w.ds(sg, "Offsets", i64(o))
            w.ds(sg, "Connectivity", i64(c))
        bcd = group(g, "CellData")
        w.ds(bcd, "boundary", np.full(len(faces), float(i)))
        for fname, values in face_fields:
            w.ds(bcd, fname, np.asarray([values[k] for k in faces], dtype="<f8"))
        group(g, "PointData")
        group(g, "FieldData")
        g.attrs.create("Index", np.int64(i + 1))
        boundary[name] = h5py.SoftLink(f"/VTKHDF/{name}")
    f.close()


def centres(pts, cells):
    out = []
    for faces in cells:
        nodes = sorted({n for fc in faces for n in fc})
        out.append(pts[nodes].mean(axis=0))
    return np.array(out)


def channel_fields(pts, cells, bpolys, bpatch, patch_names):
    """Velocity, speed, pressure and turbulence fields of a fully developed
    duct flow along x, analytic in the cell centres."""
    lo, hi = pts.min(axis=0), pts.max(axis=0)
    c = centres(pts, cells)
    xn = (c[:, 0] - lo[0]) / (hi[0] - lo[0])
    yn = (c[:, 1] - lo[1]) / (hi[1] - lo[1])
    zn = (c[:, 2] - lo[2]) / (hi[2] - lo[2])
    ux = 2.25 * 4 * yn * (1 - yn) * 4 * zn * (1 - zn)
    uy = 0.05 * np.sin(math.pi * xn)
    uz = np.zeros_like(ux)
    speed = np.sqrt(ux * ux + uy * uy + uz * uz)
    p = 50.0 - 100.0 * xn
    k = 0.01 + 0.1 * (1 - 4 * yn * (1 - yn))
    omega = 10.0 + 100.0 * (1 - 4 * zn * (1 - zn))
    nu_t = k / omega
    fields = [
        ("velocity_x", ux),
        ("velocity_y", uy),
        ("velocity_z", uz),
        ("speed", speed),
        ("pressure", p),
        ("k", k),
        ("omega", omega),
        ("nu_t", nu_t),
    ]
    walls = {i for i, n in patch_names.items() if "wall" in n}
    tau, yplus = [], []
    for k_, poly in enumerate(bpolys):
        fc = pts[poly].mean(axis=0)
        on_wall = bpatch[k_] in walls
        t = 0.02 * (1.0 + (fc[0] - lo[0]) / (hi[0] - lo[0])) if on_wall else 0.0
        tau.append(t)
        yplus.append(30.0 * t / 0.04 if on_wall else 0.0)
    return fields, [("wall_shear_stress", tau), ("y_plus", yplus)]


UNITS = (
    "coordinates m; velocity_x m/s; velocity_y m/s; velocity_z m/s; speed m/s; "
    "pressure Pa; velocity_error m/s; k m2/s2; omega 1/s; nu_t m2/s; "
    "turbulence_wall_distance m; modified_pressure Pa; wall_shear_stress Pa; y_plus 1"
)


def main():
    pts, cells, zone, bpolys, bpatch, names = read_vtu(
        os.path.join(FIXTURES, "channel", "foehncraft.vtu"))
    fields, face_fields = channel_fields(pts, cells, bpolys, bpatch, names)
    attrs = [
        ("foehncraft_format", "foehncraft-fields"),
        ("foehncraft_format_version", 1),
        ("foehncraft_version", "1.0.0"),
        ("case", "channel"),
        ("units", UNITS),
        ("iteration", 250),
    ]
    write(os.path.join(HERE, "channel_fields.vtkhdf"), pts, cells, zone, bpolys, bpatch,
          names, fields, face_fields, attrs, Writer())

    pts, cells, zone, bpolys, bpatch, names = read_vtu(
        os.path.join(FIXTURES, "box", "foehncraft.vtu"))
    c = centres(pts, cells)
    many = [("pressure", 100.0 * c[:, 0]), ("velocity_x", c[:, 1]),
            ("velocity_y", c[:, 2]), ("velocity_z", -c[:, 0])]
    many += [(f"s{i}", c[:, 0] + i) for i in range(10)]
    attrs_t = attrs[:4] + [("units", "pressure Pa; velocity_x m/s; velocity_y m/s; velocity_z m/s"),
                           ("step", 12), ("time", 0.25)]
    write(os.path.join(HERE, "box_chunked.vtkhdf"), pts, cells, zone, bpolys, bpatch, names,
          many, [], attrs_t, Writer(chunks=2, compress=True, shuffle=True))
    extra = attrs_t + [(f"note_{i}", f"attribute {i}") for i in range(6)]
    write(os.path.join(HERE, "box_latest.vtkhdf"), pts, cells, zone, bpolys, bpatch, names,
          many, [], extra,
          Writer(chunks=2, compress=True,
                 special={"s0": "single_chunk", "s1": "compact", "s2": "big_endian",
                          "Points": "compact"}),
          libver="latest")


if __name__ == "__main__":
    sys.exit(main())
