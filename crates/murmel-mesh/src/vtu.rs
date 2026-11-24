//! Reader for VTK XML UnstructuredGrid files (`.vtu`), written from the
//! VTK file-format documentation (VTK User's Guide, "VTK File Formats",
//! XML formats and the polyhedron cell layout with `faces` and
//! `faceoffsets`) and KoflCraft's `docs/foehncraft.md`.
//!
//! Supported: one `Piece`, `ascii` and uncompressed `binary` (base64)
//! arrays, little endian, cell types polyhedron (42), tetra (10),
//! hexahedron (12), wedge (13), pyramid (14) and the boundary polygons
//! triangle (5), polygon (7) and quad (9). The cell array `region` gives
//! the zone of a volume cell and the patch of a boundary face; the
//! field-data strings `zone_names`, `boundary_names` and `boundary_roles`
//! (`id=name`) name them. Appended and compressed data give `MESH007`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::field::{Field, Location, Results, group_vectors};
use crate::{CellKind, Mesh, MeshError, Patch, PatchRole, Polygons, boundary, numbers};

pub(crate) const POLYHEDRON: i64 = 42;
pub(crate) const TETRA: i64 = 10;
pub(crate) const HEXAHEDRON: i64 = 12;
pub(crate) const WEDGE: i64 = 13;
pub(crate) const PYRAMID: i64 = 14;
pub(crate) const TRIANGLE: i64 = 5;
pub(crate) const POLYGON: i64 = 7;
pub(crate) const QUAD: i64 = 9;

pub(crate) const TETRA_FACES: &[&[usize]] = &[&[0, 1, 3], &[1, 2, 3], &[2, 0, 3], &[0, 2, 1]];
pub(crate) const HEX_FACES: &[&[usize]] = &[
    &[0, 4, 7, 3],
    &[1, 2, 6, 5],
    &[0, 1, 5, 4],
    &[3, 7, 6, 2],
    &[0, 3, 2, 1],
    &[4, 5, 6, 7],
];
/// The faces of VTK's wedge, outward: its base (0, 1, 2) points
/// towards the top (3, 4, 5), as for the base of the tetrahedron,
/// hexahedron and pyramid (VTK 9 `vtkWedge` and its cell sizes; written
/// so by FoehnCraft and KoflCraft).
pub(crate) const WEDGE_FACES: &[&[usize]] = &[
    &[0, 2, 1],
    &[3, 4, 5],
    &[0, 1, 4, 3],
    &[1, 2, 5, 4],
    &[2, 0, 3, 5],
];
pub(crate) const PYRAMID_FACES: &[&[usize]] = &[
    &[0, 3, 2, 1],
    &[0, 1, 4],
    &[1, 2, 4],
    &[2, 3, 4],
    &[3, 0, 4],
];

fn err(code: &'static str, message: impl Into<String>) -> MeshError {
    MeshError::new(code, message)
}

mod cells;

/// The size of the chunks a `.vtu` file is read in.
const CHUNK: usize = 32 << 20;

/// Read a `.vtu` file.
pub fn read_vtu(path: &Path) -> Result<Mesh, MeshError> {
    read_vtu_results(path).map(|(m, _)| m)
}

/// Read a `.vtu` file with its data arrays as fields.
///
/// The file streams through in chunks: one thread reads ahead, another
/// takes the SHA-256 of what was read (the validity badge and session
/// files need it), and this one parses each array as it passes. The text
/// of the file is never held whole (a 10 M cell ascii `.vtu` is about
/// 12 GB), only the numbers it holds.
pub fn read_vtu_results(path: &Path) -> Result<(Mesh, Results), MeshError> {
    read_vtu_file(path, true)
}

/// As [`read_vtu_results`]; without `hash` no SHA-256 is taken
/// (`Mesh::file_blake3` stays empty).
pub(crate) fn read_vtu_file(path: &Path, hash: bool) -> Result<(Mesh, Results), MeshError> {
    if !path.is_file() {
        return Err(err(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let cannot = |e: std::io::Error| err("MESH001", format!("cannot read {}: {e}", path.display()));
    let file = File::open(path).map_err(cannot)?;
    let len = file.metadata().map_err(cannot)?.len();
    crate::profile::check()?;
    let (parsed, digest) = std::thread::scope(|s| {
        let (to_parse, chunks) = sync_channel(2);
        let (to_hash, hashed) = if hash {
            let (tx, rx) = sync_channel(2);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let digest = hashed.map(|rx| s.spawn(move || blake3_of(rx)));
        s.spawn(move || read_ahead(file, to_parse, to_hash));
        // The parser drops its end of the chunks when it returns, which
        // stops the reader early after a failure or a cancel.
        let parsed = parse(Chunks::new(chunks), len);
        (parsed, digest.map(|h| h.join().unwrap_or_default()))
    });
    let (mut mesh, polys, results) = parsed?;
    crate::profile::check()?;
    let _t = crate::profile::span("read.topology");
    polys.link(&mut mesh);
    mesh.file_blake3 = digest.into_iter().collect();
    Ok((mesh, results))
}

/// Read `file` in chunks of [`CHUNK`] bytes, each to the hasher (if any)
/// and then to the parser, until the end of the file or until the parser
/// stops listening (it failed or was cancelled).
fn read_ahead(
    mut file: File,
    parse: SyncSender<std::io::Result<Arc<Vec<u8>>>>,
    hash: Option<SyncSender<Arc<Vec<u8>>>>,
) {
    loop {
        let mut buf = Vec::with_capacity(CHUNK);
        match (&mut file).take(CHUNK as u64).read_to_end(&mut buf) {
            Ok(0) => return,
            Ok(_) => {
                let chunk = Arc::new(buf);
                let hashed = hash.as_ref().is_none_or(|h| h.send(chunk.clone()).is_ok());
                if !hashed || parse.send(Ok(chunk)).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = parse.send(Err(e));
                return;
            }
        }
    }
}

/// The BLAKE3 hash (hex) of the chunks of a file, in their order.
fn blake3_of(chunks: Receiver<Arc<Vec<u8>>>) -> String {
    let _t = crate::profile::span("read.blake3");
    let mut h = crate::FileHash::Blake3.hasher();
    for c in chunks {
        h.update(c.as_slice());
    }
    h.finish()
}

/// The file as the parser reads it: the chunks [`read_ahead`] sends, and
/// the end of the file once it has sent the last.
struct Chunks {
    rx: Receiver<std::io::Result<Arc<Vec<u8>>>>,
    cur: Arc<Vec<u8>>,
    pos: usize,
}

impl Chunks {
    fn new(rx: Receiver<std::io::Result<Arc<Vec<u8>>>>) -> Chunks {
        Chunks {
            rx,
            cur: Arc::default(),
            pos: 0,
        }
    }
}

impl Read for Chunks {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let b = self.fill_buf()?;
        let n = b.len().min(out.len());
        out[..n].copy_from_slice(&b[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Chunks {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.pos >= self.cur.len() {
            self.pos = 0;
            self.cur = match self.rx.recv() {
                Ok(chunk) => chunk?,
                // The reader is done: the end of the file.
                Err(_) => Arc::default(),
            };
        }
        Ok(self.cur.get(self.pos..).unwrap_or(&[]))
    }

    fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.cur.len());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Other,
    FieldData,
    Points,
    Cells,
    CellData,
    PointData,
}

/// A data array of the file, with its values when the reader uses them.
#[derive(Debug)]
struct Array {
    section: Section,
    name: String,
    ty: String,
    components: usize,
    format: String,
    values: Values,
}

/// The values of an array, in the type its use needs: coordinates and
/// fields as `f64`, point ids and face streams as `u32`, offsets, types,
/// zones and strings as `i64`.
#[derive(Debug, Default)]
enum Values {
    /// Not used by the reader, passed over unparsed.
    #[default]
    Skipped,
    Floats(Vec<f64>),
    /// The points, grouped by three once read.
    Points(Vec<[f64; 3]>),
    Ints(Vec<i64>),
    Ids(Vec<u32>),
}

/// What an array is parsed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Skip,
    Floats,
    Ints,
    Ids,
}

impl Array {
    fn floats(&mut self) -> Vec<f64> {
        match std::mem::take(&mut self.values) {
            Values::Floats(v) => v,
            _ => Vec::new(),
        }
    }

    fn ints(&mut self) -> Vec<i64> {
        match std::mem::take(&mut self.values) {
            Values::Ints(v) => v,
            _ => Vec::new(),
        }
    }

    fn ids(&mut self) -> Vec<u32> {
        match std::mem::take(&mut self.values) {
            Values::Ids(v) => v,
            _ => Vec::new(),
        }
    }
}

struct Header {
    points: usize,
    cells: usize,
}

fn attr(e: &BytesStart<'_>, key: &str) -> Result<Option<String>, MeshError> {
    match e.try_get_attribute(key) {
        Ok(Some(a)) => a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map(|v| Some(v.into_owned()))
            .map_err(|x| err("MESH003", format!("bad attribute {key}: {x}"))),
        Ok(None) => Ok(None),
        Err(x) => Err(err("MESH003", format!("bad attribute {key}: {x}"))),
    }
}

fn count_attr(e: &BytesStart<'_>, key: &str) -> Result<usize, MeshError> {
    let v = attr(e, key)?.ok_or_else(|| err("MESH003", format!("Piece has no {key}")))?;
    v.trim()
        .parse()
        .map_err(|_| err("MESH003", format!("{key}=\"{v}\" is not a count")))
}

/// Read a `.vtu` from its text.
pub fn read_vtu_str(text: &str) -> Result<Mesh, MeshError> {
    read_vtu_str_results(text).map(|(m, _)| m)
}

/// Read a `.vtu` from its text, with its data arrays as fields.
pub fn read_vtu_str_results(text: &str) -> Result<(Mesh, Results), MeshError> {
    let (mut mesh, polys, results) = parse(text.as_bytes(), text.len() as u64)?;
    polys.link(&mut mesh);
    Ok((mesh, results))
}

fn xml_error(e: quick_xml::Error) -> MeshError {
    match e {
        quick_xml::Error::Io(x) => io_error(&x),
        e => err("MESH003", format!("XML error: {e}")),
    }
}

fn io_error(e: &std::io::Error) -> MeshError {
    err("MESH001", format!("cannot read the file: {e}"))
}

/// The mesh of a `.vtu` read from `src` (`len` bytes long) before its face
/// topology (`boundary::link`), with the faces of its cells as listed, and
/// its data arrays. The markup goes through the XML parser; the text of
/// each array is parsed as it streams past (see [`values`]).
fn parse<R: BufRead>(src: R, len: u64) -> Result<(Mesh, CellFaces, Results), MeshError> {
    let stream = crate::profile::span("read.stream");
    let len = usize::try_from(len).unwrap_or(usize::MAX);
    let mut reader = Reader::from_reader(src);
    let mut buf = Vec::new();
    let mut section = Section::Other;
    let mut header: Option<Header> = None;
    let mut saw_file = false;
    let mut header64 = false;
    // Elements opened and not yet closed: a file cut short ends inside one.
    let mut depth = 0usize;
    let mut arrays: Vec<Array> = Vec::new();
    loop {
        buf.clear();
        let ev = reader.read_event_into(&mut buf).map_err(xml_error)?;
        if matches!(ev, Event::Start(_)) {
            depth += 1;
        }
        match ev {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "VTKFile" => {
                saw_file = true;
                let ty = attr(&e, "type")?.unwrap_or_default();
                if ty != "UnstructuredGrid" {
                    return Err(err(
                        "MESH002",
                        format!("VTKFile type is \"{ty}\", not UnstructuredGrid"),
                    ));
                }
                if attr(&e, "compressor")?.is_some_and(|c| !c.is_empty()) {
                    return Err(err("MESH007", "compressed data is not supported"));
                }
                if attr(&e, "byte_order")?.is_some_and(|b| b == "BigEndian") {
                    return Err(err("MESH007", "big-endian binary data is not supported"));
                }
                header64 = attr(&e, "header_type")?.is_some_and(|h| h == "UInt64");
            }
            Event::Start(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "Piece" => piece(&e, &mut header, len)?,
                    "FieldData" => section = Section::FieldData,
                    "Points" => section = Section::Points,
                    "Cells" => section = Section::Cells,
                    "CellData" => section = Section::CellData,
                    "PointData" => section = Section::PointData,
                    "AppendedData" => {
                        return Err(err("MESH007", "appended data is not supported"));
                    }
                    "DataArray" => {
                        let mut a = array(&e, section)?;
                        let kind = kind_of(&a, &arrays);
                        let room = Room {
                            declared: declared(&a, header.as_ref(), len),
                            rest: len.saturating_sub(
                                usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX),
                            ),
                        };
                        a.values = values(&mut reader.stream(), &a, header64, kind, room)?;
                        arrays.push(a);
                        crate::profile::check()?;
                    }
                    _ => {}
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == "Piece" => {
                piece(&e, &mut header, len)?;
            }
            Event::Empty(e) if e.local_name().as_ref() == "DataArray" => {
                let mut a = array(&e, section)?;
                a.values = match kind_of(&a, &arrays) {
                    Kind::Skip => Values::Skipped,
                    Kind::Floats => Values::Floats(Vec::new()),
                    Kind::Ints => Values::Ints(Vec::new()),
                    Kind::Ids => Values::Ids(Vec::new()),
                };
                arrays.push(a);
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                match e.local_name().as_ref() {
                    "FieldData" | "Points" | "Cells" | "CellData" | "PointData" => {
                        section = Section::Other;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if !saw_file {
        return Err(err("MESH002", "no VTKFile element"));
    }
    if depth > 0 {
        return Err(err(
            "MESH003",
            "the file ends before its XML elements close (cut short?)",
        ));
    }
    let header = header.ok_or_else(|| err("MESH003", "no Piece element"))?;
    drop(stream);
    let (mesh, polys, slots) = assemble(&header, &mut arrays)?;
    let fields = data_fields(&header, &mut arrays, &slots, &mesh)?;
    Ok((
        mesh,
        polys,
        Results {
            fields,
            ..Results::default()
        },
    ))
}

/// What the reader parses an array into, by where it is and its name:
/// the first array of `Points`; the cell arrays `connectivity`, `offsets`,
/// `types`, `faces`, `faceoffsets` and `region` (the first of each name);
/// the data arrays that become fields; the strings of the field data.
/// The rest is passed over.
fn kind_of(a: &Array, before: &[Array]) -> Kind {
    let first = || {
        !before
            .iter()
            .any(|b| b.section == a.section && b.name == a.name)
    };
    match a.section {
        Section::Points if !before.iter().any(|b| b.section == Section::Points) => Kind::Floats,
        Section::Cells if first() => match a.name.as_str() {
            "connectivity" | "faces" => Kind::Ids,
            "offsets" | "types" | "faceoffsets" => Kind::Ints,
            _ => Kind::Skip,
        },
        Section::CellData if a.name == "region" => {
            if first() {
                Kind::Ints
            } else {
                Kind::Skip
            }
        }
        Section::CellData | Section::PointData if is_field(a) => Kind::Floats,
        Section::FieldData => match a.name.as_str() {
            "zone_names" | "boundary_names" | "boundary_roles" => {
                if first() {
                    Kind::Ints
                } else {
                    Kind::Skip
                }
            }
            _ if a.ty == "String" => Kind::Ints,
            _ => Kind::Skip,
        },
        _ => Kind::Skip,
    }
}

/// A cell or point data array that becomes a field (`region` names zones
/// and patches instead).
fn is_field(a: &Array) -> bool {
    matches!(a.section, Section::CellData | Section::PointData)
        && a.name != "region"
        && !a.name.is_empty()
        && element_size(&a.ty).is_some()
        && a.ty != "String"
}

/// The values of the array whose start tag was just read: its text up to
/// the `<` that ends it, parsed into `kind` as it streams in (into `room`),
/// or passed over. The first array of `Points` is grouped by three: ascii
/// coordinates parse straight into points.
fn values<R: BufRead>(
    src: &mut R,
    a: &Array,
    header64: bool,
    kind: Kind,
    room: Room,
) -> Result<Values, MeshError> {
    let name = &a.name;
    let not_number = |t: &str| err("MESH005", format!("{name}: \"{t}\" is not a number"));
    Ok(match kind {
        Kind::Skip => {
            content(src, None, |_| Ok(()))?;
            Values::Skipped
        }
        Kind::Floats if a.section == Section::Points && a.format != "binary" => {
            match ascii::<R, [f64; 3]>(src, room, not_number)? {
                Ok(p) => Values::Points(p),
                Err(flat) => Values::Floats(flat),
            }
        }
        Kind::Floats => {
            let v = numbers_of(src, a, header64, room, f64_of, not_number)?;
            if a.section == Section::Points && v.len() % 3 == 0 {
                Values::Points(v.as_chunks::<3>().0.to_vec())
            } else {
                Values::Floats(v)
            }
        }
        Kind::Ints => Values::Ints(numbers_of(src, a, header64, room, i64_of, |t| {
            err("MESH005", format!("{name}: \"{t}\" is not an integer"))
        })?),
        // Point ids and counts: whole numbers from 0 to `u32::MAX`, kept
        // in 4 bytes each instead of 8. A negative or larger value is out
        // of range (`MESH006`), and in binary becomes `u32::MAX`, which no
        // index check passes.
        Kind::Ids => Values::Ids(numbers_of(src, a, header64, room, u32_of, |t| {
            if <i64 as numbers::Number>::parse(t.as_bytes()).is_some() {
                err("MESH006", format!("{name}: {t} is out of range"))
            } else {
                err("MESH005", format!("{name}: \"{t}\" is not an integer"))
            }
        })?),
    })
}

/// What an ascii array is parsed into: room for the values the `Piece`
/// declares, or, for the connectivity and face streams (no count given),
/// for as many as the rest of the file holds at the density of their
/// first megabyte. Room never touched costs no memory, and room too small
/// grows (see [`numbers::Filling`]).
#[derive(Clone, Copy)]
struct Room {
    declared: Option<usize>,
    /// Bytes of the file from the array on.
    rest: usize,
}

/// The values an array should hold by the counts of the `Piece`, at most
/// one for every two bytes of the file: a wrong count cannot claim more
/// memory than the numbers the file could hold.
fn declared(a: &Array, h: Option<&Header>, len: usize) -> Option<usize> {
    let h = h?;
    let n = match (a.section, a.name.as_str()) {
        (Section::Points | Section::PointData, _) => h.points,
        (Section::Cells, "offsets" | "types" | "faceoffsets") | (Section::CellData, _) => h.cells,
        _ => return None,
    };
    Some(n.saturating_mul(a.components).min(len.div_ceil(2)))
}

/// The ascii numbers of an array as slots of `S`, parsed in parallel piece
/// by piece as the text streams in, into `room`; every value read when
/// they do not fill whole slots. `bad` names a token that is not a number.
fn ascii<R: BufRead, S: numbers::Slot>(
    src: &mut R,
    room: Room,
    bad: impl Fn(&str) -> MeshError,
) -> Result<Result<Vec<S>, Vec<S::Value>>, MeshError> {
    let mut out = numbers::Filling::new(room.declared.unwrap_or(0));
    // The density is sampled from the first piece long enough to tell.
    let mut guess = room.declared.is_none();
    content(src, Some(&numbers::SPACE), |text| {
        if guess && text.len() >= 1 << 16 {
            guess = false;
            out.expect(numbers::estimate(text, &numbers::SPACE, room.rest));
        }
        numbers::parse_append(text, &numbers::SPACE, &mut out).map_err(|t| bad(&t))
    })?;
    Ok(out.finish())
}

/// How a binary value of `T` is read from its little-endian bytes.
type Converter<T> = fn(&[u8]) -> T;

/// The numbers of an array as `T`: ascii tokens parsed in parallel piece
/// by piece (see [`ascii`]), or base64 decoded and converted by the
/// function its type selects (`convert`). `bad` names a token that is not
/// a `T`.
fn numbers_of<R: BufRead, T: numbers::Number + numbers::Slot<Value = T>>(
    src: &mut R,
    a: &Array,
    header64: bool,
    room: Room,
    convert: fn(&str) -> Option<Converter<T>>,
    bad: impl Fn(&str) -> MeshError,
) -> Result<Vec<T>, MeshError> {
    if a.format != "binary" {
        return Ok(ascii::<R, T>(src, room, bad)?.unwrap_or_else(|v| v));
    }
    let mut out = Vec::new();
    let type_error = || err("MESH005", format!("{}: type {}", a.name, a.ty));
    let mut b = Base64 {
        name: &a.name,
        header: if header64 { 8 } else { 4 },
        size: element_size(&a.ty).ok_or_else(type_error)?,
        convert: convert(&a.ty).ok_or_else(type_error)?,
        group: Vec::new(),
        bytes: Vec::new(),
        decoded: 0,
        padded: false,
        data: None,
    };
    content(src, None, |text| b.eat(text, &mut out))?;
    b.finish()?;
    Ok(out)
}

/// Hand the text of an array, from here to the `<` that ends it (left
/// unread), to `eat` piece by piece as the file streams in. With `sep`,
/// every piece ends at a separator, so no number is cut in two; without,
/// the pieces end anywhere.
fn content<R: BufRead>(
    src: &mut R,
    sep: Option<&numbers::Sep>,
    mut eat: impl FnMut(&[u8]) -> Result<(), MeshError>,
) -> Result<(), MeshError> {
    // The start of a number cut by the end of the last piece.
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let buf = src.fill_buf().map_err(|e| io_error(&e))?;
        let end = memchr::memchr(b'<', buf);
        let n = end.unwrap_or(buf.len());
        let last = end.is_some() || buf.is_empty();
        let text = &buf[..n];
        match sep {
            Some(sep) => {
                let mut from = 0;
                if !carry.is_empty() {
                    let k = if last {
                        n
                    } else {
                        text.iter().position(|&c| sep.has(c)).unwrap_or(n)
                    };
                    carry.extend_from_slice(&text[..k]);
                    from = k;
                    if k < n || last {
                        eat(&carry)?;
                        carry.clear();
                    }
                }
                let rest = &text[from..];
                let whole = if last {
                    rest.len()
                } else {
                    rest.iter().rposition(|&c| sep.has(c)).map_or(0, |i| i + 1)
                };
                eat(&rest[..whole])?;
                carry.extend_from_slice(&rest[whole..]);
            }
            None => eat(text)?,
        }
        src.consume(n);
        if last {
            return Ok(());
        }
        crate::profile::check()?;
    }
}

/// A base64 array decoded as it streams in: whole groups of four
/// characters decode in parallel, and the bytes after the length header
/// (`header` bytes, written with the data as one run) convert to values
/// of `size` bytes each.
struct Base64<'a, T> {
    name: &'a str,
    header: usize,
    size: usize,
    convert: fn(&[u8]) -> T,
    /// Characters of a group not yet complete (fewer than four).
    group: Vec<u8>,
    /// Decoded bytes not yet converted.
    bytes: Vec<u8>,
    /// Bytes decoded so far, the header included.
    decoded: usize,
    /// A group with padding was decoded: it must be the last.
    padded: bool,
    /// The data bytes the header announced, once read.
    data: Option<usize>,
}

impl<T: Send> Base64<'_, T> {
    fn fail(&self, what: impl std::fmt::Display) -> MeshError {
        err("MESH005", format!("{}: {what}", self.name))
    }

    fn eat(&mut self, text: &[u8], out: &mut Vec<T>) -> Result<(), MeshError> {
        // Whitespace (rare inside the run) is dropped first.
        let cleaned: Vec<u8>;
        let mut text = if text.iter().fold(false, |s, &c| s | (c <= b' ')) {
            cleaned = text
                .iter()
                .copied()
                .filter(|&c| !numbers::SPACE.has(c))
                .collect();
            cleaned.as_slice()
        } else {
            text
        };
        if !self.group.is_empty() {
            let k = (4 - self.group.len()).min(text.len());
            self.group.extend_from_slice(&text[..k]);
            text = &text[k..];
            if self.group.len() == 4 {
                let g = std::mem::take(&mut self.group);
                self.decode(&g)?;
            }
        }
        let whole = text.len() / 4 * 4;
        self.decode(&text[..whole])?;
        self.group.extend_from_slice(&text[whole..]);
        self.convert_into(out);
        Ok(())
    }

    fn decode(&mut self, b: &[u8]) -> Result<(), MeshError> {
        if b.is_empty() {
            return Ok(());
        }
        if self.padded {
            return Err(self.fail("bad base64: padding inside the data"));
        }
        let wrote = numbers::base64_append(b, &mut self.bytes)
            .map_err(|e| self.fail(format_args!("bad base64: {e}")))?;
        self.padded = wrote < b.len() / 4 * 3;
        self.decoded += wrote;
        Ok(())
    }

    /// Convert the whole values decoded so far; the bytes past the data
    /// the header announced are dropped.
    fn convert_into(&mut self, out: &mut Vec<T>) {
        if self.data.is_none() && self.bytes.len() >= self.header {
            let mut n = [0u8; 8];
            n[..self.header].copy_from_slice(&self.bytes[..self.header]);
            self.data = Some(usize::try_from(u64::from_le_bytes(n)).unwrap_or(usize::MAX));
            self.bytes.drain(..self.header);
        }
        let Some(data) = self.data else { return };
        let done = out.len() * self.size;
        let left = data.saturating_sub(done);
        let usable = self.bytes.len().min(left) / self.size * self.size;
        out.extend(numbers::le(&self.bytes[..usable], self.size, self.convert));
        self.bytes.drain(..usable);
        // What is left over waits for the rest of a value, at most.
        self.bytes.truncate(left - usable);
    }

    /// The checks once the whole run is decoded: the header was there,
    /// the data it announced was, and it holds whole values.
    fn finish(&self) -> Result<(), MeshError> {
        if !self.group.is_empty() {
            return Err(self.fail("bad base64: the characters are not a multiple of 4"));
        }
        let Some(data) = self.data else {
            return Err(self.fail("binary data too short"));
        };
        let present = self.decoded - self.header;
        if present < data {
            return Err(self.fail(format_args!("header says {data} bytes, {present} present")));
        }
        if data % self.size != 0 {
            return Err(self.fail("ragged binary data"));
        }
        Ok(())
    }
}

/// What a VTU cell became: a volume cell or a boundary polygon.
#[derive(Clone, Copy)]
enum Slot {
    Volume,
    Boundary,
}

/// The numeric cell and point arrays (except `region`) as fields: the
/// values of the volume cells, or of the boundary polygons when the file
/// has no volume cells; point arrays on the points.
fn data_fields(
    h: &Header,
    arrays: &mut [Array],
    slots: &[Slot],
    mesh: &Mesh,
) -> Result<Vec<Field>, MeshError> {
    let mut out = Vec::new();
    for a in arrays.iter_mut().filter(|a| is_field(a)) {
        let location = match a.section {
            Section::PointData => Location::Point,
            _ if mesh.cell_count() > 0 => Location::Cell,
            _ => Location::Boundary,
        };
        let k = a.components.max(1);
        let v = a.floats();
        let n = if location == Location::Point {
            h.points
        } else {
            h.cells
        };
        if v.len() != n * k {
            return Err(err(
                "MESH005",
                format!("{} has {} values for {n} entities of {k}", a.name, v.len()),
            ));
        }
        let values: Vec<f64> = if location == Location::Point {
            v
        } else {
            let want = |s: Slot| match location {
                Location::Cell => matches!(s, Slot::Volume),
                _ => matches!(s, Slot::Boundary),
            };
            slots
                .iter()
                .enumerate()
                .filter(|(_, s)| want(**s))
                .flat_map(|(c, _)| v[c * k..c * k + k].iter().copied())
                .collect()
        };
        if k == 1 || k == 3 {
            out.push(Field {
                name: a.name.clone(),
                unit: None,
                location,
                components: k,
                values,
            });
        } else {
            for j in 0..k {
                out.push(Field {
                    name: format!("{}_{j}", a.name),
                    unit: None,
                    location,
                    components: 1,
                    values: values.iter().skip(j).step_by(k).copied().collect(),
                });
            }
        }
    }
    Ok(group_vectors(out))
}

fn piece(e: &BytesStart<'_>, header: &mut Option<Header>, len: usize) -> Result<(), MeshError> {
    if header.is_some() {
        return Err(err("MESH012", "the file has more than one Piece"));
    }
    let points = count_attr(e, "NumberOfPoints")?;
    let cells = count_attr(e, "NumberOfCells")?;
    // Every coordinate takes at least two bytes ("0 ") and every cell at
    // least one type and one offset.
    if points.saturating_mul(6) > len || cells.saturating_mul(2) > len {
        return Err(err(
            "MESH004",
            format!("{points} points and {cells} cells cannot fit in {len} bytes"),
        ));
    }
    *header = Some(Header { points, cells });
    Ok(())
}

fn array(e: &BytesStart<'_>, section: Section) -> Result<Array, MeshError> {
    let components = match attr(e, "NumberOfComponents")? {
        Some(c) => c
            .trim()
            .parse()
            .map_err(|_| err("MESH003", format!("NumberOfComponents=\"{c}\"")))?,
        None => 1,
    };
    let format = attr(e, "format")?.unwrap_or_else(|| "ascii".into());
    if format == "appended" {
        return Err(err("MESH007", "appended data is not supported"));
    }
    Ok(Array {
        section,
        name: attr(e, "Name")?.unwrap_or_default(),
        ty: attr(e, "type")?.unwrap_or_default(),
        components,
        format,
        values: Values::Skipped,
    })
}

fn find<'a>(arrays: &'a mut [Array], section: Section, name: &str) -> Option<&'a mut Array> {
    arrays
        .iter_mut()
        .find(|a| a.section == section && a.name == name)
}

fn element_size(ty: &str) -> Option<usize> {
    Some(match ty {
        "Int8" | "UInt8" | "String" | "Char" => 1,
        "Int16" | "UInt16" => 2,
        "Int32" | "UInt32" | "Float32" => 4,
        "Int64" | "UInt64" | "Float64" => 8,
        _ => return None,
    })
}

fn le<const N: usize>(c: &[u8]) -> [u8; N] {
    let mut b = [0u8; N];
    b.copy_from_slice(&c[..N]);
    b
}

/// The converter of a binary type to `f64`.
fn f64_of(ty: &str) -> Option<fn(&[u8]) -> f64> {
    let f: fn(&[u8]) -> f64 = match ty {
        "Float32" => |c| f64::from(f32::from_le_bytes(le(c))),
        "Float64" => |c| f64::from_le_bytes(le(c)),
        "Int8" => |c| f64::from(c[0] as i8),
        "UInt8" | "String" | "Char" => |c| f64::from(c[0]),
        "Int16" => |c| f64::from(i16::from_le_bytes(le(c))),
        "UInt16" => |c| f64::from(u16::from_le_bytes(le(c))),
        "Int32" => |c| f64::from(i32::from_le_bytes(le(c))),
        "UInt32" => |c| f64::from(u32::from_le_bytes(le(c))),
        "Int64" => |c| i64::from_le_bytes(le(c)) as f64,
        "UInt64" => |c| u64::from_le_bytes(le(c)) as f64,
        _ => return None,
    };
    Some(f)
}

/// The converter of a binary type to `i64`.
fn i64_of(ty: &str) -> Option<fn(&[u8]) -> i64> {
    let f: fn(&[u8]) -> i64 = match ty {
        "Int8" => |c| i64::from(c[0] as i8),
        "UInt8" | "String" | "Char" => |c| i64::from(c[0]),
        "Int16" => |c| i64::from(i16::from_le_bytes(le(c))),
        "UInt16" => |c| i64::from(u16::from_le_bytes(le(c))),
        "Int32" => |c| i64::from(i32::from_le_bytes(le(c))),
        "UInt32" => |c| i64::from(u32::from_le_bytes(le(c))),
        "Int64" => |c| i64::from_le_bytes(le(c)),
        "UInt64" => |c| u64::from_le_bytes(le(c)) as i64,
        "Float32" => |c| f32::from_le_bytes(le(c)) as i64,
        "Float64" => |c| f64::from_le_bytes(le(c)) as i64,
        _ => return None,
    };
    Some(f)
}

/// The converter of a binary type to a point id or count (`u32::MAX`
/// when the value does not fit).
fn u32_of(ty: &str) -> Option<fn(&[u8]) -> u32> {
    let f: fn(&[u8]) -> u32 = match ty {
        "Int8" => |c| u32::try_from(c[0] as i8).unwrap_or(u32::MAX),
        "UInt8" => |c| u32::from(c[0]),
        "Int16" => |c| u32::try_from(i16::from_le_bytes(le(c))).unwrap_or(u32::MAX),
        "UInt16" => |c| u32::from(u16::from_le_bytes(le(c))),
        "Int32" => |c| u32::try_from(i32::from_le_bytes(le(c))).unwrap_or(u32::MAX),
        "UInt32" => |c| u32::from_le_bytes(le(c)),
        "Int64" => |c| u32::try_from(i64::from_le_bytes(le(c))).unwrap_or(u32::MAX),
        "UInt64" => |c| u32::try_from(u64::from_le_bytes(le(c))).unwrap_or(u32::MAX),
        _ => return None,
    };
    Some(f)
}

/// A string array: bytes, each string ended by a zero byte.
fn strings(a: &Array) -> Result<Vec<String>, MeshError> {
    let ints: &[i64] = match &a.values {
        Values::Ints(v) => v,
        _ => &[],
    };
    let bytes: Vec<u8> = ints
        .iter()
        .map(|&v| u8::try_from(v).map_err(|_| err("MESH005", format!("{}: byte {v}", a.name))))
        .collect::<Result<_, _>>()?;
    Ok(bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

/// `id=name` entries into a table indexed by id.
pub(crate) fn id_table(entries: &[String]) -> BTreeMap<usize, String> {
    let mut t = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        match e.split_once('=') {
            Some((id, name)) if id.trim().parse::<usize>().is_ok() => {
                if let Ok(id) = id.trim().parse::<usize>() {
                    t.insert(id, name.to_string());
                }
            }
            _ => {
                t.insert(i, e.clone());
            }
        }
    }
    t
}

/// `MESH006` when a point id of `ids` is not below `points`.
fn points_in_range(ids: &[u32], points: usize) -> Result<(), MeshError> {
    match ids.iter().find(|&&v| v as usize >= points) {
        Some(v) => Err(err(
            "MESH006",
            format!("point {v} is out of range 0..{points}"),
        )),
        None => Ok(()),
    }
}

/// The faces of the cells as the reader leaves them for linking.
enum CellFaces {
    Packed(Polygons),
    InStream(cells::InStream),
}

impl CellFaces {
    /// Link the faces into `mesh`'s face topology.
    fn link(self, mesh: &mut Mesh) {
        match self {
            CellFaces::Packed(p) => boundary::link(mesh, p),
            CellFaces::InStream(s) => boundary::link(mesh, s),
        }
    }
}

fn assemble(h: &Header, arrays: &mut [Array]) -> Result<(Mesh, CellFaces, Vec<Slot>), MeshError> {
    let mut mesh = Mesh::default();
    let mut slots = Vec::with_capacity(h.cells);

    let pts = arrays
        .iter_mut()
        .find(|a| a.section == Section::Points)
        .ok_or_else(|| err("MESH003", "no Points array"))?;
    if pts.components != 3 {
        return Err(err("MESH005", "Points must have 3 components"));
    }
    mesh.points = match std::mem::take(&mut pts.values) {
        Values::Points(p) if p.len() == h.points => p,
        v => {
            let n = match v {
                Values::Points(p) => 3 * p.len(),
                Values::Floats(f) => f.len(),
                _ => 0,
            };
            return Err(err(
                "MESH005",
                format!("Points has {n} values, {} expected", 3 * h.points),
            ));
        }
    };

    fn need<'a>(arrays: &'a mut [Array], name: &str) -> Result<&'a mut Array, MeshError> {
        find(arrays, Section::Cells, name)
            .ok_or_else(|| err("MESH003", format!("Cells has no {name} array")))
    }
    let conn = need(arrays, "connectivity")?.ids();
    let offsets = need(arrays, "offsets")?.ints();
    let types = need(arrays, "types")?.ints();
    if offsets.len() != h.cells || types.len() != h.cells {
        return Err(err(
            "MESH005",
            format!(
                "{} offsets and {} types for {} cells",
                offsets.len(),
                types.len(),
                h.cells
            ),
        ));
    }
    let stream = find(arrays, Section::Cells, "faces").map_or_else(Vec::new, Array::ids);
    let face_offsets =
        find(arrays, Section::Cells, "faceoffsets").map_or_else(Vec::new, Array::ints);
    let region = match find(arrays, Section::CellData, "region") {
        Some(a) => {
            let r = a.ints();
            if r.len() != h.cells {
                return Err(err(
                    "MESH005",
                    format!("region has {} values for {} cells", r.len(), h.cells),
                ));
            }
            Some(r)
        }
        None => None,
    };

    crate::profile::check()?;
    let _cells = crate::profile::span("read.cells");
    let read = cells::read(cells::Input {
        connectivity: &conn,
        offsets: &offsets,
        types: &types,
        stream: &stream,
        face_offsets: &face_offsets,
        region: region.as_deref(),
        points: mesh.points.len(),
    })?;
    drop((conn, face_offsets, region));
    slots.extend(types.iter().map(|&t| {
        if matches!(t, TRIANGLE | POLYGON | QUAD) {
            Slot::Boundary
        } else {
            Slot::Volume
        }
    }));
    let polys = match read.faces {
        cells::Faces::Packed(p) => {
            drop(stream);
            CellFaces::Packed(p)
        }
        cells::Faces::InStream(at) => CellFaces::InStream(cells::InStream { stream, at }),
    };
    mesh.cell_faces = read.cell_faces;
    mesh.cell_zone = read.cell_zone;
    mesh.cell_kind = read.cell_kind;
    mesh.boundary = read.boundary;
    mesh.boundary_patch = read.boundary_patch;
    let (zone_max, patch_max) = (read.zone_max, read.patch_max);

    // Names and roles from the field data.
    let field = |arrays: &mut [Array], name: &str| -> Result<Vec<String>, MeshError> {
        match find(arrays, Section::FieldData, name) {
            Some(a) => strings(a),
            None => Ok(Vec::new()),
        }
    };
    let zone_names = id_table(&field(arrays, "zone_names")?);
    let patch_names = id_table(&field(arrays, "boundary_names")?);
    let patch_roles = id_table(&field(arrays, "boundary_roles")?);
    let zones = zone_max
        .map_or(0, |m| m as usize + 1)
        .max(zone_names.keys().next_back().map_or(0, |k| k + 1));
    mesh.zones = (0..zones)
        .map(|i| {
            zone_names.get(&i).cloned().unwrap_or_else(|| {
                if zones == 1 {
                    "cells".into()
                } else {
                    format!("zone_{i}")
                }
            })
        })
        .collect();
    let patches = patch_max
        .map_or(0, |m| m as usize + 1)
        .max(patch_names.keys().next_back().map_or(0, |k| k + 1));
    mesh.patches = (0..patches)
        .map(|i| Patch {
            name: patch_names
                .get(&i)
                .cloned()
                .unwrap_or_else(|| format!("patch_{i}")),
            role: patch_roles
                .get(&i)
                .map_or(PatchRole::Unspecified, |r| PatchRole::parse(r)),
        })
        .collect();
    for a in arrays.iter().filter(|a| a.section == Section::FieldData) {
        if matches!(
            a.name.as_str(),
            "zone_names" | "boundary_names" | "boundary_roles"
        ) {
            continue;
        }
        if a.ty == "String"
            && let Some(v) = strings(a)?.into_iter().next()
        {
            mesh.metadata.insert(a.name.clone(), v);
        }
    }

    if mesh.cell_count() == 0 && mesh.boundary.is_empty() {
        return Err(err("MESH010", "no volume cells and no boundary faces"));
    }
    Ok((mesh, polys, slots))
}

/// Close a cell whose faces end at cell face `faces`.
pub(crate) fn push_cell(
    mesh: &mut Mesh,
    faces: usize,
    zone: u32,
    kind: CellKind,
    zone_max: &mut Option<u32>,
) {
    mesh.cell_faces.push(faces as u32);
    mesh.cell_zone.push(zone);
    mesh.cell_kind.push(kind);
    *zone_max = Some(zone_max.map_or(zone, |m| m.max(zone)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NO_CELL;

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    /// Invariant: the box hand-off (4 Voronoi cells, 6 named patches) reads
    /// with every cell, face, name and role. A reader that dropped the
    /// boundary polygons or the field data would change these counts.
    #[test]
    fn reads_the_koflcraft_box() {
        let m = read_vtu(&fixture("box/foehncraft.vtu")).unwrap();
        assert_eq!(m.cell_count(), 4);
        assert_eq!(m.zones, vec!["fluid".to_string()]);
        let names: Vec<_> = m.patches.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["xmin", "xmax", "ymin", "ymax", "zmin", "zmax"]);
        assert!(!m.boundary_extracted);
        assert!(m.boundary.len() >= 6);
        assert_eq!(
            m.metadata.get("koflcraft_version").map(String::as_str),
            Some("0.1.0")
        );
        let b = m.bounds();
        assert_eq!(b.min, [0.0; 3]);
        assert_eq!(b.max, [1.0; 3]);
    }

    /// Invariant: the straight channel's prism layers are classed as prisms
    /// and its patch faces and roles match the evidence KoflCraft wrote
    /// beside it (776 cells; inlet 85, outlet 85, walls 256 faces).
    #[test]
    fn reads_the_channel_with_roles_and_prisms() {
        let m = read_vtu(&fixture("channel/foehncraft.vtu")).unwrap();
        let s = m.summary();
        assert_eq!(s.cells, 776);
        assert_eq!(s.boundary_faces, 426);
        let p: Vec<_> = s
            .patches
            .iter()
            .map(|p| (p.name.as_str(), p.role, p.faces))
            .collect();
        assert_eq!(
            p,
            [
                ("inlet", PatchRole::Inlet, 85),
                ("outlet", PatchRole::Outlet, 85),
                ("walls", PatchRole::Wall, 256)
            ]
        );
        assert!(s.cell_kinds.get("prism").copied().unwrap_or(0) > 0);
        assert_eq!(s.cell_kinds.values().sum::<usize>(), 776);
    }

    /// Invariant: every boundary polygon of the channel matches a face of
    /// exactly one cell, and the distinct cell faces are the evidence's
    /// 2173 (1747 interior plus 426 boundary). A face matched by node
    /// order instead of node set would leave owners unknown.
    #[test]
    fn links_boundary_faces_to_their_cells() {
        let m = read_vtu(&fixture("channel/foehncraft.vtu")).unwrap();
        assert_eq!(m.boundary_owner.len(), 426);
        assert!(
            m.boundary_owner
                .iter()
                .all(|&c| (c as usize) < m.cell_count())
        );
        assert_eq!(m.unique_faces, 2173);
    }

    /// Invariant: the face topology is consistent. Every distinct face has
    /// an owner; exactly the 1747 interior faces have a neighbour; each
    /// boundary polygon is a distinct face with no neighbour and points
    /// back to its polygon; each cell face maps to a distinct face that
    /// lists that cell. Picking, the crinkle clip and hiding cells all
    /// walk this table; a wrong neighbour would open holes in the surface.
    #[test]
    fn face_topology_is_consistent() {
        let m = read_vtu(&fixture("channel/foehncraft.vtu")).unwrap();
        assert_eq!(m.face_owner.len(), 2173);
        let interior = m.face_neighbour.iter().filter(|&&n| n != NO_CELL).count();
        assert_eq!(interior, 1747);
        for (b, &u) in m.boundary_face_id.iter().enumerate() {
            assert_ne!(u, NO_CELL);
            assert_eq!(m.face_neighbour[u as usize], NO_CELL);
            assert_eq!(m.face_boundary[u as usize], b as u32);
        }
        for c in 0..m.cell_count() {
            for f in m.cell_face_range(c) {
                let u = m.cell_face_id[f] as usize;
                let c = c as u32;
                assert!(m.face_owner[u] == c || m.face_neighbour[u] == c);
                assert_ne!(m.across(u, c), c);
            }
        }
    }

    const TET: &str = r#"<?xml version="1.0"?>
<VTKFile type="UnstructuredGrid" version="1.0" byte_order="LittleEndian">
<UnstructuredGrid><Piece NumberOfPoints="4" NumberOfCells="1">
<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">
0 0 0 1 0 0 0 1 0 0 0 1</DataArray></Points>
<Cells>
<DataArray type="Int64" Name="connectivity" format="ascii">0 1 2 3</DataArray>
<DataArray type="Int64" Name="offsets" format="ascii">4</DataArray>
<DataArray type="UInt8" Name="types" format="ascii">10</DataArray>
</Cells></Piece></UnstructuredGrid></VTKFile>"#;

    /// Invariant: a volume file without boundary polygons still shows its
    /// surface, found as the faces used by one cell.
    #[test]
    fn extracts_the_boundary_of_a_plain_volume_file() {
        let m = read_vtu_str(TET).unwrap();
        assert_eq!(m.cell_count(), 1);
        assert_eq!(m.cell_kind, vec![CellKind::Tetrahedron]);
        assert_eq!(m.boundary.len(), 4);
        assert!(m.boundary_extracted);
        assert_eq!(m.boundary_owner, [0; 4]);
        assert_eq!(m.unique_faces, 4);
        assert_eq!(m.patches.len(), 1);
    }

    /// The signed volume of cell `c` by the divergence theorem over its
    /// fan-triangulated faces; positive when the faces point outward.
    fn signed_volume(m: &Mesh, c: usize) -> f64 {
        let mut v = 0.0;
        for f in m.cell(c) {
            let p0 = m.points[f.at(0) as usize];
            for k in 1..f.len() - 1 {
                let (a, b) = (m.points[f.at(k) as usize], m.points[f.at(k + 1) as usize]);
                v += p0[0] * (a[1] * b[2] - a[2] * b[1]) - p0[1] * (a[0] * b[2] - a[2] * b[0])
                    + p0[2] * (a[0] * b[1] - a[1] * b[0]);
            }
        }
        v / 6.0
    }

    /// Invariant: the linear cells in VTK's node order (the base of the
    /// tetrahedron, hexahedron, wedge and pyramid pointing towards the
    /// rest of the cell, as VTK 9, FoehnCraft and KoflCraft write them)
    /// open with outward faces: a positive volume, the unit volume of
    /// each shape. A wedge was read inside out before 2026-11-24.
    #[test]
    fn linear_cells_open_outward() {
        let cells: [(&str, &str, f64); 4] = [
            ("10", "0 0 0 1 0 0 0 1 0 0 0 1", 1.0 / 6.0),
            ("12", "0 0 0 1 0 0 1 1 0 0 1 0 0 0 1 1 0 1 1 1 1 0 1 1", 1.0),
            ("13", "0 0 0 1 0 0 0 1 0 0 0 1 1 0 1 0 1 1", 0.5),
            ("14", "0 0 0 1 0 0 1 1 0 0 1 0 0.5 0.5 1", 1.0 / 3.0),
        ];
        for (ty, points, volume) in cells {
            let n = points.split_whitespace().count() / 3;
            let ids: Vec<String> = (0..n).map(|i| i.to_string()).collect();
            let text = TET
                .replace("NumberOfPoints=\"4\"", &format!("NumberOfPoints=\"{n}\""))
                .replace("0 0 0 1 0 0 0 1 0 0 0 1<", &format!("{points}<"))
                .replace(">0 1 2 3<", &format!(">{}<", ids.join(" ")))
                .replace(">4</", &format!(">{n}</"))
                .replace(">10</", &format!(">{ty}</"));
            let m = read_vtu_str(&text).unwrap();
            assert_eq!(m.cell_count(), 1, "type {ty}");
            let v = signed_volume(&m, 0);
            assert!((v - volume).abs() < 1e-12, "type {ty}: volume {v}");
        }
    }

    /// Invariant: binary (base64) arrays decode the same as ascii ones.
    #[test]
    fn reads_binary_arrays() {
        let enc = |bytes: Vec<u8>| {
            let mut v = (bytes.len() as u32).to_le_bytes().to_vec();
            v.extend(bytes);
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v)
        };
        let pts: Vec<u8> = [0.0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let conn: Vec<u8> = [0i32, 1, 2, 3]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let text = TET
            .replace(
                "type=\"Float64\" NumberOfComponents=\"3\" format=\"ascii\">\n0 0 0 1 0 0 0 1 0 0 0 1",
                &format!("type=\"Float32\" NumberOfComponents=\"3\" format=\"binary\">{}", enc(pts)),
            )
            .replace(
                "type=\"Int64\" Name=\"connectivity\" format=\"ascii\">0 1 2 3",
                &format!("type=\"Int32\" Name=\"connectivity\" format=\"binary\">{}", enc(conn)),
            );
        let m = read_vtu_str(&text).unwrap();
        assert_eq!(m.points[3], [0.0, 0.0, 1.0]);
        assert_eq!(m.boundary.len(), 4);
    }

    fn code(text: &str) -> &'static str {
        match read_vtu_str(text) {
            Ok(_) => "ok",
            Err(e) => e.code,
        }
    }

    /// The broken-input corpus (KoflCraft AG rule: the input of every crash
    /// joins it). Each case must give its code, never a panic.
    #[test]
    fn broken_inputs_give_codes() {
        assert_eq!(code(TET), "ok");
        assert_eq!(code(""), "MESH002");
        assert_eq!(code("<VTKFile type=\"PolyData\"/>"), "MESH002");
        assert_eq!(code("<VTKFile type=\"UnstructuredGrid\"><Piece"), "MESH003");
        assert_eq!(
            code(&TET.replace("NumberOfPoints=\"4\"", "NumberOfPoints=\"99999999\"")),
            "MESH004"
        );
        assert_eq!(
            code(&TET.replace("0 1 0 0 0 1</", "0 1 0 0 0</")),
            "MESH005"
        );
        assert_eq!(code(&TET.replace("0 0 0 1 0 0", "0 0 x 1 0 0")), "MESH005");
        assert_eq!(code(&TET.replace(">0 1 2 3<", ">0 1 2 7<")), "MESH006");
        assert_eq!(code(&TET.replace(">4</", ">9</")), "MESH006");
        assert_eq!(code(&TET.replace(">10</", ">3</")), "MESH008");
        assert_eq!(
            code(&TET.replace(
                "format=\"ascii\">0 1 2 3",
                "format=\"appended\" offset=\"0\">"
            )),
            "MESH007"
        );
        assert_eq!(
            code(&TET.replace(
                "byte_order",
                "compressor=\"vtkZLibDataCompressor\" byte_order"
            )),
            "MESH007"
        );
        assert_eq!(
            code(&TET.replace(
                "<Piece",
                "<Piece NumberOfPoints=\"0\" NumberOfCells=\"0\"/><Piece"
            )),
            "MESH012"
        );
        assert_eq!(
            code(
                &TET.replace("NumberOfCells=\"1\"", "NumberOfCells=\"0\"")
                    .replace(">4</", "></")
                    .replace(">10</", "></")
                    .replace(">0 1 2 3<", "><")
            ),
            "MESH010"
        );
        // A polyhedron whose face stream is cut short.
        let poly = TET
            .replace(">10</", ">42</")
            .replace(
                "</Cells>",
                "<DataArray type=\"Int64\" Name=\"faces\" format=\"ascii\">4 3 0 1 3 3 1 2 3 3 2 0 3 3 0 2</DataArray><DataArray type=\"Int64\" Name=\"faceoffsets\" format=\"ascii\">16</DataArray></Cells>",
            );
        assert_eq!(code(&poly), "MESH009");
        assert_eq!(
            code(&poly.replace("0 2</", "0 2 1</").replace(">16<", ">17<")),
            "ok"
        );
    }

    /// Read `bytes` through a reader that hands them over `cap` bytes at a
    /// time, as a file streams in chunks.
    fn read_in_pieces(bytes: &[u8], cap: usize) -> Result<(Mesh, Results), MeshError> {
        let src = std::io::BufReader::with_capacity(cap, bytes);
        let (mut m, polys, r) = parse(src, bytes.len() as u64)?;
        polys.link(&mut m);
        Ok((m, r))
    }

    /// Invariant: the streaming reader reads the same mesh and fields
    /// however the file is cut into chunks (inside a number, a base64
    /// group, a tag or the whitespace between them), ascii and base64,
    /// at one thread and at several, as it reads the whole text at once.
    /// A number cut in two at a chunk's end, a lost base64 remainder or a
    /// header split over two chunks would change the mesh; the other tests
    /// read small files in one chunk and would not see it.
    #[test]
    fn streams_in_any_pieces() {
        let dir = std::env::temp_dir().join(format!("murmel-vtu-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mesh = crate::synth::honeycomb(2);
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for (name, enc) in [
            ("ascii", crate::synth::Encoding::Ascii),
            ("binary", crate::synth::Encoding::Binary),
        ] {
            let path = dir.join(format!("{name}.vtu"));
            crate::synth::write_vtu(&mesh, &path, enc).unwrap();
            files.push((name.into(), std::fs::read(&path).unwrap()));
        }
        // Base64 broken over lines, as some writers do.
        let wrapped = String::from_utf8(files[1].1.clone())
            .unwrap()
            .lines()
            .map(|l| {
                if l.len() > 200 {
                    l.as_bytes()
                        .chunks(76)
                        .map(|c| String::from_utf8_lossy(c).into_owned())
                        .collect::<Vec<_>>()
                        .join("\n ")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        files.push(("wrapped".into(), wrapped.into_bytes()));
        files.push((
            "box".into(),
            std::fs::read(fixture("box/foehncraft.vtu")).unwrap(),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let whole = |b: &[u8]| {
            let (m, r) = read_vtu_str_results(std::str::from_utf8(b).unwrap()).unwrap();
            format!("{m:?} {r:?}")
        };
        let ascii = whole(&files[0].1);
        for (name, bytes) in &files {
            let want = whole(bytes);
            if name != "box" {
                // ascii and base64 hold the same numbers.
                assert_eq!(want, ascii, "{name}");
            }
            for threads in [1, 3] {
                crate::par::with_threads(threads, || {
                    for cap in [1, 2, 3, 5, 7, 64, 4099] {
                        let (m, r) = read_in_pieces(bytes, cap).unwrap();
                        assert_eq!(format!("{m:?} {r:?}"), want, "{name}, chunks of {cap}");
                    }
                });
            }
        }
    }

    /// Invariant: the arrays with no declared count (the connectivity and
    /// the face stream, here over 64 KiB of text each) read the same into
    /// room guessed from their first piece (the whole text at once, or
    /// pieces of 256 KiB) as into room that grows from nothing (pieces too
    /// short to guess from), at one thread and at several. A guess that
    /// lost or moved the values read before it would change the cells.
    #[test]
    fn guessed_room_reads_the_same() {
        let dir = std::env::temp_dir().join(format!("murmel-vtu-guess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mesh = crate::synth::honeycomb(8);
        let path = dir.join("ascii.vtu");
        crate::synth::write_vtu(&mesh, &path, crate::synth::Encoding::Ascii).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        for threads in [1, 3] {
            crate::par::with_threads(threads, || {
                let grown = format!("{:?}", read_in_pieces(&bytes, 4099).unwrap());
                let (m, r) = read_vtu_str_results(std::str::from_utf8(&bytes).unwrap()).unwrap();
                assert_eq!(m.cell_count(), mesh.cell_count());
                assert_eq!(m.points, mesh.points);
                assert_eq!(format!("{:?}", (m, r)), grown);
                let pieces = format!("{:?}", read_in_pieces(&bytes, 1 << 18).unwrap());
                assert_eq!(pieces, grown);
            });
        }
    }

    /// Invariant: polyhedra read the same faces whether their points are
    /// written over the face stream (only polyhedra and polygons) or
    /// copied out of it (a tetrahedron among them). A write that ran
    /// ahead of the stream it reads, or a stream kept with its counts,
    /// would change the faces of the polyhedron.
    #[test]
    fn faces_written_in_place_match_copied_ones() {
        let faces = "<DataArray type=\"Int64\" Name=\"faces\" format=\"ascii\">4 3 0 1 3 3 1 2 3 3 2 0 3 3 0 2 1</DataArray><DataArray type=\"Int64\" Name=\"faceoffsets\" format=\"ascii\">17</DataArray></Cells>";
        let poly = TET.replace(">10</", ">42</").replace("</Cells>", faces);
        let mixed = poly
            .replace("NumberOfCells=\"1\"", "NumberOfCells=\"2\"")
            .replace(">0 1 2 3<", ">0 1 2 3 0 1 2 3<")
            .replace(">4</", ">4 8</")
            .replace(">42</", ">42 10</")
            .replace(">17<", ">17 -1<");
        let a = read_vtu_str(&poly).unwrap();
        let b = read_vtu_str(&mixed).unwrap();
        let cell0 = |m: &Mesh| {
            m.cell(0)
                .map(|f| f.oriented().collect::<Vec<u32>>())
                .collect::<Vec<_>>()
        };
        let want = vec![vec![0, 1, 3], vec![1, 2, 3], vec![2, 0, 3], vec![0, 2, 1]];
        assert_eq!(cell0(&a), want);
        assert_eq!(cell0(&b), want);
        assert_eq!(b.cell_count(), 2);
        assert_eq!(a.distinct.nodes.len(), 12);
    }

    /// The broken inputs of the streaming reader: a file cut short inside
    /// an array or before its elements close, and raw appended data (not
    /// UTF-8) after the arrays. Each must give its code.
    #[test]
    fn cut_files_and_raw_data_give_codes() {
        let at = TET.find("0 1 2 3").unwrap() + 3;
        assert_eq!(code(&TET[..at]), "MESH003");
        let at = TET.find("</Piece>").unwrap();
        assert_eq!(code(&TET[..at]), "MESH003");
        let mut raw = TET
            .replace(
                "</UnstructuredGrid>",
                "</UnstructuredGrid><AppendedData encoding=\"raw\">_",
            )
            .replace("</VTKFile>", "")
            .into_bytes();
        raw.extend([0xff, 0xfe, 0x00, 0x80]);
        raw.extend(b"</AppendedData></VTKFile>");
        let e = parse(raw.as_slice(), raw.len() as u64)
            .map(|_| ())
            .unwrap_err();
        assert_eq!(e.code, "MESH007");
        let empty = TET.replace(
            "format=\"ascii\">0 1 2 3</DataArray>",
            "format=\"binary\">AAAA</DataArray>",
        );
        assert_eq!(code(&empty), "MESH005");
    }
}
