//! Reader for OpenFOAM `polyMesh` directories, written from the OpenFOAM
//! user guide's description of the file format ("Basic input/output file
//! format": the `FoamFile` header, dictionaries, `N ( … )` lists, binary
//! lists as raw bytes between the parentheses) and of the mesh ("Mesh
//! description": `points`, `faces`, `owner`, `neighbour`, `boundary`), and
//! from KoflCraft's `docs/foehncraft.md` (FC-03, FC-04). No OpenFOAM source
//! was read.
//!
//! The mesh is face based: every face has an owner cell and, if it is an
//! interior face, a neighbour; its normal points from the owner to the
//! neighbour. Interior faces come first, then the boundary faces patch by
//! patch (`startFace`, `nFaces`). The reader turns it into the cell-based
//! model of [`Mesh`]: every cell lists its faces outward (a neighbour's
//! faces are reversed), and the face topology comes straight from
//! `owner` and `neighbour`, so `cell:<n>` and `face:<n>` are OpenFOAM's
//! own cell and face labels.
//!
//! Supported: `ascii` and `binary` files (32 or 64 bit labels, 32 or 64
//! bit scalars, either byte order, from the header's `arch`), `faceList`
//! and `faceCompactList`, files compressed with gzip (`points.gz`), and
//! `cellZones`. Cells in no zone join a zone `cells` (the name the VTU
//! reader gives unnamed cells); a cell in two zones stays in the first.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::{CellKind, Mesh, MeshError, NO_CELL, Patch, PatchRole, Polygons, numbers};

/// The files of a polyMesh the reader needs, without `cellZones`.
const REQUIRED: [&str; 5] = ["points", "faces", "owner", "neighbour", "boundary"];

/// Decompressed files larger than this are refused (a gzip bomb).
const MAX_DECOMPRESSED: u64 = 16 << 30;

fn err(code: &'static str, message: impl Into<String>) -> MeshError {
    MeshError::new(code, message)
}

fn has_file(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file() || dir.join(format!("{name}.gz")).is_file()
}

fn is_polymesh_dir(dir: &Path) -> bool {
    dir.is_dir() && has_file(dir, "faces") && has_file(dir, "owner")
}

/// True when `path` names an OpenFOAM mesh: a case, `constant` or
/// `polyMesh` directory, a `.foam` file in a case, or one of the files
/// of a `polyMesh`.
pub fn is_openfoam(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("foam") || ext.eq_ignore_ascii_case("openfoam") {
        return true;
    }
    path.parent().is_some_and(is_polymesh_dir)
}

/// The `polyMesh` directory `path` names (see [`is_openfoam`]).
pub fn polymesh_dir(path: &Path) -> Result<PathBuf, MeshError> {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        return Err(err("MESH001", format!("{} does not exist", path.display())));
    };
    for dir in [
        start.clone(),
        start.join("polyMesh"),
        start.join("constant").join("polyMesh"),
    ] {
        if is_polymesh_dir(&dir) {
            return Ok(dir);
        }
    }
    Err(err(
        "MESH013",
        format!(
            "{} holds no OpenFOAM mesh (no polyMesh/faces or constant/polyMesh/faces)",
            path.display()
        ),
    ))
}

/// The bytes of `dir/name`, or of `dir/name.gz` decompressed, and the
/// path read; `None` when neither exists.
/// A file of the polyMesh as read: its bytes on disk (hashed for the
/// validity badge) and, for a `.gz`, the decompressed text.
struct FileBytes {
    raw: Vec<u8>,
    unzipped: Option<Vec<u8>>,
    path: PathBuf,
}

impl FileBytes {
    fn text(&self) -> &[u8] {
        self.unzipped.as_deref().unwrap_or(&self.raw)
    }
}

fn read_file(dir: &Path, name: &str) -> Result<Option<FileBytes>, MeshError> {
    let plain = dir.join(name);
    if plain.is_file() {
        let raw = std::fs::read(&plain)
            .map_err(|e| err("MESH001", format!("cannot read {}: {e}", plain.display())))?;
        return Ok(Some(FileBytes {
            raw,
            unzipped: None,
            path: plain,
        }));
    }
    let gz = dir.join(format!("{name}.gz"));
    if gz.is_file() {
        let raw = std::fs::read(&gz)
            .map_err(|e| err("MESH001", format!("cannot read {}: {e}", gz.display())))?;
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(&raw[..])
            .take(MAX_DECOMPRESSED + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                err(
                    "MESH001",
                    format!("cannot decompress {}: {e}", gz.display()),
                )
            })?;
        if bytes.len() as u64 > MAX_DECOMPRESSED {
            return Err(err(
                "MESH004",
                format!("{} decompresses to more than 16 GiB", gz.display()),
            ));
        }
        return Ok(Some(FileBytes {
            raw,
            unzipped: Some(bytes),
            path: gz,
        }));
    }
    Ok(None)
}

/// Read the OpenFOAM mesh `path` names (see [`is_openfoam`]). The paths of
/// the files read are in [`Mesh::files`].
pub fn read_polymesh(path: &Path) -> Result<Mesh, MeshError> {
    read_polymesh_with(path, true)
}

/// As [`read_polymesh`]; without `hash` no BLAKE3 hash is taken
/// (`Mesh::file_blake3` stays empty).
pub(crate) fn read_polymesh_with(path: &Path, hash: bool) -> Result<Mesh, MeshError> {
    let dir = polymesh_dir(path)?;
    let io = crate::profile::span("read.file");
    let mut read: Vec<FileBytes> = Vec::new();
    for name in REQUIRED {
        let Some(f) = read_file(&dir, name)? else {
            return Err(err(
                "MESH013",
                format!("{} has no {name} file", dir.display()),
            ));
        };
        read.push(f);
    }
    let zones = read_file(&dir, "cellZones")?;
    drop(io);
    crate::profile::check()?;
    let all: Vec<&FileBytes> = read.iter().chain(zones.as_ref()).collect();
    // The files are hashed on other threads while they are parsed.
    let (mesh, digests) = std::thread::scope(|s| {
        let hashes: Vec<_> = all
            .iter()
            .filter(|_| hash)
            .map(|f| s.spawn(|| crate::FileHash::Blake3.of_bytes(&f.raw)))
            .collect();
        let [points, faces, owner, neighbour, boundary] = [0, 1, 2, 3, 4].map(|i| read[i].text());
        let mesh = read_polymesh_bytes(PolyMeshFiles {
            points,
            faces,
            owner,
            neighbour,
            boundary,
            cell_zones: zones.as_ref().map(FileBytes::text),
        });
        let digests: Vec<String> = hashes
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect();
        (mesh, digests)
    });
    let mut mesh = mesh?;
    mesh.files = all.iter().map(|f| f.path.clone()).collect();
    mesh.file_blake3 = digests;
    Ok(mesh)
}

/// The contents of the files of a polyMesh, for [`read_polymesh_bytes`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PolyMeshFiles<'a> {
    pub points: &'a [u8],
    pub faces: &'a [u8],
    pub owner: &'a [u8],
    pub neighbour: &'a [u8],
    pub boundary: &'a [u8],
    pub cell_zones: Option<&'a [u8]>,
}

// ---- lexer ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Ascii,
    Binary,
}

/// What a file's header says.
#[derive(Clone, Debug)]
struct Header {
    format: Format,
    class: String,
    note: Option<String>,
    label_bytes: usize,
    scalar_bytes: usize,
    big_endian: bool,
}

struct Lex<'a> {
    b: &'a [u8],
    i: usize,
    file: &'static str,
}

fn is_delim(c: u8) -> bool {
    c.is_ascii_whitespace() || matches!(c, b'(' | b')' | b'{' | b'}' | b';' | b'"')
}

impl<'a> Lex<'a> {
    fn new(b: &'a [u8], file: &'static str) -> Self {
        Self { b, i: 0, file }
    }

    fn bad(&self, what: impl std::fmt::Display) -> MeshError {
        err(
            "MESH014",
            format!("{}: {what} (byte {})", self.file, self.i),
        )
    }

    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.i)
    }

    /// Skip white space and `//` and `/* */` comments.
    fn skip(&mut self) {
        loop {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.b[self.i.min(self.b.len())..].starts_with(b"//") {
                while self.i < self.b.len() && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.b[self.i.min(self.b.len())..].starts_with(b"/*") {
                match find(&self.b[self.i + 2..], b"*/") {
                    Some(k) => self.i += 2 + k + 2,
                    None => self.i = self.b.len(),
                }
            } else {
                return;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.b.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), MeshError> {
        match self.peek() {
            Some(x) if x == c => {
                self.i += 1;
                Ok(())
            }
            Some(x) => Err(self.bad(format!(
                "expected '{}', found '{}'",
                c as char,
                (x as char).escape_default()
            ))),
            None => Err(self.bad(format!("expected '{}', found the end", c as char))),
        }
    }

    /// Take `c` if it comes next.
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// A word or a number; a quoted string without its quotes.
    fn word(&mut self) -> Result<&'a str, MeshError> {
        match self.peek() {
            None => Err(self.bad("unexpected end of file")),
            Some(b'"') => {
                self.i += 1;
                let start = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'"' {
                    self.i += 1;
                }
                let s = &self.b[start..self.i];
                if self.i >= self.b.len() {
                    return Err(self.bad("a string is not closed"));
                }
                self.i += 1;
                std::str::from_utf8(s).map_err(|_| self.bad("a string is not UTF-8"))
            }
            Some(c) if is_delim(c) => {
                Err(self.bad(format!("expected a word, found '{}'", c as char)))
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.b.len() && !is_delim(self.b[self.i]) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i])
                    .map_err(|_| self.bad("a word is not UTF-8"))
            }
        }
    }

    fn label(&mut self) -> Result<i64, MeshError> {
        let w = self.word()?;
        w.parse::<i64>().map_err(|_| {
            err(
                "MESH005",
                format!("{}: '{w}' is not a whole number", self.file),
            )
        })
    }

    fn scalar(&mut self) -> Result<f64, MeshError> {
        let w = self.word()?;
        w.parse::<f64>()
            .map_err(|_| err("MESH005", format!("{}: '{w}' is not a number", self.file)))
    }

    /// Skip the tokens of a dictionary entry's value up to its `;` (or the
    /// `}` that closes the dictionary), nested lists included.
    fn skip_value(&mut self) -> Result<(), MeshError> {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => return Err(self.bad("an entry is not closed")),
                Some(b';') if depth == 0 => {
                    self.i += 1;
                    return Ok(());
                }
                Some(b'}') if depth == 0 => return Ok(()),
                Some(b'(' | b'{') => {
                    depth += 1;
                    self.i += 1;
                }
                Some(b')' | b'}') => {
                    if depth == 0 {
                        return Err(self.bad("a ')' closes nothing"));
                    }
                    depth -= 1;
                    self.i += 1;
                }
                Some(b';') => self.i += 1,
                Some(_) => {
                    self.word()?;
                }
            }
        }
    }

    /// The `FoamFile` header dictionary.
    fn header(&mut self) -> Result<Header, MeshError> {
        if self.word()? != "FoamFile" {
            return Err(self.bad("the file does not start with a FoamFile header"));
        }
        self.expect(b'{')?;
        let mut h = Header {
            format: Format::Ascii,
            class: String::new(),
            note: None,
            label_bytes: 4,
            scalar_bytes: 8,
            big_endian: false,
        };
        while !self.eat(b'}') {
            let key = self.word()?;
            match key {
                "format" | "class" | "note" | "arch" => {
                    let v = self.word()?;
                    match key {
                        "format" => {
                            h.format = match v {
                                "ascii" => Format::Ascii,
                                "binary" => Format::Binary,
                                _ => {
                                    return Err(err(
                                        "MESH007",
                                        format!("{}: format {v} is not ascii or binary", self.file),
                                    ));
                                }
                            }
                        }
                        "class" => h.class = v.to_string(),
                        "note" => h.note = Some(v.to_string()),
                        _ => arch(&mut h, v, self.file)?,
                    }
                    self.skip_value()?;
                }
                _ => self.skip_value()?,
            }
        }
        Ok(h)
    }

    /// The count of a list, after an optional `List<…>` word; the `(`
    /// after it is taken.
    fn list_start(&mut self, min_bytes: usize) -> Result<usize, MeshError> {
        let mut w = self.word()?;
        if w.starts_with("List<") || w.ends_with("List") {
            w = self.word()?;
        }
        let n: usize = w
            .parse()
            .map_err(|_| self.bad(format!("expected the size of a list, found '{w}'")))?;
        if self.peek() == Some(b'{') {
            return Err(self.bad("uniform lists (N{value}) are not supported"));
        }
        self.expect(b'(')?;
        if n.saturating_mul(min_bytes) > self.remaining() {
            return Err(err(
                "MESH004",
                format!(
                    "{}: a list of {n} entries cannot fit in the {} bytes left",
                    self.file,
                    self.remaining()
                ),
            ));
        }
        Ok(n)
    }

    /// The `per * n` numbers of the ascii list body that starts here (after
    /// its `(`), parsed in parallel, leaving the `)` that closes the list
    /// to the caller; `None` (nothing taken) when the body holds anything
    /// else, such as a comment, so the token-by-token reader can say what
    /// is wrong.
    fn fast_list<S: numbers::Slot>(
        &mut self,
        n: usize,
        per: usize,
        sep: &numbers::Sep,
    ) -> Option<Vec<S>> {
        let rest = &self.b[self.i..];
        let end = list_end(rest);
        let end = end?;
        let v = numbers::parse::<S>(&rest[..end], sep).ok()?;
        if v.len().checked_mul(S::N)? != n.checked_mul(per)? {
            return None;
        }
        self.i += end;
        Some(v)
    }

    /// `n` raw binary values of `size` bytes each.
    fn raw(&mut self, n: usize, size: usize) -> Result<&'a [u8], MeshError> {
        let len = n.saturating_mul(size);
        if len > self.remaining() {
            return Err(err(
                "MESH004",
                format!("{}: {n} binary values cannot fit in the file", self.file),
            ));
        }
        let s = &self.b[self.i..self.i + len];
        self.i += len;
        Ok(s)
    }

    /// A list of labels, ascii or binary.
    fn labels(&mut self, h: &Header) -> Result<Vec<i64>, MeshError> {
        let v = match h.format {
            Format::Ascii => {
                let n = self.list_start(2)?;
                match self.fast_list::<i64>(n, 1, &numbers::SPACE) {
                    Some(v) => v,
                    None => {
                        let mut v = Vec::with_capacity(n);
                        for _ in 0..n {
                            v.push(self.label()?);
                        }
                        v
                    }
                }
            }
            Format::Binary => {
                let n = self.list_start(h.label_bytes)?;
                let raw = self.raw(n, h.label_bytes)?;
                numbers::le(raw, h.label_bytes, int_decoder(h.label_bytes, h.big_endian))
            }
        };
        self.expect(b')')?;
        Ok(v)
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `arch "LSB;label=32;scalar=64"`.
fn arch(h: &mut Header, v: &str, file: &str) -> Result<(), MeshError> {
    for part in v.split(';') {
        let part = part.trim();
        let bits = |s: &str| -> Result<usize, MeshError> {
            match s {
                "32" => Ok(4),
                "64" => Ok(8),
                _ => Err(err(
                    "MESH007",
                    format!("{file}: arch {v}: only 32 and 64 bit values are read"),
                )),
            }
        };
        if part == "MSB" {
            h.big_endian = true;
        } else if part == "LSB" {
            h.big_endian = false;
        } else if let Some(b) = part.strip_prefix("label=") {
            h.label_bytes = bits(b)?;
        } else if let Some(b) = part.strip_prefix("scalar=") {
            h.scalar_bytes = bits(b)?;
        }
    }
    Ok(())
}

/// The decoder of one label of `size` bytes (chosen once per list).
fn int_decoder(size: usize, big: bool) -> fn(&[u8]) -> i64 {
    match (size, big) {
        (4, false) => |c| i64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]])),
        (4, true) => |c| i64::from(i32::from_be_bytes([c[0], c[1], c[2], c[3]])),
        (_, false) => |c| decode_int(c, false),
        (_, true) => |c| decode_int(c, true),
    }
}

fn decode_int(c: &[u8], big: bool) -> i64 {
    match c.len() {
        4 => {
            let a = [c[0], c[1], c[2], c[3]];
            i64::from(if big {
                i32::from_be_bytes(a)
            } else {
                i32::from_le_bytes(a)
            })
        }
        _ => {
            let a = [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]];
            if big {
                i64::from_be_bytes(a)
            } else {
                i64::from_le_bytes(a)
            }
        }
    }
}

fn decode_float(c: &[u8], big: bool) -> f64 {
    match c.len() {
        4 => {
            let a = [c[0], c[1], c[2], c[3]];
            f64::from(if big {
                f32::from_be_bytes(a)
            } else {
                f32::from_le_bytes(a)
            })
        }
        _ => {
            let a = [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]];
            if big {
                f64::from_be_bytes(a)
            } else {
                f64::from_le_bytes(a)
            }
        }
    }
}

// ---- files ------------------------------------------------------------------

fn points(b: &[u8]) -> Result<Vec<[f64; 3]>, MeshError> {
    let mut l = Lex::new(b, "points");
    let h = l.header()?;
    let v = match h.format {
        Format::Ascii => {
            let n = l.list_start(7)?;
            match l.fast_list::<[f64; 3]>(n, 3, &numbers::SPACE_PAREN) {
                Some(xyz) => xyz,
                None => {
                    let mut v = Vec::with_capacity(n);
                    for _ in 0..n {
                        l.expect(b'(')?;
                        let p = [l.scalar()?, l.scalar()?, l.scalar()?];
                        l.expect(b')')?;
                        v.push(p);
                    }
                    v
                }
            }
        }
        Format::Binary => {
            let s = h.scalar_bytes;
            let n = l.list_start(3 * s)?;
            let raw = l.raw(n, 3 * s)?;
            let big = h.big_endian;
            if (s, big) == (8, false) {
                numbers::le(raw, 24, |c| {
                    let f = |k: usize| {
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&c[8 * k..8 * k + 8]);
                        f64::from_le_bytes(b)
                    };
                    [f(0), f(1), f(2)]
                })
            } else {
                raw.chunks_exact(3 * s)
                    .map(|c| [0, 1, 2].map(|k| decode_float(&c[k * s..(k + 1) * s], big)))
                    .collect()
            }
        }
    };
    l.expect(b')')?;
    if let Some(k) = v.iter().position(|p| p.iter().any(|x| !x.is_finite())) {
        return Err(err("MESH005", format!("points: point {k} is not finite")));
    }
    Ok(v)
}

/// The faces, flat, with the header (its `note` is kept as metadata).
fn faces(b: &[u8], n_points: usize) -> Result<(Polygons, Header), MeshError> {
    let mut l = Lex::new(b, "faces");
    let h = l.header()?;
    let index = |v: i64, f: usize| -> Result<u32, MeshError> {
        if v < 0 || v as u64 >= n_points as u64 {
            Err(err(
                "MESH006",
                format!("faces: face {f} uses point {v}, but there are {n_points} points"),
            ))
        } else {
            Ok(v as u32)
        }
    };
    let mut out = Polygons::new();
    if h.class == "faceCompactList" {
        if let Some(fast) = compact_u32(&mut l, &h, n_points) {
            return Ok((fast, h));
        }
        let offsets = l.labels(&h)?;
        let nodes = l.labels(&h)?;
        if offsets.first().copied().unwrap_or(0) != 0 {
            return Err(err(
                "MESH005",
                "faces: the compact offsets do not start at 0",
            ));
        }
        // Checked and converted in parallel; the loop below names the
        // first broken face when the check fails.
        let ok_offsets = crate::par::map(offsets.len().saturating_sub(1), 1 << 16, |r| {
            r.into_iter().all(|f| {
                let (a, z) = (offsets[f], offsets[f + 1]);
                a <= z && z as u64 <= nodes.len() as u64 && z - a >= 3
            })
        });
        let ok_nodes = crate::par::map(nodes.len(), 1 << 16, |r| {
            nodes[r]
                .iter()
                .all(|&v| v >= 0 && (v as u64) < n_points as u64)
        });
        let in_use = offsets.last().map_or(0, |&z| z as usize);
        if ok_offsets.iter().all(|&b| b) && ok_nodes.iter().all(|&b| b) {
            out.start = crate::par::collect(offsets.len(), 1 << 16, |r| {
                offsets[r].iter().map(|&v| v as u32).collect()
            });
            out.nodes = crate::par::collect(in_use.min(nodes.len()), 1 << 16, |r| {
                nodes[r].iter().map(|&v| v as u32).collect()
            });
            if out.start.is_empty() {
                out.start.push(0);
            }
            return Ok((out, h));
        }
        for (f, w) in offsets.windows(2).enumerate() {
            let (a, z) = (w[0], w[1]);
            if a > z || z as u64 > nodes.len() as u64 || z - a < 3 {
                return Err(err(
                    "MESH005",
                    format!(
                        "faces: face {f} has offsets {a}..{z} of {} labels",
                        nodes.len()
                    ),
                ));
            }
            let face = &nodes[a as usize..z as usize];
            if let Some(&v) = face.iter().find(|&&v| v < 0 || v as u64 >= n_points as u64) {
                index(v, f)?;
            }
            out.push(face.iter().map(|&v| v as u32));
        }
    } else if let Some(faces) = (h.format == Format::Ascii)
        .then(|| fast_faces(&mut l, n_points))
        .flatten()
    {
        out = faces;
    } else {
        let n = l.list_start(4)?;
        out.start.reserve(n);
        for f in 0..n {
            let k = l.label()?;
            if k < 3 {
                return Err(err("MESH005", format!("faces: face {f} has {k} points")));
            }
            let k = k as usize;
            l.expect(b'(')?;
            let mut face = Vec::with_capacity(k.min(l.remaining()));
            match h.format {
                Format::Ascii => {
                    for _ in 0..k {
                        face.push(index(l.label()?, f)?);
                    }
                }
                Format::Binary => {
                    for c in l.raw(k, h.label_bytes)?.chunks_exact(h.label_bytes) {
                        face.push(index(decode_int(c, h.big_endian), f)?);
                    }
                }
            }
            l.expect(b')')?;
            out.push(face);
        }
        l.expect(b')')?;
    }
    Ok((out, h))
}

/// A binary `faceCompactList` of 32-bit little-endian labels decoded
/// straight into the faces and checked in parallel; `None`, with nothing
/// taken, when it is another layout or broken, so the general reader can
/// say what is wrong.
fn compact_u32(l: &mut Lex<'_>, h: &Header, n_points: usize) -> Option<Polygons> {
    if h.format != Format::Binary || h.label_bytes != 4 || h.big_endian {
        return None;
    }
    let at = l.i;
    let list = |l: &mut Lex<'_>| -> Option<Vec<u32>> {
        let n = l.list_start(4).ok()?;
        let raw = l.raw(n, 4).ok()?;
        let v = numbers::le(raw, 4, |c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        l.expect(b')').ok()?;
        Some(v)
    };
    let parsed = list(l).and_then(|start| Some((start, list(l)?)));
    let ok = parsed.as_ref().is_some_and(|(start, nodes)| {
        let len = nodes.len() as u64;
        // Labels are signed: a negative one reads as 2^31 or more.
        start.first() == Some(&0)
            && crate::par::map(start.len().saturating_sub(1), 1 << 16, |r| {
                r.into_iter().all(|f| {
                    let (a, z) = (start[f], start[f + 1]);
                    a <= z && u64::from(z) <= len && z - a >= 3 && z < 1 << 31
                })
            })
            .into_iter()
            .all(|b| b)
            && crate::par::map(nodes.len(), 1 << 16, |r| {
                nodes[r]
                    .iter()
                    .all(|&v| (v as usize) < n_points && v < 1 << 31)
            })
            .into_iter()
            .all(|b| b)
    });
    match parsed {
        Some((start, mut nodes)) if ok => {
            nodes.truncate(start.last().map_or(0, |&z| z as usize));
            Some(Polygons { start, nodes })
        }
        _ => {
            l.i = at;
            None
        }
    }
}

/// Where the ascii list body `rest` (after its `(`) closes: the `)` at
/// depth 0, with nothing but numbers, whitespace and parentheses before
/// it; `None` when a comment, a dictionary or a string comes first. A
/// list that ends its file (points, faces, owner, neighbour) closes at
/// the file's last `)`, which is checked in parallel pieces; any other
/// list is scanned from the start.
fn list_end(rest: &[u8]) -> Option<usize> {
    let other = |c: u8| matches!(c, b'/' | b'{' | b'}' | b';' | b'"');
    if let Some(end) = rest.iter().rposition(|&c| c == b')') {
        let tail = &rest[end + 1..];
        let trivia = tail.split(|&c| c == b'\n').all(|line| {
            let t = line.trim_ascii();
            t.is_empty() || t.starts_with(b"//")
        });
        if trivia {
            let body = &rest[..end];
            // Each piece: no other character, its change of depth and the
            // lowest depth inside it.
            let pieces = crate::par::map(body.len(), 1 << 20, |r| {
                let (mut d, mut low, mut clean) = (0i64, 0i64, true);
                for &c in &body[r] {
                    match c {
                        b'(' => d += 1,
                        b')' => {
                            d -= 1;
                            low = low.min(d);
                        }
                        c if other(c) => clean = false,
                        _ => {}
                    }
                }
                (d, low, clean)
            });
            let mut depth = 0i64;
            let mut ok = true;
            for (d, low, clean) in pieces {
                ok &= clean && depth + low >= 0;
                depth += d;
            }
            if ok && depth == 0 {
                return Some(end);
            }
        }
    }
    let mut depth = 0usize;
    for (k, &c) in rest.iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' if depth == 0 => return Some(k),
            b')' => depth -= 1,
            c if other(c) => return None,
            _ => {}
        }
    }
    None
}

/// An ascii `faceList` read as one stream of numbers in parallel (`4(a b c
/// d)` is the count and the ids); `None`, with nothing taken, when it is
/// anything else, so the token reader can say what is wrong.
fn fast_faces(l: &mut Lex<'_>, n_points: usize) -> Option<Polygons> {
    let at = l.i;
    let n = l.list_start(4).ok()?;
    let body_start = l.i;
    let rest = &l.b[body_start..];
    let end = list_end(rest);
    let parsed = end.and_then(|e| numbers::parse::<u32>(&rest[..e], &numbers::SPACE_PAREN).ok());
    let Some(stream) = parsed else {
        l.i = at;
        return None;
    };
    let mut out = Polygons::new();
    out.start.reserve(n);
    out.nodes.reserve(stream.len().saturating_sub(n));
    let mut k = 0;
    while k < stream.len() {
        let m = stream[k] as usize;
        let face = stream.get(k + 1..k + 1 + m);
        match face {
            Some(face) if m >= 3 && face.iter().all(|&v| (v as usize) < n_points) => {
                out.push(face.iter().copied());
            }
            _ => {
                l.i = at;
                return None;
            }
        }
        k += 1 + m;
    }
    if out.len() != n {
        l.i = at;
        return None;
    }
    l.i = body_start + end.unwrap_or(0);
    l.expect(b')').ok()?;
    Some(out)
}

fn label_file(b: &[u8], file: &'static str) -> Result<Vec<i64>, MeshError> {
    let mut l = Lex::new(b, file);
    let h = l.header()?;
    l.labels(&h)
}

/// One entry of `boundary`.
struct PatchEntry {
    name: String,
    kind: String,
    n_faces: i64,
    start: i64,
}

fn boundary(b: &[u8]) -> Result<Vec<PatchEntry>, MeshError> {
    let mut l = Lex::new(b, "boundary");
    l.header()?;
    let n = l.list_start(8)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let name = l.word()?.to_string();
        l.expect(b'{')?;
        let (mut kind, mut n_faces, mut start) = (String::new(), None, None);
        while !l.eat(b'}') {
            match l.word()? {
                "type" => {
                    kind = l.word()?.to_string();
                    l.skip_value()?;
                }
                "nFaces" => {
                    n_faces = Some(l.label()?);
                    l.skip_value()?;
                }
                "startFace" => {
                    start = Some(l.label()?);
                    l.skip_value()?;
                }
                _ => l.skip_value()?,
            }
        }
        let (Some(n_faces), Some(start)) = (n_faces, start) else {
            return Err(err(
                "MESH015",
                format!("boundary: patch {name} has no nFaces or startFace"),
            ));
        };
        v.push(PatchEntry {
            name,
            kind,
            n_faces,
            start,
        });
    }
    l.expect(b')')?;
    Ok(v)
}

/// The zones of `cellZones`: names and the cells of each.
fn cell_zones(b: &[u8]) -> Result<Vec<(String, Vec<i64>)>, MeshError> {
    let mut l = Lex::new(b, "cellZones");
    let h = l.header()?;
    let n = l.list_start(8)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let name = l.word()?.to_string();
        l.expect(b'{')?;
        let mut cells = Vec::new();
        while !l.eat(b'}') {
            if l.word()? == "cellLabels" {
                cells = l.labels(&h)?;
                l.eat(b';');
            } else {
                l.skip_value()?;
            }
        }
        v.push((name, cells));
    }
    l.expect(b')')?;
    Ok(v)
}

/// The role of a patch: from its OpenFOAM type (`wall`, `symmetry`,
/// `symmetryPlane`), else from its name as KoflCraft names them (FC-04:
/// the name contains `inlet`, `outlet`, `wall`, `symmetry` or
/// `interface`).
pub fn patch_role(name: &str, kind: &str) -> PatchRole {
    match kind {
        "wall" => return PatchRole::Wall,
        "symmetry" | "symmetryPlane" => return PatchRole::Symmetry,
        _ => {}
    }
    let n = name.to_ascii_lowercase();
    for (word, role) in [
        ("inlet", PatchRole::Inlet),
        ("outlet", PatchRole::Outlet),
        ("wall", PatchRole::Wall),
        ("symmetry", PatchRole::Symmetry),
        ("interface", PatchRole::Interface),
    ] {
        if n.contains(word) {
            return role;
        }
    }
    PatchRole::Unspecified
}

// ---- assembly -----------------------------------------------------------------

/// Read a polyMesh from the contents of its files.
pub fn read_polymesh_bytes(files: PolyMeshFiles<'_>) -> Result<Mesh, MeshError> {
    let t = crate::profile::span("read.points");
    let points = points(files.points)?;
    drop(t);
    crate::profile::check()?;
    let t = crate::profile::span("read.faces");
    let (face_nodes, fh) = faces(files.faces, points.len())?;
    drop(t);
    let t = crate::profile::span("read.owner_neighbour");
    let owner = label_file(files.owner, "owner")?;
    let neighbour = label_file(files.neighbour, "neighbour")?;
    drop(t);
    crate::profile::check()?;
    let _cells = crate::profile::span("read.cells");
    let patches = boundary(files.boundary)?;
    let zones = match files.cell_zones {
        Some(b) => cell_zones(b)?,
        None => Vec::new(),
    };

    let n_faces = face_nodes.len();
    let n_internal = neighbour.len();
    if owner.len() != n_faces {
        return Err(err(
            "MESH005",
            format!("owner has {} labels for {n_faces} faces", owner.len()),
        ));
    }
    if n_internal > n_faces {
        return Err(err(
            "MESH005",
            format!("neighbour has {n_internal} labels for {n_faces} faces"),
        ));
    }
    // Every cell has at least 4 faces and every face at most 2 cells, so
    // a label above n_faces / 2 is broken (and would allocate too much).
    let max_cells = n_faces / 2 + 1;
    let cell = |v: i64, what: &str, f: usize| -> Result<u32, MeshError> {
        if v < 0 || v as u64 >= max_cells as u64 {
            Err(err(
                "MESH006",
                format!("{what} of face {f} is cell {v}, out of range for {n_faces} faces"),
            ))
        } else {
            Ok(v as u32)
        }
    };
    let mut own = Vec::with_capacity(n_faces);
    for (f, &o) in owner.iter().enumerate() {
        own.push(cell(o, "the owner", f)?);
    }
    let mut nei = vec![NO_CELL; n_faces];
    for (f, &n) in neighbour.iter().enumerate() {
        let n = cell(n, "the neighbour", f)?;
        if n == own[f] {
            return Err(err(
                "MESH006",
                format!("face {f} has cell {n} as its owner and its neighbour"),
            ));
        }
        nei[f] = n;
    }
    let n_cells = own
        .iter()
        .chain(&nei[..n_internal])
        .map(|&c| c as usize + 1)
        .max()
        .unwrap_or(0);
    if n_cells == 0 {
        return Err(err("MESH010", "the polyMesh has no cells"));
    }

    // The faces of each cell, in face order: counting sort by cell.
    let mut count = vec![0u32; n_cells + 1];
    for f in 0..n_faces {
        count[own[f] as usize + 1] += 1;
        if f < n_internal {
            count[nei[f] as usize + 1] += 1;
        }
    }
    for c in 0..n_cells {
        if count[c + 1] < 4 {
            return Err(err(
                "MESH006",
                format!(
                    "cell {c} has {} faces; a cell needs at least 4",
                    count[c + 1]
                ),
            ));
        }
        count[c + 1] += count[c];
    }
    let cell_faces = count.clone();
    let total = cell_faces[n_cells] as usize;
    let mut slot_face = vec![0u32; total];
    let mut next = count;
    for f in 0..n_faces {
        for c in [own[f], if f < n_internal { nei[f] } else { NO_CELL }] {
            if c != NO_CELL {
                let s = &mut next[c as usize];
                slot_face[*s as usize] = f as u32;
                *s += 1;
            }
        }
    }

    let mut mesh = Mesh {
        points,
        cell_faces,
        // `Polygons::default()` has no leading 0; `new()` does.
        boundary: Polygons::new(),
        ..Mesh::default()
    };
    // OpenFOAM stores each face once, turned outward from its owner (the
    // lower cell): the distinct faces as they are, and the neighbour sees
    // its face the other way round.
    let size = |f: u32| face_nodes.start[f as usize + 1] - face_nodes.start[f as usize];
    let cf = &mesh.cell_faces;
    mesh.cell_kind = crate::par::collect(n_cells, 1 << 14, |r| {
        r.map(|c| CellKind::classify(cf_of(cf, c).map(|s| size(slot_face[s]) as usize)))
            .collect()
    });
    let (own_ref, slots) = (&own, &slot_face);
    mesh.cell_face_flip = crate::par::collect(n_cells, 1 << 14, |r| {
        r.flat_map(|c| cf_of(cf, c).map(move |s| own_ref[slots[s] as usize] as usize != c))
            .collect()
    });
    mesh.cell_face_id = slot_face;
    mesh.unique_faces = n_faces;
    mesh.face_owner = own;
    mesh.face_neighbour = nei;

    // Boundary faces, patch by patch; the patches must cover the faces
    // after the interior faces once each.
    let mut covered = vec![false; n_faces - n_internal];
    let mut boundary_of = vec![NO_CELL; n_faces];
    for (p, e) in patches.iter().enumerate() {
        let (start, n) = (e.start, e.n_faces);
        if n < 0 || start < n_internal as i64 || start.saturating_add(n) > n_faces as i64 {
            return Err(err(
                "MESH015",
                format!(
                    "boundary: patch {} has faces {start}..{}, outside the boundary faces {n_internal}..{n_faces}",
                    e.name,
                    start.saturating_add(n)
                ),
            ));
        }
        for f in start as usize..(start + n) as usize {
            if std::mem::replace(&mut covered[f - n_internal], true) {
                return Err(err(
                    "MESH015",
                    format!("boundary: face {f} is in two patches"),
                ));
            }
            boundary_of[f] = mesh.boundary.len() as u32;
            mesh.boundary.push(face_nodes.get(f).iter().copied());
            mesh.boundary_patch.push(p as u32);
            mesh.boundary_face_id.push(f as u32);
            mesh.boundary_owner.push(mesh.face_owner[f]);
        }
        mesh.patches.push(Patch {
            name: e.name.clone(),
            role: patch_role(&e.name, &e.kind),
        });
        mesh.metadata
            .insert(format!("openfoam_patch_type.{}", e.name), e.kind.clone());
    }
    if let Some(k) = covered.iter().position(|&c| !c) {
        return Err(err(
            "MESH015",
            format!("boundary: face {} is in no patch", n_internal + k),
        ));
    }
    mesh.face_boundary = boundary_of;
    mesh.distinct = face_nodes;

    // Zones: cellZones in their order, cells in none in a zone "cells".
    let mut cell_zone = vec![NO_CELL; n_cells];
    for (z, (name, cells)) in zones.iter().enumerate() {
        for &c in cells {
            if c < 0 || c as u64 >= n_cells as u64 {
                return Err(err(
                    "MESH006",
                    format!("cellZones: zone {name} lists cell {c} of {n_cells}"),
                ));
            }
            let slot = &mut cell_zone[c as usize];
            if *slot == NO_CELL {
                *slot = z as u32;
            }
        }
        mesh.zones.push(name.clone());
    }
    if cell_zone.contains(&NO_CELL) {
        let rest = mesh.zones.len() as u32;
        mesh.zones.push(unique_name(&mesh.zones, "cells"));
        for z in &mut cell_zone {
            if *z == NO_CELL {
                *z = rest;
            }
        }
    }
    mesh.cell_zone = cell_zone;

    mesh.metadata
        .insert("source_format".into(), "openfoam".into());
    mesh.metadata.insert(
        "openfoam_format".into(),
        match fh.format {
            Format::Ascii => "ascii",
            Format::Binary => "binary",
        }
        .into(),
    );
    if let Some(note) = fh.note {
        mesh.metadata.insert("openfoam_note".into(), note);
    }
    Ok(mesh)
}

/// The cell-face indices of cell `c` by the starts `cell_faces`.
fn cf_of(cell_faces: &[u32], c: usize) -> std::ops::Range<usize> {
    cell_faces[c] as usize..cell_faces[c + 1] as usize
}

fn unique_name(taken: &[String], base: &str) -> String {
    let mut name = base.to_string();
    let mut k = 2;
    while taken.contains(&name) {
        name = format!("{base}_{k}");
        k += 1;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/openfoam")
            .join(rel)
    }

    /// Sum of the area vectors of the faces of cell `c` (Newell's method):
    /// zero when every face of the cell turns outward.
    fn closure(m: &Mesh, c: usize) -> f64 {
        let mut s = [0.0f64; 3];
        for face in m.cell(c) {
            for i in 0..face.len() {
                let p = m.points[face.at(i) as usize];
                let q = m.points[face.at((i + 1) % face.len()) as usize];
                s[0] += (p[1] - q[1]) * (p[2] + q[2]);
                s[1] += (p[2] - q[2]) * (p[0] + q[0]);
                s[2] += (p[0] - q[0]) * (p[1] + q[1]);
            }
        }
        (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt()
    }

    /// The volume of cell `c` by the divergence theorem (fan triangles
    /// from each face's first node): positive when its faces turn outward.
    fn volume(m: &Mesh, c: usize) -> f64 {
        let mut v = 0.0;
        for face in m.cell(c) {
            let a = m.points[face.at(0) as usize];
            for k in 1..face.len().saturating_sub(1) {
                let (b, d) = (
                    m.points[face.at(k) as usize],
                    m.points[face.at(k + 1) as usize],
                );
                v += a[0] * (b[1] * d[2] - b[2] * d[1]) - a[1] * (b[0] * d[2] - b[2] * d[0])
                    + a[2] * (b[0] * d[1] - b[1] * d[0]);
            }
        }
        v / 6.0
    }

    /// The checks every polyMesh must pass: each cell closed with its faces
    /// outward, the topology consistent with the boundary and the cell
    /// faces, and the same as the hash-based linking of the VTU reader.
    fn check_topology(m: &Mesh) {
        for c in 0..m.cell_count() {
            assert!(closure(m, c) < 1e-9, "cell {c} is not closed");
            assert!(volume(m, c) > 0.0, "cell {c} turns inward");
        }
        for (b, &u) in m.boundary_face_id.iter().enumerate() {
            assert_eq!(m.face_neighbour[u as usize], NO_CELL);
            assert_eq!(m.face_boundary[u as usize], b as u32);
            assert_eq!(m.boundary_owner[b], m.face_owner[u as usize]);
        }
        for c in 0..m.cell_count() {
            for f in m.cell_face_range(c) {
                let u = m.cell_face_id[f] as usize;
                assert!(m.face_owner[u] == c as u32 || m.face_neighbour[u] == c as u32);
            }
        }
        let mut linked = m.clone();
        crate::boundary::link(&mut linked, m.cell_polygons());
        assert_eq!(linked.unique_faces, m.unique_faces);
        assert_eq!(linked.boundary_owner, m.boundary_owner);
        for f in 0..m.cell_face_id.len() {
            let (a, b) = (m.cell_face_id[f] as usize, linked.cell_face_id[f] as usize);
            let mut x = [m.face_owner[a], m.face_neighbour[a]];
            let mut y = [linked.face_owner[b], linked.face_neighbour[b]];
            x.sort_unstable();
            y.sort_unstable();
            assert_eq!(x, y, "cell face {f}");
        }
    }

    /// Invariant: KoflCraft's OpenFOAM export of the 4-cell box reads as the
    /// same mesh as its VTU hand-off: 4 cells in zone `fluid`, 6 patches,
    /// 16 boundary faces, the unit box, closed cells. A reader that lost a
    /// patch, reversed the neighbour faces wrongly or skipped cellZones
    /// would differ; the VTU tests cannot see the polyMesh path.
    #[test]
    fn reads_the_koflcraft_box() {
        let m = read_polymesh(&fixture("box")).unwrap();
        let v = crate::read(&fixture("../box/foehncraft.vtu")).unwrap();
        let (s, t) = (m.summary(), v.summary());
        assert_eq!(s.cells, 4);
        assert_eq!(s.boundary_faces, 16);
        assert_eq!(m.zones, ["fluid"]);
        assert_eq!(s.cell_kinds, t.cell_kinds);
        assert_eq!(s.patches, t.patches);
        assert_eq!(s.bounds, t.bounds);
        assert_eq!(m.unique_faces, 22);
        assert_eq!(m.files.len(), 6);
        assert_eq!(
            m.metadata.get("openfoam_format").map(String::as_str),
            Some("ascii")
        );
        check_topology(&m);
    }

    /// Invariant: the FoehnCraft channel (blockMesh 4 x 2 x 2 hexahedra)
    /// reads the same from ascii and from binary with a faceCompactList:
    /// 16 hexahedra, zones `right` and `left` of 8 cells, the patch roles
    /// from the types (`walls` is a wall) and the names (`inlet`,
    /// `outlet`). A broken binary or compact-list decoder would change
    /// the points, the faces or the zones.
    #[test]
    fn reads_ascii_and_binary_alike() {
        let a = read_polymesh(&fixture("channel_ascii")).unwrap();
        let b = read_polymesh(&fixture("channel_binary")).unwrap();
        for m in [&a, &b] {
            let s = m.summary();
            assert_eq!(s.cells, 16);
            assert_eq!(s.cell_kinds.get("hexahedron"), Some(&16));
            let zones: Vec<_> = s.zones.iter().map(|z| (z.name.as_str(), z.cells)).collect();
            assert_eq!(zones, [("right", 8), ("left", 8)]);
            let p: Vec<_> = s
                .patches
                .iter()
                .map(|p| (p.name.as_str(), p.role, p.faces))
                .collect();
            assert_eq!(
                p,
                [
                    ("inlet", PatchRole::Inlet, 4),
                    ("outlet", PatchRole::Outlet, 4),
                    ("walls", PatchRole::Wall, 16),
                    ("sides", PatchRole::Unspecified, 16),
                ]
            );
            check_topology(m);
        }
        assert_eq!(
            b.metadata.get("openfoam_format").map(String::as_str),
            Some("binary")
        );
        assert_eq!(a.distinct, b.distinct);
        assert_eq!(a.cell_face_flip, b.cell_face_flip);
        assert_eq!(a.cell_zone, b.cell_zone);
        for (p, q) in a.points.iter().zip(&b.points) {
            for k in 0..3 {
                assert!((p[k] - q[k]).abs() < 1e-12);
            }
        }
    }

    /// Invariant: the polyDualMesh of a tetrahedral cube reads as 81 true
    /// polyhedra, each closed with its faces outward. A neighbour face not
    /// reversed would leave the cells open (the closure test fails).
    #[test]
    fn reads_dual_polyhedra() {
        let m = read_polymesh(&fixture("dual_polyhedra")).unwrap();
        assert_eq!(m.cell_count(), 81);
        assert!(m.cell_kind.contains(&CellKind::Polyhedron));
        assert_eq!(m.zones, ["cells"]);
        check_topology(&m);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("murmel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Invariant: gzip-compressed files (`faces.gz`, as OpenFOAM writes
    /// with `writeCompression on`) read as the plain files do.
    #[test]
    fn reads_gzip_files() {
        use std::io::Write as _;
        let src = fixture("channel_binary/constant/polyMesh");
        let dir = temp_dir("polymesh-gz");
        for e in std::fs::read_dir(&src).unwrap() {
            let e = e.unwrap();
            let bytes = std::fs::read(e.path()).unwrap();
            let out =
                std::fs::File::create(dir.join(format!("{}.gz", e.file_name().to_string_lossy())))
                    .unwrap();
            let mut z = flate2::write::GzEncoder::new(out, flate2::Compression::fast());
            z.write_all(&bytes).unwrap();
            z.finish().unwrap();
        }
        let m = read_polymesh(&dir).unwrap();
        let b = read_polymesh(&src).unwrap();
        assert_eq!(m.distinct, b.distinct);
        assert_eq!(m.points, b.points);
        assert_eq!(m.zones, b.zones);
        assert!(
            m.files
                .iter()
                .all(|f| f.extension().is_some_and(|e| e == "gz"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Invariant: a case folder, its `constant` folder, the `polyMesh`
    /// folder, a `.foam` file in the case and a file of the polyMesh all
    /// name the same mesh; a folder without one gives MESH013. The window
    /// opens folders and the file dialog `.foam` files, so each path must
    /// work.
    #[test]
    fn finds_the_polymesh_folder() {
        let case = fixture("channel_ascii");
        let poly = case.join("constant/polyMesh");
        for p in [case.clone(), case.join("constant"), poly.join("faces")] {
            assert!(crate::polymesh::is_openfoam(&p));
            assert_eq!(polymesh_dir(&p).unwrap(), poly);
            assert_eq!(crate::read(&p).unwrap().cell_count(), 16);
        }
        let dir = temp_dir("polymesh-find");
        std::fs::write(dir.join("case.foam"), "").unwrap();
        assert_eq!(
            read_polymesh(&dir.join("case.foam")).unwrap_err().code,
            "MESH013"
        );
        assert_eq!(read_polymesh(&dir).unwrap_err().code, "MESH013");
        assert_eq!(crate::read(&dir.join("none")).unwrap_err().code, "MESH001");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const HEAD: &str = "FoamFile { version 2.0; format ascii; class labelList; object x; }\n";

    /// A single tetrahedron as polyMesh files, ascii.
    fn tet() -> [String; 5] {
        [
            format!("{HEAD}4((0 0 0)(1 0 0)(0 1 0)(0 0 1))"),
            format!("{HEAD}4(3(0 2 1) 3(0 1 3) 3(1 2 3) 3(0 3 2))"),
            format!("{HEAD}4(0 0 0 0)"),
            format!("{HEAD}0()"),
            format!(
                "{HEAD}1(walls {{ type wall; inGroups List<word> 1(wall); nFaces 4; startFace 0; }})"
            ),
        ]
    }

    fn code(files: &[String; 5], zones: Option<&str>) -> &'static str {
        match read_polymesh_bytes(PolyMeshFiles {
            points: files[0].as_bytes(),
            faces: files[1].as_bytes(),
            owner: files[2].as_bytes(),
            neighbour: files[3].as_bytes(),
            boundary: files[4].as_bytes(),
            cell_zones: zones.map(str::as_bytes),
        }) {
            Ok(_) => "ok",
            Err(e) => e.code,
        }
    }

    fn with(i: usize, from: &str, to: &str) -> [String; 5] {
        let mut f = tet();
        assert!(f[i].contains(from), "{from} not in file {i}");
        f[i] = f[i].replacen(from, to, 1);
        f
    }

    /// The broken-input corpus of the polyMesh reader (AGENTS.md: the
    /// input of every reader failure joins it). Each case must give its
    /// code, never a panic or a huge allocation.
    #[test]
    fn broken_inputs_give_codes() {
        assert_eq!(code(&tet(), None), "ok");
        // Header and syntax.
        assert_eq!(code(&with(0, "FoamFile", "Foam"), None), "MESH014");
        assert_eq!(
            code(&with(0, "format ascii", "format gzip"), None),
            "MESH007"
        );
        assert_eq!(code(&with(2, "4(0 0 0 0)", "4(0 0 0 0 ;"), None), "MESH014");
        assert_eq!(code(&with(2, "4(0 0 0 0)", "4(0 0 0 0"), None), "MESH004");
        assert_eq!(code(&with(2, "4(0 0 0 0)", "4{0}"), None), "MESH014");
        assert_eq!(code(&with(4, "1(walls {", "1(walls ) {"), None), "MESH014");
        assert_eq!(
            code(&with(0, "4((", "/* a (comment) */ 4( // x\n("), None),
            "ok"
        );
        // Counts larger than the file (KoflCraft SW-17).
        assert_eq!(
            code(&with(2, "4(0 0 0 0)", "99999999999(0 0 0 0)"), None),
            "MESH004"
        );
        assert_eq!(code(&with(0, "4((", "4000000000(("), None), "MESH004");
        // Values.
        assert_eq!(code(&with(0, "(1 0 0)", "(1 x 0)"), None), "MESH005");
        assert_eq!(code(&with(0, "(1 0 0)", "(1 nan 0)"), None), "MESH005");
        assert_eq!(code(&with(2, "4(0 0 0 0)", "3(0 0 0)"), None), "MESH005");
        assert_eq!(code(&with(1, "3(0 2 1)", "2(0 2)"), None), "MESH005");
        // Indices.
        assert_eq!(code(&with(1, "3(0 2 1)", "3(0 2 9)"), None), "MESH006");
        assert_eq!(code(&with(2, "4(0 0 0 0)", "4(0 0 0 -1)"), None), "MESH006");
        assert_eq!(code(&with(2, "4(0 0 0 0)", "4(0 0 0 1)"), None), "MESH006");
        assert_eq!(
            code(&with(2, "4(0 0 0 0)", "4(0 0 0 2000000000)"), None),
            "MESH006"
        );
        assert_eq!(code(&with(3, "0()", "1(0)"), None), "MESH006");
        // Patches.
        assert_eq!(code(&with(4, "nFaces 4", "nFaces 3"), None), "MESH015");
        assert_eq!(code(&with(4, "nFaces 4", "nFaces 5"), None), "MESH015");
        assert_eq!(code(&with(4, "nFaces 4;", ""), None), "MESH015");
        assert_eq!(
            code(
                &with(
                    4,
                    "1(walls { type wall; inGroups List<word> 1(wall); nFaces 4; startFace 0; })",
                    "2(a { type patch; nFaces 2; startFace 0; } b { type patch; nFaces 3; startFace 1; })"
                ),
                None
            ),
            "MESH015"
        );
        // Zones.
        let zone = |cells: &str| {
            format!("{HEAD}1(z {{ type cellZone; cellLabels List<label> {cells}; }})")
        };
        assert_eq!(code(&tet(), Some(&zone("1(0)"))), "ok");
        assert_eq!(code(&tet(), Some(&zone("1(4)"))), "MESH006");
        assert_eq!(code(&tet(), Some(&zone("1(0"))), "MESH014");
        // Binary lists cut short.
        let bin = format!(
            "{}4(\u{0}\u{0}\u{0}\u{0})",
            HEAD.replace("format ascii", "format binary")
        );
        let mut f = tet();
        f[2] = bin;
        assert_eq!(code(&f, None), "MESH004");
        // No cells at all.
        let mut f = tet();
        f[1] = format!("{HEAD}0()");
        f[2] = format!("{HEAD}0()");
        f[4] = format!("{HEAD}0()");
        assert_eq!(code(&f, None), "MESH010");
    }

    /// Invariant: binary labels and scalars decode for both widths and
    /// byte orders the header's `arch` names.
    #[test]
    fn decodes_binary_widths_and_orders() {
        let mut h = Header {
            format: Format::Binary,
            class: String::new(),
            note: None,
            label_bytes: 4,
            scalar_bytes: 8,
            big_endian: false,
        };
        arch(&mut h, "MSB;label=64;scalar=32", "x").unwrap();
        assert_eq!((h.big_endian, h.label_bytes, h.scalar_bytes), (true, 8, 4));
        assert_eq!(decode_int(&(-7i64).to_be_bytes(), true), -7);
        assert_eq!(decode_int(&(-7i32).to_le_bytes(), false), -7);
        assert_eq!(decode_float(&1.5f32.to_be_bytes(), true), 1.5);
        assert_eq!(decode_float(&(-2.25f64).to_le_bytes(), false), -2.25);
        assert_eq!(
            arch(&mut h, "LSB;label=16", "x").unwrap_err().code,
            "MESH007"
        );
    }

    /// Invariant: roles come from the OpenFOAM type first, then from the
    /// name as KoflCraft names patches (FC-04).
    #[test]
    fn roles_from_type_and_name() {
        assert_eq!(patch_role("hull", "wall"), PatchRole::Wall);
        assert_eq!(patch_role("mid", "symmetryPlane"), PatchRole::Symmetry);
        assert_eq!(patch_role("west_inlet", "patch"), PatchRole::Inlet);
        assert_eq!(patch_role("Outlet", "patch"), PatchRole::Outlet);
        assert_eq!(patch_role("pipe_walls", "patch"), PatchRole::Wall);
        assert_eq!(patch_role("xmin", "patch"), PatchRole::Unspecified);
        assert_eq!(patch_role("front", "empty"), PatchRole::Unspecified);
    }
}
