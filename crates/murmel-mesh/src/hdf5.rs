//! A reader for the part of HDF5 that VTKHDF files use, written from the
//! published "HDF5 File Format Specification Version 3.0" (The HDF Group);
//! no HDF5 library source was read.
//!
//! Supported: superblocks 0 to 3; object headers 1 and 2 with
//! continuation blocks; old-style groups (symbol table: version 1 B-tree
//! and local heap) and new-style groups with links in the header or in
//! dense storage (fractal heap and version 2 B-tree); hard and soft links;
//! attributes in the header or in dense storage; datatypes fixed-point,
//! floating-point (IEEE, either byte order), fixed and variable-length
//! strings (global heap); dataspaces scalar, simple and null; layouts
//! compact, contiguous and chunked (version 1 B-tree index, and the single
//! chunk, implicit and unpaged fixed-array indices of layout version 4);
//! filters deflate, shuffle and Fletcher-32.
//!
//! Not supported (`MESH017`): extensible-array and version 2 B-tree chunk
//! indices, paged fixed arrays, huge fractal-heap objects, filtered
//! fractal heaps, other filters (szip, nbit, scale-offset), virtual
//! datasets and external files.
//!
//! A file opened by its path is not held in memory: [`H5::open`] reads
//! the file's structure (superblock, object headers, groups, attributes,
//! chunk indices) in pages until every object of it is found, and the
//! data of a dataset is read from the file when it is asked for
//! (positional reads, on all threads for large datasets). A file given
//! as bytes ([`H5::from_bytes`]) is read from memory. Every address and
//! size is checked against the file before it is used, so a broken file
//! gives `MESH016` and never a panic or an allocation larger than the
//! file allows.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read as _;
use std::ops::Range;
use std::sync::Mutex;

use crate::MeshError;

const SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";

/// Largest number of continuation blocks, B-tree nodes or heap blocks
/// one walk visits; a cycle in a broken file stops here.
const MAX_VISITS: usize = 1 << 20;

fn broken(message: impl Into<String>) -> MeshError {
    MeshError::new("MESH016", message)
}

fn not_hdf5() -> MeshError {
    MeshError::new("MESH018", "the file is not an HDF5 file")
}

fn unsupported(message: impl Into<String>) -> MeshError {
    MeshError::new("MESH017", message)
}

/// Bytes needed to store `n` (at least one).
fn enc_size(n: u64) -> usize {
    (64 - n.max(1).leading_zeros() as usize).div_ceil(8).max(1)
}

fn log2_exact(n: u64) -> Option<u32> {
    n.is_power_of_two().then(|| n.trailing_zeros())
}

/// A read position in the file: position `at + p` of the file, in a run
/// of its bytes `d` that starts at position `at`.
#[derive(Clone)]
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
    at: usize,
    o: usize,
    l: usize,
    /// Where to note the position after `d` when a read runs past it and
    /// the file goes on there unread (while the file is being opened).
    holes: Option<&'a Mutex<Vec<usize>>>,
}

impl<'a> Cur<'a> {
    /// The position in the file.
    fn pos(&self) -> usize {
        self.at + self.p
    }

    /// Whether `n` bytes can be read here; notes the hole when not.
    fn has(&self, n: usize) -> bool {
        let ok = self.p.checked_add(n).is_some_and(|e| e <= self.d.len());
        if !ok && let Some(h) = self.holes {
            note(h, self.at + self.d.len());
        }
        ok
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], MeshError> {
        if !self.has(n) {
            return Err(broken(format!(
                "read of {n} bytes at {} runs past the end",
                self.pos()
            )));
        }
        let end = self.p + n;
        let s = &self.d[self.p..end];
        self.p = end;
        Ok(s)
    }

    fn skip(&mut self, n: usize) -> Result<(), MeshError> {
        self.bytes(n).map(|_| ())
    }

    fn uint(&mut self, n: usize) -> Result<u64, MeshError> {
        let b = self.bytes(n)?;
        let mut v = 0u64;
        for (i, &x) in b.iter().enumerate().take(8) {
            v |= (x as u64) << (8 * i);
        }
        Ok(v)
    }

    fn u8(&mut self) -> Result<u8, MeshError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MeshError> {
        Ok(self.uint(2)? as u16)
    }

    fn u32(&mut self) -> Result<u32, MeshError> {
        Ok(self.uint(4)? as u32)
    }

    /// An address; `None` for the undefined address (all bits set).
    fn addr(&mut self) -> Result<Option<u64>, MeshError> {
        let o = self.o;
        let v = self.uint(o)?;
        let undef = if o >= 8 {
            u64::MAX
        } else {
            (1u64 << (8 * o)) - 1
        };
        Ok((v != undef).then_some(v))
    }

    fn len(&mut self) -> Result<u64, MeshError> {
        let l = self.l;
        self.uint(l)
    }

    fn sig(&mut self, s: &[u8; 4]) -> Result<(), MeshError> {
        let at = self.p;
        if self.bytes(4)? != s {
            return Err(broken(format!(
                "expected {} at {at}",
                String::from_utf8_lossy(s)
            )));
        }
        Ok(())
    }
}

/// An open HDF5 file.
pub struct H5 {
    /// The runs of the file's bytes read so far, by position, in order
    /// and apart: the whole file for [`H5::from_bytes`], its structure
    /// for [`H5::open`].
    parts: Vec<Part>,
    /// The file, for data outside `parts`.
    file: Option<File>,
    len: usize,
    o: usize,
    l: usize,
    base: u64,
    root: u64,
    /// Positions a parse needed outside `parts` (while opening).
    holes: Mutex<Vec<usize>>,
    /// The bytes read first, and at least at each hole.
    page: usize,
}

/// A run of the file's bytes from position `at`.
struct Part {
    at: usize,
    bytes: Vec<u8>,
}

impl Part {
    fn end(&self) -> usize {
        self.at + self.bytes.len()
    }
}

/// Note a position a parse needed and did not find.
fn note(holes: &Mutex<Vec<usize>>, pos: usize) {
    if let Ok(mut h) = holes.lock()
        && h.len() < MAX_HOLES
    {
        h.push(pos);
    }
}

/// The bytes the file reads first, and then at least at each hole: most
/// writers keep a file's structure together, and a page holds many of
/// its pieces.
const PAGE: usize = 1 << 16;
/// Holes noted in one round of opening; more are found in the next.
const MAX_HOLES: usize = 1 << 12;
/// Rounds of opening: each finds the pieces of the structure that the
/// pieces found before point to.
const MAX_ROUNDS: usize = 64;
/// Bytes of data read in one piece by one thread.
const READ_PIECE: usize = 1 << 23;

/// Fill `buf` from position `pos` of `f`.
#[cfg(unix)]
fn read_exact_at(f: &File, buf: &mut [u8], pos: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, pos)
}

/// Fill `buf` from position `pos` of `f`.
#[cfg(windows)]
fn read_exact_at(f: &File, mut buf: &mut [u8], mut pos: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match f.seek_read(buf, pos) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = &mut buf[n..];
                pos += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Fill `buf` from position `pos` of `f`.
#[cfg(not(any(unix, windows)))]
fn read_exact_at(_: &File, _: &mut [u8], _: u64) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "positional reads are not supported here",
    ))
}

fn cannot_read(e: std::io::Error) -> MeshError {
    MeshError::new("MESH001", format!("cannot read the file: {e}"))
}

/// A datatype the reader converts.
#[derive(Clone, Debug, PartialEq)]
pub enum Dtype {
    Int {
        size: usize,
        signed: bool,
        be: bool,
    },
    Float {
        size: usize,
        be: bool,
    },
    FixedString {
        size: usize,
    },
    VarString,
    /// Any other class (compound, array, reference, …), by class number.
    Other(u8),
}

impl Dtype {
    pub fn size(&self) -> usize {
        match self {
            Dtype::Int { size, .. } | Dtype::Float { size, .. } | Dtype::FixedString { size } => {
                *size
            }
            Dtype::VarString | Dtype::Other(_) => 0,
        }
    }
}

/// A link of a group to an object.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub name: String,
    pub target: Target,
    /// The creation order, when the group tracks it.
    pub order: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The object header address.
    Hard(u64),
    Soft(String),
    External,
}

/// The value of an attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(Vec<String>),
    Int(Vec<i64>),
    Float(Vec<f64>),
    /// A type the reader does not convert.
    Other,
}

impl Value {
    /// The first string, or the first number written as text.
    pub fn text(&self) -> Option<String> {
        match self {
            Value::Str(v) => v.first().cloned(),
            Value::Int(v) => v.first().map(|x| x.to_string()),
            Value::Float(v) => v.first().map(|x| x.to_string()),
            Value::Other => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Int(v) => v.first().map(|&x| x as f64),
            Value::Float(v) => v.first().copied(),
            Value::Str(v) => v.first().and_then(|s| s.trim().parse().ok()),
            Value::Other => None,
        }
    }
}

/// A header message: its type and the range of its data in the file.
#[derive(Clone, Debug)]
struct Msg {
    ty: u16,
    data: Range<usize>,
}

/// An object (group or dataset) by its header messages.
#[derive(Clone, Debug)]
pub struct Object {
    pub addr: u64,
    msgs: Vec<Msg>,
}

#[derive(Clone, Debug)]
enum Layout {
    Compact(Range<usize>),
    Contiguous {
        addr: Option<u64>,
        size: u64,
    },
    /// Chunked; `dims` are the chunk's dimensions without the element
    /// size.
    Chunked {
        dims: Vec<u64>,
        index: ChunkIndex,
    },
}

/// A chunk of a chunked dataset.
struct Chunk {
    /// Its offsets in elements.
    off: Vec<u64>,
    addr: u64,
    /// Its size in the file (filtered).
    size: u64,
    mask: u32,
}

#[derive(Clone, Debug)]
enum ChunkIndex {
    BTree1(Option<u64>),
    Single {
        addr: Option<u64>,
        filtered_size: Option<u64>,
        mask: u32,
    },
    Implicit(Option<u64>),
    FixedArray(Option<u64>),
}

#[derive(Clone, Debug)]
struct Filter {
    id: u16,
    data: Vec<u32>,
}

/// How the values of a numeric dataset are stored.
#[derive(Clone, Copy, Debug)]
enum Num {
    I64Le,
    F64Le,
    Int {
        size: usize,
        signed: bool,
        be: bool,
    },
    F32 {
        be: bool,
    },
    F64Be,
    /// Sizes the readers do not convert (no values).
    Unread,
}

/// The values of a numeric dataset, read where they lie ([`H5::values`]),
/// or some of them ([`H5::values_in`]): values `first..` of the dataset's
/// `len`.
pub struct Values<'a> {
    bytes: Cow<'a, [u8]>,
    num: Num,
    len: usize,
    first: usize,
}

impl<'a> Values<'a> {
    /// The values of `bytes` stored as `dtype`; `None` when it is not a
    /// number type. A size the readers do not convert gives no values, as
    /// [`H5::read_i64`] and [`H5::read_f64`] give none.
    pub(crate) fn new(bytes: Cow<'a, [u8]>, dtype: &Dtype) -> Option<Values<'a>> {
        let num = match *dtype {
            Dtype::Int {
                size: 8, be: false, ..
            } => Num::I64Le,
            Dtype::Float { size: 8, be: false } => Num::F64Le,
            Dtype::Int { size, signed, be } if (1..=8).contains(&size) => {
                Num::Int { size, signed, be }
            }
            Dtype::Float { size: 4, be } => Num::F32 { be },
            Dtype::Float { size: 8, be: true } => Num::F64Be,
            Dtype::Int { .. } | Dtype::Float { .. } => Num::Unread,
            _ => return None,
        };
        let len = match num {
            Num::Unread => 0,
            _ => bytes.len() / dtype.size(),
        };
        Some(Values {
            bytes,
            num,
            len,
            first: 0,
        })
    }

    /// The values `first..` of a dataset of `len` values, from `bytes`
    /// stored as `dtype`.
    fn window(bytes: Cow<'a, [u8]>, dtype: &Dtype, first: usize, len: usize) -> Option<Values<'a>> {
        let mut v = Values::new(bytes, dtype)?;
        if !matches!(v.num, Num::Unread) {
            v.first = first;
            v.len = len;
        }
        Some(v)
    }

    /// No values (of a dataset that is not there).
    pub(crate) fn none() -> Values<'static> {
        Values {
            bytes: Cow::Borrowed(&[]),
            num: Num::Unread,
            len: 0,
            first: 0,
        }
    }

    /// Values `rows` of `all`, for tests: a window of a dataset of floats.
    #[cfg(test)]
    pub(crate) fn of_floats_in(all: &[f64], rows: Range<usize>) -> Values<'static> {
        let mut v = Values::of_floats(&all[rows.clone()]);
        v.first = rows.start;
        v.len = all.len();
        v
    }

    /// Values `first..` of `all`, for tests: a window of a dataset.
    #[cfg(test)]
    pub(crate) fn of_ints_in(all: &[i64], rows: Range<usize>) -> Values<'static> {
        let mut v = Values::of_ints(&all[rows.clone()]);
        v.first = rows.start;
        v.len = all.len();
        v
    }

    /// Values of 64-bit integers, for tests.
    #[cfg(test)]
    pub(crate) fn of_ints(v: &[i64]) -> Values<'static> {
        let bytes = v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let dtype = Dtype::Int {
            size: 8,
            signed: true,
            be: false,
        };
        Values::new(Cow::Owned(bytes), &dtype).unwrap_or_else(|| unreachable!())
    }

    /// Values of 64-bit floats, for tests.
    #[cfg(test)]
    pub(crate) fn of_floats(v: &[f64]) -> Values<'static> {
        let bytes = v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let dtype = Dtype::Float { size: 8, be: false };
        Values::new(Cow::Owned(bytes), &dtype).unwrap_or_else(|| unreachable!())
    }
}

impl Values<'_> {
    /// How many values the dataset has (those held may be fewer).
    pub fn len(&self) -> usize {
        self.len
    }

    /// The values held: `held().contains(&i)` for each `i` that
    /// [`Values::int`] and [`Values::float`] take.
    pub fn held(&self) -> Range<usize> {
        let size = match self.num {
            Num::I64Le | Num::F64Le | Num::F64Be => 8,
            Num::F32 { .. } => 4,
            Num::Int { size, .. } => size,
            Num::Unread => return 0..0,
        };
        self.first..self.first + self.bytes.len() / size
    }

    /// The same values, borrowed.
    pub fn view(&self) -> Values<'_> {
        Values {
            bytes: Cow::Borrowed(&self.bytes),
            num: self.num,
            len: self.len,
            first: self.first,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Value `i` as an integer (a float is truncated); `i` is held.
    pub fn int(&self, i: usize) -> i64 {
        let i = i - self.first;
        match self.num {
            Num::I64Le => i64::from_le_bytes(self.eight(i)),
            Num::Int { size, signed, be } => {
                let b = &self.bytes[i * size..(i + 1) * size];
                let mut v = 0u64;
                for k in 0..size {
                    let byte = if be { b[size - 1 - k] } else { b[k] };
                    v |= (byte as u64) << (8 * k);
                }
                if signed && size < 8 && v & (1 << (8 * size - 1)) != 0 {
                    v |= u64::MAX << (8 * size);
                }
                v as i64
            }
            Num::Unread => 0,
            _ => self.float(i + self.first) as i64,
        }
    }

    /// Value `i` as a float (an integer converts); `i` is held.
    pub fn float(&self, i: usize) -> f64 {
        let i = i - self.first;
        match self.num {
            Num::F64Le => f64::from_le_bytes(self.eight(i)),
            Num::F32 { be } => {
                let mut a = [0u8; 4];
                a.copy_from_slice(&self.bytes[i * 4..i * 4 + 4]);
                (if be {
                    f32::from_be_bytes(a)
                } else {
                    f32::from_le_bytes(a)
                }) as f64
            }
            Num::F64Be => f64::from_be_bytes(self.eight(i)),
            Num::Unread => 0.0,
            _ => self.int(i + self.first) as f64,
        }
    }

    fn eight(&self, i: usize) -> [u8; 8] {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.bytes[i * 8..i * 8 + 8]);
        a
    }
}

/// What is needed to read a dataset.
#[derive(Clone, Debug)]
pub struct Dataset {
    pub dtype: Dtype,
    /// Dimensions; empty for a scalar.
    pub dims: Vec<u64>,
    /// No elements at all (a null dataspace).
    pub null: bool,
    layout: Layout,
    filters: Vec<Filter>,
}

impl Dataset {
    /// The number of elements.
    pub fn count(&self) -> u64 {
        if self.null {
            0
        } else {
            self.dims.iter().product()
        }
    }
}

impl H5 {
    /// Open an HDF5 file from its bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<H5, MeshError> {
        // The superblock is at 0, 512, 1024, 2048, …
        let mut at = None;
        let mut s = 0usize;
        while s + 8 <= data.len() {
            if &data[s..s + 8] == SIGNATURE {
                at = Some(s);
                break;
            }
            s = if s == 0 { 512 } else { s * 2 };
        }
        let at = at.ok_or_else(not_hdf5)?;
        let len = data.len();
        H5::with(vec![Part { at: 0, bytes: data }], None, len, at)
    }

    /// Open the HDF5 file at `path`: its structure is read now, the data
    /// of its datasets when they are read.
    pub fn open(path: &std::path::Path) -> Result<H5, MeshError> {
        H5::open_paged(path, PAGE)
    }

    /// [`H5::open`], reading the structure in pages of `page` bytes.
    fn open_paged(path: &std::path::Path, page: usize) -> Result<H5, MeshError> {
        let cannot = |e: std::io::Error| {
            MeshError::new("MESH001", format!("cannot read {}: {e}", path.display()))
        };
        let file = File::open(path).map_err(cannot)?;
        let len = file
            .metadata()
            .map_err(cannot)
            .and_then(|m| usize::try_from(m.len()).map_err(|_| not_hdf5()))?;
        // The superblock is at 0, 512, 1024, 2048, …
        let mut s = 0usize;
        let at = loop {
            if s + 8 > len {
                return Err(not_hdf5());
            }
            let mut sig = [0u8; 8];
            read_exact_at(&file, &mut sig, s as u64).map_err(cannot)?;
            if &sig == SIGNATURE {
                break s;
            }
            s = if s == 0 { 512 } else { s * 2 };
        };
        let mut parts = Vec::new();
        for start in [0, at] {
            let end = (start + page.max(4096)).min(len);
            let mut bytes = vec![0; end - start];
            read_exact_at(&file, &mut bytes, start as u64).map_err(cannot)?;
            parts.push(Part { at: start, bytes });
            if at < end {
                break;
            }
        }
        let mut h = H5::with(parts, Some(file), len, at)?;
        h.page = page.max(1);
        h.load_structure();
        Ok(h)
    }

    /// The file whose superblock is at `at`, with the `parts` read so far
    /// (they hold the superblock).
    fn with(parts: Vec<Part>, file: Option<File>, len: usize, at: usize) -> Result<H5, MeshError> {
        let part = parts
            .iter()
            .find(|p| p.at <= at && at < p.end())
            .ok_or_else(not_hdf5)?;
        let mut c = Cur {
            d: &part.bytes,
            p: at - part.at + 8,
            at: part.at,
            o: 8,
            l: 8,
            holes: None,
        };
        let version = c.u8()?;
        let (o, l, base, root);
        match version {
            0 | 1 => {
                c.skip(4)?; // free space, root entry and shared header versions
                o = c.u8()? as usize;
                l = c.u8()? as usize;
                c.skip(1)?;
                c.skip(4)?; // leaf and internal node K
                c.skip(4)?; // consistency flags
                if version == 1 {
                    c.skip(4)?;
                }
                check_sizes(o, l)?;
                c.o = o;
                c.l = l;
                base = c.addr()?.unwrap_or(0);
                c.addr()?; // free space
                c.addr()?; // end of file
                c.addr()?; // driver information
                // root group symbol table entry
                c.addr()?; // link name offset
                root = c
                    .addr()?
                    .ok_or_else(|| broken("the root group has no object header"))?;
            }
            2 | 3 => {
                o = c.u8()? as usize;
                l = c.u8()? as usize;
                check_sizes(o, l)?;
                c.o = o;
                c.l = l;
                c.skip(1)?; // consistency flags
                base = c.addr()?.unwrap_or(0);
                c.addr()?; // superblock extension
                c.addr()?; // end of file
                root = c
                    .addr()?
                    .ok_or_else(|| broken("the root group has no object header"))?;
            }
            v => return Err(unsupported(format!("HDF5 superblock version {v}"))),
        }
        Ok(H5 {
            parts,
            file,
            len,
            o,
            l,
            base,
            root,
            holes: Mutex::new(Vec::new()),
            page: PAGE,
        })
    }

    /// Read the file's structure: walk every object from the root, note
    /// what the walk needed and had not read, read it (a page at least,
    /// or twice the run it continues), and walk again until nothing is
    /// missing. A file whose structure is broken keeps what was found;
    /// its broken objects fail when they are read.
    fn load_structure(&mut self) {
        for _ in 0..MAX_ROUNDS {
            self.walk();
            let mut holes = std::mem::take(self.holes.get_mut().unwrap_or_else(|e| e.into_inner()));
            if holes.is_empty() {
                return;
            }
            holes.sort_unstable();
            holes.dedup();
            for pos in holes {
                if self.read_part(pos).is_err() {
                    return;
                }
            }
        }
    }

    /// Read the bytes from `pos`: a page, or as many as the run that
    /// ends there, joined with the runs it meets.
    fn read_part(&mut self, pos: usize) -> std::io::Result<()> {
        if pos >= self.len || self.parts.iter().any(|p| p.at <= pos && pos < p.end()) {
            return Ok(());
        }
        let grow = self
            .parts
            .iter()
            .find(|p| p.end() == pos)
            .map_or(self.page, |p| p.bytes.len().max(self.page));
        let mut start = pos - pos % self.page;
        let mut end = (pos + grow).min(self.len);
        // join the runs this one meets or touches
        let (meet, keep): (Vec<Part>, Vec<Part>) = std::mem::take(&mut self.parts)
            .into_iter()
            .partition(|p| p.at <= end && start <= p.end());
        for p in &meet {
            start = start.min(p.at);
            end = end.max(p.end());
        }
        self.parts = keep;
        let mut bytes = vec![0; end - start];
        let read = match &self.file {
            Some(f) => read_exact_at(f, &mut bytes, start as u64),
            None => Err(std::io::Error::other("no file")),
        };
        if read.is_err() {
            // keep what was read before
            self.parts.extend(meet);
            self.parts.sort_by_key(|p| p.at);
            return read;
        }
        self.parts.push(Part { at: start, bytes });
        self.parts.sort_by_key(|p| p.at);
        Ok(())
    }

    /// Walk every object reachable from the root, as the readers will:
    /// headers, links, attributes, dataset descriptions and chunk
    /// indices, and the strings of variable-length string datasets. A
    /// part that fails is passed over.
    fn walk(&self) {
        let mut seen = HashSet::new();
        let mut stack = vec![self.root];
        while let Some(addr) = stack.pop() {
            if !seen.insert(addr) || seen.len() > MAX_VISITS {
                continue;
            }
            let Ok(obj) = self.object(addr) else {
                continue;
            };
            let _ = self.attrs(&obj);
            if self.is_group(&obj) {
                if let Ok(links) = self.links(&obj) {
                    stack.extend(links.into_iter().filter_map(|l| match l.target {
                        Target::Hard(a) => Some(a),
                        _ => None,
                    }));
                }
            } else if let Ok(ds) = self.dataset(&obj) {
                if let Layout::Chunked { dims, index } = &ds.layout {
                    let _ = self.chunk_list(&ds, dims, index);
                }
                if ds.dtype == Dtype::VarString {
                    let _ = self.read_strings(&ds);
                }
            }
        }
    }

    /// The length of the file in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The run holding positions `start..end`.
    fn part(&self, start: usize, end: usize) -> Option<&Part> {
        let i = self.parts.partition_point(|p| p.end() <= start);
        self.parts
            .get(i)
            .filter(|p| p.at <= start && end <= p.end())
            .or_else(|| {
                // an empty read at a run's end
                (start == end)
                    .then(|| self.parts.iter().find(|p| p.end() == start))
                    .flatten()
            })
    }

    /// Note a hole at `pos` while the file is opened.
    fn hole(&self, pos: usize) {
        if self.file.is_some() {
            note(&self.holes, pos);
        }
    }

    /// The first position of `start..end` that no run holds.
    fn first_missing(&self, start: usize) -> usize {
        match self.parts.iter().find(|p| p.at <= start && start < p.end()) {
            Some(p) => p.end(),
            None => start,
        }
    }

    /// A cursor at the position of `addr`, over the run that holds it.
    fn cur(&self, addr: u64) -> Result<Cur<'_>, MeshError> {
        let pos = self
            .base
            .checked_add(addr)
            .and_then(|a| usize::try_from(a).ok())
            .filter(|&a| a <= self.len)
            .ok_or_else(|| broken(format!("address {addr} is outside the file")))?;
        let Some(part) = self.part(pos, pos) else {
            self.hole(pos);
            return Err(broken(format!(
                "address {addr} is not in the structure read from the file"
            )));
        };
        let open = self.file.is_some() && part.end() < self.len;
        Ok(Cur {
            d: &part.bytes,
            p: pos - part.at,
            at: part.at,
            o: self.o,
            l: self.l,
            holes: open.then_some(&self.holes),
        })
    }

    /// A cursor over positions `start..end` of the file.
    fn cur_in(&self, start: usize, end: usize) -> Result<Cur<'_>, MeshError> {
        if start > end || end > self.len {
            return Err(broken("a piece of the structure runs past the end"));
        }
        let Some(part) = self.part(start, end) else {
            self.hole(self.first_missing(start));
            return Err(broken(
                "a piece of the structure is not in what was read from the file",
            ));
        };
        Ok(Cur {
            d: &part.bytes[..end - part.at],
            p: start - part.at,
            at: part.at,
            o: self.o,
            l: self.l,
            holes: None,
        })
    }

    /// A cursor over bytes that are not where they lie in the file.
    fn local<'a>(&self, d: &'a [u8]) -> Cur<'a> {
        Cur {
            d,
            p: 0,
            at: 0,
            o: self.o,
            l: self.l,
            holes: None,
        }
    }

    /// A cursor over a message's data (empty if it was not read).
    fn msg_cur(&self, r: &Range<usize>) -> Cur<'_> {
        self.cur_in(r.start, r.end)
            .unwrap_or_else(|_| self.local(&[]))
    }

    /// A message's bytes (empty if they were not read).
    fn msg_bytes(&self, r: &Range<usize>) -> &[u8] {
        self.part(r.start, r.end)
            .map_or(&[], |p| &p.bytes[r.start - p.at..r.end - p.at])
    }

    /// The bytes of the structure at `addr`, `n` long.
    fn slice(&self, addr: u64, n: u64) -> Result<&[u8], MeshError> {
        let n = usize::try_from(n).map_err(|_| broken("size too large"))?;
        let start = self
            .base
            .checked_add(addr)
            .and_then(|a| usize::try_from(a).ok())
            .ok_or_else(|| broken(format!("address {addr} is outside the file")))?;
        let end = start
            .checked_add(n)
            .ok_or_else(|| broken("size too large"))?;
        let c = self.cur_in(start, end)?;
        Ok(&c.d[c.p..])
    }

    /// The data at `addr`, `n` bytes: where it lies when it was read with
    /// the structure, else read from the file (large pieces on all
    /// threads).
    fn data(&self, addr: u64, n: u64) -> Result<Cow<'_, [u8]>, MeshError> {
        let start = self
            .base
            .checked_add(addr)
            .and_then(|a| usize::try_from(a).ok());
        let end = start
            .zip(usize::try_from(n).ok())
            .and_then(|(s, n)| s.checked_add(n));
        let (Some(start), Some(end)) = (start, end) else {
            return Err(broken("data runs past the end of the file"));
        };
        if end > self.len {
            return Err(broken(format!(
                "{n} bytes of data at {addr} run past the end of the file"
            )));
        }
        if let Some(p) = self.part(start, end) {
            return Ok(Cow::Borrowed(&p.bytes[start - p.at..end - p.at]));
        }
        let Some(f) = &self.file else {
            return Err(broken("data outside the file"));
        };
        let mut bytes = vec![0u8; end - start];
        let ranges = crate::par::ranges(bytes.len(), READ_PIECE);
        let cuts: Vec<usize> = ranges
            .iter()
            .map(|r| r.start)
            .chain([bytes.len()])
            .collect();
        let pieces: Vec<_> = ranges
            .into_iter()
            .zip(crate::par::split_mut(&mut bytes, &cuts))
            .collect();
        for read in crate::par::each(pieces, |(r, piece)| {
            read_exact_at(f, piece, (start + r.start) as u64)
        }) {
            read.map_err(cannot_read)?;
        }
        Ok(Cow::Owned(bytes))
    }

    pub fn root(&self) -> Result<Object, MeshError> {
        self.object(self.root)
    }

    /// The object at a header address.
    pub fn object(&self, addr: u64) -> Result<Object, MeshError> {
        let mut c = self.cur(addr)?;
        let mut msgs = Vec::new();
        if !c.has(4) {
            return Err(broken(format!("object header at {addr} runs past the end")));
        }
        if c.d.get(c.p..c.p + 4) == Some(b"OHDR") {
            self.header_v2(&mut c, &mut msgs)?;
        } else {
            self.header_v1(&mut c, &mut msgs)?;
        }
        Ok(Object { addr, msgs })
    }

    fn header_v1(&self, c: &mut Cur<'_>, msgs: &mut Vec<Msg>) -> Result<(), MeshError> {
        let start = c.pos();
        let version = c.u8()?;
        if version != 1 {
            return Err(broken(format!(
                "object header version {version} at {start}"
            )));
        }
        c.skip(1)?;
        let total = c.u16()? as usize;
        c.skip(4)?; // reference count
        let size = c.u32()? as usize;
        c.skip(4)?; // alignment to 8 bytes
        let mut blocks = vec![(c.pos(), size)];
        let mut seen = HashSet::new();
        let mut count = 0usize;
        while let Some((p, size)) = blocks.pop() {
            if !seen.insert(p) || seen.len() > MAX_VISITS {
                return Err(broken("object header continuation loops"));
            }
            let end = p
                .checked_add(size)
                .filter(|&e| e <= self.len)
                .ok_or_else(|| broken("object header runs past the end"))?;
            let mut m = self.cur_in(p, end)?;
            while m.p + 8 <= m.d.len() && count < total {
                let ty = m.u16()?;
                let len = m.u16()? as usize;
                m.skip(4)?; // flags and reserved
                let at = m.pos();
                m.skip(len)?;
                count += 1;
                if ty == 0x10 {
                    let mut k = self.msg_cur(&(at..at + len));
                    let a = k
                        .addr()?
                        .ok_or_else(|| broken("continuation without address"))?;
                    let n = k.len()?;
                    let a = self.cur(a)?.pos();
                    blocks.push((
                        a,
                        usize::try_from(n).map_err(|_| broken("continuation size"))?,
                    ));
                } else {
                    msgs.push(Msg {
                        ty,
                        data: at..at + len,
                    });
                }
            }
        }
        Ok(())
    }

    fn header_v2(&self, c: &mut Cur<'_>, msgs: &mut Vec<Msg>) -> Result<(), MeshError> {
        c.sig(b"OHDR")?;
        let version = c.u8()?;
        if version != 2 {
            return Err(broken(format!("OHDR version {version}")));
        }
        let flags = c.u8()?;
        if flags & 0x20 != 0 {
            c.skip(16)?;
        }
        if flags & 0x10 != 0 {
            c.skip(4)?;
        }
        let size = c.uint(1 << (flags & 3))? as usize;
        let order = flags & 0x04 != 0;
        let mut blocks = vec![(c.pos(), size)];
        let mut seen = HashSet::new();
        while let Some((p, size)) = blocks.pop() {
            if !seen.insert(p) || seen.len() > MAX_VISITS {
                return Err(broken("object header continuation loops"));
            }
            let end = p
                .checked_add(size)
                .filter(|&e| e <= self.len)
                .ok_or_else(|| broken("object header runs past the end"))?;
            let mut m = self.cur_in(p, end)?;
            let head = if order { 6 } else { 4 };
            while m.p + head <= m.d.len() {
                let ty = m.u8()? as u16;
                let len = m.u16()? as usize;
                m.skip(1)?; // flags
                if order {
                    m.skip(2)?;
                }
                let at = m.pos();
                m.skip(len)?;
                if ty == 0x10 {
                    let mut k = self.msg_cur(&(at..at + len));
                    let a = k
                        .addr()?
                        .ok_or_else(|| broken("continuation without address"))?;
                    let n = usize::try_from(k.len()?).map_err(|_| broken("continuation size"))?;
                    let mut b = self.cur(a)?;
                    b.sig(b"OCHK")?;
                    // signature before, checksum after the messages
                    blocks.push((b.pos(), n.saturating_sub(8)));
                } else if ty != 0 {
                    msgs.push(Msg {
                        ty,
                        data: at..at + len,
                    });
                }
            }
        }
        Ok(())
    }

    fn first(&self, obj: &Object, ty: u16) -> Option<Range<usize>> {
        obj.msgs.iter().find(|m| m.ty == ty).map(|m| m.data.clone())
    }

    /// Whether the object is a group.
    pub fn is_group(&self, obj: &Object) -> bool {
        obj.msgs.iter().any(|m| matches!(m.ty, 0x02 | 0x06 | 0x11))
    }

    /// The links of a group, in creation order when the group tracks it,
    /// else in the order the file stores them.
    pub fn links(&self, obj: &Object) -> Result<Vec<Link>, MeshError> {
        let mut out = Vec::new();
        if let Some(r) = self.first(obj, 0x11) {
            let mut c = self.msg_cur(&r);
            let tree = c.addr()?;
            let heap = c
                .addr()?
                .ok_or_else(|| broken("symbol table without heap"))?;
            if let Some(tree) = tree {
                self.symbol_table(tree, heap, &mut out)?;
            }
        }
        for m in obj.msgs.iter().filter(|m| m.ty == 0x06) {
            out.push(self.link_msg(self.msg_bytes(&m.data))?);
        }
        if let Some(r) = self.first(obj, 0x02) {
            let mut c = self.msg_cur(&r);
            c.skip(1)?; // version
            let flags = c.u8()?;
            if flags & 1 != 0 {
                c.skip(8)?;
            }
            let heap = c.addr()?;
            let names = c.addr()?;
            if let (Some(heap), Some(names)) = (heap, names) {
                let heap = self.fractal_heap(heap)?;
                for rec in self.btree2_records(names)? {
                    let id = rec
                        .get(4..)
                        .ok_or_else(|| broken("link record too short"))?;
                    let bytes = self.heap_object(&heap, id)?;
                    out.push(self.link_msg(&bytes)?);
                }
            }
        }
        if out.iter().all(|l| l.order.is_some()) {
            out.sort_by_key(|l| l.order);
        }
        Ok(out)
    }

    fn link_msg(&self, d: &[u8]) -> Result<Link, MeshError> {
        let mut c = self.local(d);
        let version = c.u8()?;
        if version != 1 {
            return Err(broken(format!("link message version {version}")));
        }
        let flags = c.u8()?;
        let kind = if flags & 0x08 != 0 { c.u8()? } else { 0 };
        let order = if flags & 0x04 != 0 {
            Some(c.uint(8)?)
        } else {
            None
        };
        if flags & 0x10 != 0 {
            c.skip(1)?;
        }
        let n = c.uint(1 << (flags & 3))? as usize;
        let name = String::from_utf8_lossy(c.bytes(n)?).into_owned();
        let target = match kind {
            0 => Target::Hard(
                c.addr()?
                    .ok_or_else(|| broken("hard link without address"))?,
            ),
            1 => {
                let n = c.u16()? as usize;
                Target::Soft(String::from_utf8_lossy(c.bytes(n)?).into_owned())
            }
            _ => Target::External,
        };
        Ok(Link {
            name,
            target,
            order,
        })
    }

    fn symbol_table(&self, tree: u64, heap: u64, out: &mut Vec<Link>) -> Result<(), MeshError> {
        let mut h = self.cur(heap)?;
        h.sig(b"HEAP")?;
        h.skip(4)?;
        let seg_size = h.len()?;
        h.len()?;
        let seg = h.addr()?.ok_or_else(|| broken("local heap without data"))?;
        let names = self.slice(seg, seg_size)?;
        let mut stack = vec![tree];
        let mut visits = 0usize;
        while let Some(node) = stack.pop() {
            visits += 1;
            if visits > MAX_VISITS {
                return Err(broken("group B-tree loops"));
            }
            let mut c = self.cur(node)?;
            c.sig(b"TREE")?;
            let ty = c.u8()?;
            if ty != 0 {
                return Err(broken("group B-tree of the wrong type"));
            }
            let level = c.u8()?;
            let used = c.u16()? as usize;
            c.addr()?;
            c.addr()?;
            let mut children = Vec::with_capacity(used.min(4096));
            for _ in 0..used {
                c.len()?; // key
                children.push(
                    c.addr()?
                        .ok_or_else(|| broken("B-tree child without address"))?,
                );
            }
            if level > 0 {
                stack.extend(children.into_iter().rev());
                continue;
            }
            for snod in children {
                let mut s = self.cur(snod)?;
                s.sig(b"SNOD")?;
                s.skip(2)?;
                let n = s.u16()? as usize;
                for _ in 0..n {
                    let off = usize::try_from(s.len_as_offset()?)
                        .map_err(|_| broken("symbol name offset"))?;
                    let addr = s.addr()?.ok_or_else(|| broken("symbol without header"))?;
                    s.skip(24)?;
                    let name = names
                        .get(off..)
                        .map(|b| b.split(|&x| x == 0).next().unwrap_or(&[]))
                        .ok_or_else(|| broken("symbol name outside the heap"))?;
                    out.push(Link {
                        name: String::from_utf8_lossy(name).into_owned(),
                        target: Target::Hard(addr),
                        order: None,
                    });
                }
            }
        }
        Ok(())
    }

    /// The child linked from group `obj` as `name`, following soft links
    /// inside the file.
    pub fn child(&self, obj: &Object, name: &str) -> Result<Option<Object>, MeshError> {
        let Some(link) = self.links(obj)?.into_iter().find(|l| l.name == name) else {
            return Ok(None);
        };
        match link.target {
            Target::Hard(a) => self.object(a).map(Some),
            Target::Soft(p) => self.path(&p).map(Some),
            Target::External => Err(unsupported(format!("{name} is an external link"))),
        }
    }

    /// The object at an absolute path such as `/VTKHDF/internal`.
    pub fn path(&self, path: &str) -> Result<Object, MeshError> {
        let mut obj = self.root()?;
        for (depth, part) in path.split('/').filter(|p| !p.is_empty()).enumerate() {
            if depth > 64 {
                return Err(broken("path too deep"));
            }
            obj = self
                .child(&obj, part)?
                .ok_or_else(|| MeshError::new("MESH018", format!("{path}: no {part}")))?;
        }
        Ok(obj)
    }

    // ----- attributes -------------------------------------------------

    /// The attributes of an object, by name.
    pub fn attrs(&self, obj: &Object) -> Result<Vec<(String, Value)>, MeshError> {
        let mut out = Vec::new();
        for m in obj.msgs.iter().filter(|m| m.ty == 0x0C) {
            if let Some(a) = self.attr_msg(self.msg_bytes(&m.data))? {
                out.push(a);
            }
        }
        if let Some(r) = self.first(obj, 0x15) {
            let mut c = self.msg_cur(&r);
            c.skip(1)?;
            let flags = c.u8()?;
            if flags & 1 != 0 {
                c.skip(2)?;
            }
            let heap = c.addr()?;
            let names = c.addr()?;
            if let (Some(heap), Some(names)) = (heap, names) {
                let heap = self.fractal_heap(heap)?;
                for rec in self.btree2_records(names)? {
                    let id = rec
                        .get(..8)
                        .ok_or_else(|| broken("attribute record too short"))?;
                    let bytes = self.heap_object(&heap, id)?;
                    if let Some(a) = self.attr_msg(&bytes)? {
                        out.push(a);
                    }
                }
            }
        }
        Ok(out)
    }

    /// One attribute's value, if the object has it.
    pub fn attr(&self, obj: &Object, name: &str) -> Result<Option<Value>, MeshError> {
        Ok(self
            .attrs(obj)?
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v))
    }

    fn attr_msg(&self, d: &[u8]) -> Result<Option<(String, Value)>, MeshError> {
        let mut c = self.local(d);
        let version = c.u8()?;
        let flags = c.u8()?;
        let name_size = c.u16()? as usize;
        let dt_size = c.u16()? as usize;
        let ds_size = c.u16()? as usize;
        let pad = |n: usize| if version == 1 { n.div_ceil(8) * 8 } else { n };
        if version == 3 {
            c.skip(1)?; // name encoding
        } else if version != 1 && version != 2 {
            return Err(broken(format!("attribute message version {version}")));
        }
        if version >= 2 && flags & 0x03 != 0 {
            // a shared datatype or dataspace: not used by VTKHDF
            return Ok(None);
        }
        let name = c.bytes(pad(name_size))?;
        let name = name.get(..name_size).unwrap_or(name);
        let name =
            String::from_utf8_lossy(name.split(|&x| x == 0).next().unwrap_or(&[])).into_owned();
        let dt_at = c.p;
        c.skip(pad(dt_size))?;
        let ds_at = c.p;
        c.skip(pad(ds_size))?;
        let dtype = parse_dtype(&mut Cur {
            p: dt_at,
            ..c.clone()
        })?;
        let (dims, null) = parse_space(&mut Cur {
            p: ds_at,
            ..c.clone()
        })?;
        let n: u64 = if null { 0 } else { dims.iter().product() };
        let rest = &d[c.p.min(d.len())..];
        let value = self.convert_value(&dtype, n, rest)?;
        Ok(Some((name, value)))
    }

    fn convert_value(&self, dtype: &Dtype, n: u64, raw: &[u8]) -> Result<Value, MeshError> {
        let n = usize::try_from(n).map_err(|_| broken("attribute too large"))?;
        Ok(match dtype {
            Dtype::Int { .. } | Dtype::Float { .. } if n * dtype.size() > raw.len() => {
                return Err(broken("attribute data too short"));
            }
            Dtype::Int { .. } => Value::Int(ints(dtype, &raw[..n * dtype.size()])),
            Dtype::Float { .. } => Value::Float(floats(dtype, &raw[..n * dtype.size()])),
            Dtype::FixedString { size } => {
                let need = n * size;
                let b = raw
                    .get(..need)
                    .ok_or_else(|| broken("attribute data too short"))?;
                Value::Str(
                    b.chunks(*size.max(&1))
                        .map(|s| {
                            String::from_utf8_lossy(s)
                                .trim_end_matches(['\0', ' '])
                                .to_string()
                        })
                        .collect(),
                )
            }
            Dtype::VarString => {
                let mut c = self.local(raw);
                let mut v = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    let len = c.u32()? as u64;
                    let coll = c.addr()?;
                    let idx = c.u32()?;
                    v.push(match coll {
                        Some(a) if len > 0 => {
                            let b = self.global_object(a, idx)?;
                            let b = b.get(..len as usize).unwrap_or(b);
                            String::from_utf8_lossy(b)
                                .trim_end_matches('\0')
                                .to_string()
                        }
                        _ => String::new(),
                    });
                }
                Value::Str(v)
            }
            Dtype::Other(_) => Value::Other,
        })
    }

    /// Object `idx` of the global heap collection at `addr`.
    fn global_object(&self, addr: u64, idx: u32) -> Result<&[u8], MeshError> {
        let mut c = self.cur(addr)?;
        c.sig(b"GCOL")?;
        c.skip(4)?;
        let size = usize::try_from(c.len()?).map_err(|_| broken("global heap size"))?;
        let start = c.p - 8 - self.l;
        let end = start
            .checked_add(size)
            .filter(|&e| c.at + e <= self.len)
            .ok_or_else(|| broken("global heap runs past the end"))?;
        if !c.has(end - c.p) {
            return Err(broken("global heap runs past what was read"));
        }
        let mut c = Cur {
            d: &c.d[..end],
            holes: None,
            ..c
        };
        while c.p + 8 + self.l <= end {
            let i = c.u16()?;
            c.skip(6)?;
            let n = usize::try_from(c.len()?).map_err(|_| broken("global heap object"))?;
            if i == 0 {
                break;
            }
            let b = c.bytes(n)?;
            if i as u32 == idx {
                return Ok(b);
            }
            c.skip((8 - n % 8) % 8).ok();
        }
        Err(broken(format!("global heap object {idx} not found")))
    }

    // ----- datasets ---------------------------------------------------

    /// The dataset description of an object.
    pub fn dataset(&self, obj: &Object) -> Result<Dataset, MeshError> {
        let dt = self.first(obj, 0x03).ok_or_else(|| {
            MeshError::new("MESH018", "the object is not a dataset (no datatype)")
        })?;
        let dtype = parse_dtype(&mut self.msg_cur(&dt))?;
        let ds = self
            .first(obj, 0x01)
            .ok_or_else(|| broken("dataset without dataspace"))?;
        let (dims, null) = parse_space(&mut self.msg_cur(&ds))?;
        let lay = self
            .first(obj, 0x08)
            .ok_or_else(|| broken("dataset without layout"))?;
        let layout = self.parse_layout(&lay, &dims, dtype.size())?;
        let mut filters = Vec::new();
        if let Some(r) = self.first(obj, 0x0B) {
            filters = parse_filters(&mut self.msg_cur(&r))?;
        }
        if obj.msgs.iter().any(|m| m.ty == 0x07) {
            return Err(unsupported("datasets in external files"));
        }
        Ok(Dataset {
            dtype,
            dims,
            null,
            layout,
            filters,
        })
    }

    fn parse_layout(
        &self,
        r: &Range<usize>,
        dims: &[u64],
        elem: usize,
    ) -> Result<Layout, MeshError> {
        let mut c = self.msg_cur(r);
        let version = c.u8()?;
        match version {
            1 | 2 => {
                let nd = c.u8()? as usize;
                let class = c.u8()?;
                c.skip(5)?;
                match class {
                    0 => {
                        c.skip(4 * nd)?;
                        let size = c.u32()? as usize;
                        let at = c.pos();
                        c.skip(size)?;
                        Ok(Layout::Compact(at..at + size))
                    }
                    1 => {
                        let addr = c.addr()?;
                        let n: u64 = dims.iter().product::<u64>() * elem as u64;
                        Ok(Layout::Contiguous { addr, size: n })
                    }
                    2 => {
                        let addr = c.addr()?;
                        let mut cd = Vec::with_capacity(nd);
                        for _ in 0..nd {
                            cd.push(c.u32()? as u64);
                        }
                        cd.pop();
                        Ok(Layout::Chunked {
                            dims: cd,
                            index: ChunkIndex::BTree1(addr),
                        })
                    }
                    k => Err(unsupported(format!("data layout class {k}"))),
                }
            }
            // Version 5 (HDF5 2.0) encodes these classes as version 4 does.
            3..=5 => {
                let class = c.u8()?;
                match class {
                    0 => {
                        let size = c.u16()? as usize;
                        let at = c.pos();
                        c.skip(size)?;
                        Ok(Layout::Compact(at..at + size))
                    }
                    1 => {
                        let addr = c.addr()?;
                        let size = c.len()?;
                        Ok(Layout::Contiguous { addr, size })
                    }
                    2 if version == 3 => {
                        let nd = c.u8()? as usize;
                        let addr = c.addr()?;
                        let mut cd = Vec::with_capacity(nd);
                        for _ in 0..nd {
                            cd.push(c.u32()? as u64);
                        }
                        cd.pop();
                        Ok(Layout::Chunked {
                            dims: cd,
                            index: ChunkIndex::BTree1(addr),
                        })
                    }
                    2 => {
                        let flags = c.u8()?;
                        let nd = c.u8()? as usize;
                        let enc = c.u8()? as usize;
                        if !(1..=8).contains(&enc) {
                            return Err(broken("chunk dimension size encoding"));
                        }
                        let mut cd = Vec::with_capacity(nd);
                        for _ in 0..nd {
                            cd.push(c.uint(enc)?);
                        }
                        cd.pop();
                        let kind = c.u8()?;
                        let index = match kind {
                            1 => {
                                let (fs, mask) = if flags & 0x02 != 0 {
                                    (Some(c.len()?), c.u32()?)
                                } else {
                                    (None, 0)
                                };
                                ChunkIndex::Single {
                                    addr: c.addr()?,
                                    filtered_size: fs,
                                    mask,
                                }
                            }
                            2 => ChunkIndex::Implicit(c.addr()?),
                            3 => {
                                c.skip(1)?;
                                ChunkIndex::FixedArray(c.addr()?)
                            }
                            4 => return Err(unsupported("extensible-array chunk index")),
                            5 => return Err(unsupported("version 2 B-tree chunk index")),
                            k => return Err(broken(format!("chunk index type {k}"))),
                        };
                        Ok(Layout::Chunked { dims: cd, index })
                    }
                    3 => Err(unsupported("virtual datasets")),
                    k => Err(broken(format!("data layout class {k}"))),
                }
            }
            v => Err(unsupported(format!("data layout version {v}"))),
        }
    }

    /// The raw bytes of a dataset, `count × element size` long.
    pub fn raw(&self, ds: &Dataset) -> Result<Vec<u8>, MeshError> {
        self.raw_in_place(ds).map(Cow::into_owned)
    }

    /// The values of a numeric dataset where they lie: in the file's
    /// bytes when it is stored whole (compact or contiguous), decoded
    /// once when it is chunked. Integers and floats convert as
    /// [`H5::read_i64`] and [`H5::read_f64`] convert them, without the
    /// copies.
    pub fn values(&self, ds: &Dataset) -> Result<Values<'_>, MeshError> {
        self.values_or(ds, "a non-numeric dataset where numbers are expected")
    }

    /// [`H5::values`] of a dataset read for its integers.
    pub fn int_values(&self, ds: &Dataset) -> Result<Values<'_>, MeshError> {
        self.values_or(ds, "a non-numeric dataset where integers are expected")
    }

    /// Whether [`H5::int_values_in`] reads rows of `ds` apart from the
    /// rest (its data lies whole in the file).
    pub fn reads_rows(&self, ds: &Dataset) -> bool {
        matches!(ds.layout, Layout::Contiguous { addr: Some(_), .. })
    }

    /// Values `rows` of a numeric dataset read for its integers (element
    /// numbers, clamped to the dataset): read apart from the rest when
    /// the dataset lies whole in the file (contiguous), else all of them
    /// ([`H5::int_values`]). The values keep the dataset's numbering and
    /// length.
    pub fn int_values_in(&self, ds: &Dataset, rows: Range<usize>) -> Result<Values<'_>, MeshError> {
        self.values_in_or(
            ds,
            rows,
            "a non-numeric dataset where integers are expected",
        )
    }

    /// [`H5::int_values_in`] of a dataset read for its numbers.
    pub fn values_in(&self, ds: &Dataset, rows: Range<usize>) -> Result<Values<'_>, MeshError> {
        self.values_in_or(ds, rows, "a non-numeric dataset where numbers are expected")
    }

    fn values_in_or(
        &self,
        ds: &Dataset,
        rows: Range<usize>,
        not_numbers: &str,
    ) -> Result<Values<'_>, MeshError> {
        let Layout::Contiguous {
            addr: Some(a),
            size,
        } = ds.layout
        else {
            return self.values_or(ds, not_numbers);
        };
        let elem = ds.dtype.size();
        if Values::new(Cow::Borrowed(&[]), &ds.dtype).is_none() {
            return Err(unsupported(not_numbers));
        }
        if elem == 0 {
            return Err(unsupported("reading a dataset of this datatype"));
        }
        let total = ds
            .count()
            .checked_mul(elem as u64)
            .ok_or_else(|| broken("dataset size overflows"))?;
        if total > self.len as u64 {
            return Err(MeshError::new(
                "MESH004",
                format!(
                    "a dataset of {total} bytes cannot come from a file of {} bytes",
                    self.len
                ),
            ));
        }
        if size < total {
            return Err(broken("contiguous data shorter than its dataspace"));
        }
        let count = (total / elem as u64) as usize;
        let start = rows.start.min(count);
        let end = rows.end.clamp(start, count);
        let bytes = self.data(
            a.checked_add((start * elem) as u64)
                .ok_or_else(|| broken("data runs past the end of the file"))?,
            ((end - start) * elem) as u64,
        )?;
        Values::window(bytes, &ds.dtype, start, count).ok_or_else(|| unsupported(not_numbers))
    }

    fn values_or(&self, ds: &Dataset, not_numbers: &str) -> Result<Values<'_>, MeshError> {
        let bytes = self.raw_in_place(ds)?;
        Values::new(bytes, &ds.dtype).ok_or_else(|| unsupported(not_numbers))
    }

    fn raw_in_place(&self, ds: &Dataset) -> Result<Cow<'_, [u8]>, MeshError> {
        let elem = ds.dtype.size();
        if elem == 0 {
            return Err(unsupported("reading a dataset of this datatype"));
        }
        let total = ds
            .count()
            .checked_mul(elem as u64)
            .ok_or_else(|| broken("dataset size overflows"))?;
        // Compressed chunks may expand: allow deflate's largest ratio.
        let limit = (self.len as u64).saturating_mul(1100).max(1 << 20);
        if total > limit {
            return Err(MeshError::new(
                "MESH004",
                format!(
                    "a dataset of {total} bytes cannot come from a file of {} bytes",
                    self.len
                ),
            ));
        }
        let total = total as usize;
        match &ds.layout {
            Layout::Compact(r) => {
                let b = self
                    .part(r.start, r.end)
                    .map(|p| &p.bytes[r.start - p.at..r.end - p.at])
                    .ok_or_else(|| broken("compact data outside the file"))?;
                if b.len() < total {
                    return Err(broken("compact data too short"));
                }
                Ok(Cow::Borrowed(&b[..total]))
            }
            Layout::Contiguous { addr, size } => match addr {
                None => Ok(Cow::Owned(vec![0; total])),
                Some(a) => {
                    if *size < total as u64 {
                        return Err(broken("contiguous data shorter than its dataspace"));
                    }
                    self.data(*a, total as u64)
                }
            },
            Layout::Chunked { dims, index } => {
                self.chunked(ds, dims, index, elem, total).map(Cow::Owned)
            }
        }
    }

    /// The chunks of a chunked dataset: where each lies, its size in the
    /// file, its offsets in elements and its filter mask. Reads only the
    /// chunk index, not the chunks.
    fn chunk_list(
        &self,
        ds: &Dataset,
        cdims: &[u64],
        index: &ChunkIndex,
    ) -> Result<Vec<Chunk>, MeshError> {
        let rank = ds.dims.len();
        if cdims.len() != rank || cdims.contains(&0) {
            return Err(broken("chunk rank differs from the dataspace"));
        }
        let chunk_bytes = self.chunk_bytes(ds, cdims)?;
        let mut chunks: Vec<Chunk> = Vec::new();
        let grid: Vec<u64> = ds
            .dims
            .iter()
            .zip(cdims)
            .map(|(&d, &c)| d.div_ceil(c))
            .collect();
        let offsets_of = |mut i: u64| -> Vec<u64> {
            let mut o = vec![0u64; rank];
            for k in (0..rank).rev() {
                o[k] = (i % grid[k].max(1)) * cdims[k];
                i /= grid[k].max(1);
            }
            o
        };
        match index {
            ChunkIndex::BTree1(None)
            | ChunkIndex::Single { addr: None, .. }
            | ChunkIndex::Implicit(None)
            | ChunkIndex::FixedArray(None) => {}
            ChunkIndex::BTree1(Some(root)) => {
                let mut stack = vec![*root];
                let mut visits = 0usize;
                while let Some(node) = stack.pop() {
                    visits += 1;
                    if visits > MAX_VISITS {
                        return Err(broken("chunk B-tree loops"));
                    }
                    let mut c = self.cur(node)?;
                    c.sig(b"TREE")?;
                    if c.u8()? != 1 {
                        return Err(broken("chunk B-tree of the wrong type"));
                    }
                    let level = c.u8()?;
                    let used = c.u16()? as usize;
                    c.addr()?;
                    c.addr()?;
                    for _ in 0..used {
                        let size = c.u32()? as u64;
                        let mask = c.u32()?;
                        let mut off = Vec::with_capacity(rank + 1);
                        for _ in 0..=rank {
                            off.push(c.uint(8)?);
                        }
                        off.pop();
                        let child = c.addr()?.ok_or_else(|| broken("chunk without address"))?;
                        if level > 0 {
                            stack.push(child);
                        } else {
                            chunks.push(Chunk {
                                off,
                                addr: child,
                                size,
                                mask,
                            });
                        }
                    }
                }
            }
            ChunkIndex::Single {
                addr: Some(a),
                filtered_size,
                mask,
            } => {
                chunks.push(Chunk {
                    off: vec![0; rank],
                    addr: *a,
                    size: filtered_size.unwrap_or(chunk_bytes as u64),
                    mask: *mask,
                });
            }
            ChunkIndex::Implicit(Some(a)) => {
                let n: u64 = grid.iter().product();
                if n.saturating_mul(chunk_bytes as u64) > self.len as u64 {
                    return Err(broken("implicit chunks run past the end"));
                }
                for i in 0..n {
                    chunks.push(Chunk {
                        off: offsets_of(i),
                        addr: a + i * chunk_bytes as u64,
                        size: chunk_bytes as u64,
                        mask: 0,
                    });
                }
            }
            ChunkIndex::FixedArray(Some(a)) => {
                let mut c = self.cur(*a)?;
                c.sig(b"FAHD")?;
                c.skip(1)?;
                let client = c.u8()?;
                let entry = c.u8()? as usize;
                let page_bits = c.u8()?;
                let max = c.len()?;
                let block = c.addr()?;
                if page_bits < 64 && max > (1u64 << page_bits) {
                    return Err(unsupported("paged fixed-array chunk index"));
                }
                let Some(block) = block else {
                    return Ok(chunks);
                };
                let mut b = self.cur(block)?;
                b.sig(b"FADB")?;
                b.skip(2)?;
                b.addr()?;
                for i in 0..max {
                    let at = b.p;
                    let addr = b.addr()?;
                    let (size, mask) = if client == 1 {
                        let n = entry
                            .checked_sub(self.o + 4)
                            .ok_or_else(|| broken("fixed-array entry size"))?;
                        (b.uint(n)?, b.u32()?)
                    } else {
                        (chunk_bytes as u64, 0)
                    };
                    b.p = at + entry;
                    if let Some(addr) = addr {
                        chunks.push(Chunk {
                            off: offsets_of(i),
                            addr,
                            size,
                            mask,
                        });
                    }
                }
            }
        }
        Ok(chunks)
    }

    /// The bytes of one whole chunk of `ds`.
    fn chunk_bytes(&self, ds: &Dataset, cdims: &[u64]) -> Result<usize, MeshError> {
        let chunk_elems: u64 = cdims.iter().product();
        usize::try_from(chunk_elems.saturating_mul(ds.dtype.size() as u64))
            .ok()
            .filter(|&b| b as u64 <= (self.len as u64).saturating_mul(1100).max(1 << 20))
            .ok_or_else(|| broken("chunk too large"))
    }

    fn chunked(
        &self,
        ds: &Dataset,
        cdims: &[u64],
        index: &ChunkIndex,
        elem: usize,
        total: usize,
    ) -> Result<Vec<u8>, MeshError> {
        let chunks = self.chunk_list(ds, cdims, index)?;
        let chunk_bytes = self.chunk_bytes(ds, cdims)?;
        let mut out = vec![0u8; total];
        for chunk in chunks {
            let raw = self.data(chunk.addr, chunk.size)?.into_owned();
            let bytes = self.unfilter(&ds.filters, raw, chunk.mask, chunk_bytes)?;
            if bytes.len() < chunk_bytes {
                return Err(broken("chunk shorter than its dimensions"));
            }
            place(&mut out, &bytes, &ds.dims, cdims, &chunk.off, elem);
        }
        Ok(out)
    }

    fn unfilter(
        &self,
        filters: &[Filter],
        raw: Vec<u8>,
        mask: u32,
        expect: usize,
    ) -> Result<Vec<u8>, MeshError> {
        let mut b = raw;
        for (i, f) in filters.iter().enumerate().rev() {
            if i < 32 && mask & (1 << i) != 0 {
                continue;
            }
            b = match f.id {
                1 => {
                    let mut out = Vec::with_capacity(expect);
                    flate2::read::ZlibDecoder::new(b.as_slice())
                        .take(expect as u64 + 1)
                        .read_to_end(&mut out)
                        .map_err(|e| broken(format!("deflate: {e}")))?;
                    out
                }
                2 => {
                    let es = f.data.first().copied().unwrap_or(1).max(1) as usize;
                    unshuffle(&b, es)
                }
                3 => {
                    let n = b.len().saturating_sub(4);
                    b.truncate(n);
                    b
                }
                id => return Err(unsupported(format!("HDF5 filter {id}"))),
            };
        }
        Ok(b)
    }

    /// The values of a numeric dataset as `f64`.
    pub fn read_f64(&self, ds: &Dataset) -> Result<Vec<f64>, MeshError> {
        let raw = self.raw(ds)?;
        match ds.dtype {
            Dtype::Float { .. } => Ok(floats(&ds.dtype, &raw)),
            Dtype::Int { .. } => Ok(ints(&ds.dtype, &raw)
                .into_iter()
                .map(|v| v as f64)
                .collect()),
            _ => Err(unsupported(
                "a non-numeric dataset where numbers are expected",
            )),
        }
    }

    /// The values of an integer dataset (floats are truncated).
    pub fn read_i64(&self, ds: &Dataset) -> Result<Vec<i64>, MeshError> {
        let raw = self.raw(ds)?;
        match ds.dtype {
            Dtype::Int { .. } => Ok(ints(&ds.dtype, &raw)),
            Dtype::Float { .. } => Ok(floats(&ds.dtype, &raw)
                .into_iter()
                .map(|v| v as i64)
                .collect()),
            _ => Err(unsupported(
                "a non-numeric dataset where integers are expected",
            )),
        }
    }

    /// The values of a string dataset, fixed-length or variable-length.
    pub fn read_strings(&self, ds: &Dataset) -> Result<Vec<String>, MeshError> {
        match ds.dtype {
            Dtype::FixedString { .. } | Dtype::VarString => {}
            _ => return Err(unsupported("a non-string dataset where text is expected")),
        }
        let raw = self.raw(ds)?;
        match self.convert_value(&ds.dtype, ds.count(), &raw)? {
            Value::Str(v) => Ok(v),
            _ => Err(broken("string dataset")),
        }
    }

    // ----- fractal heap and version 2 B-tree --------------------------

    fn fractal_heap(&self, addr: u64) -> Result<Heap, MeshError> {
        let mut c = self.cur(addr)?;
        c.sig(b"FRHP")?;
        c.skip(1)?;
        let id_len = c.u16()? as usize;
        let filter_len = c.u16()?;
        let flags = c.u8()?;
        c.skip(4)?; // max managed object size
        c.len()?;
        c.addr()?;
        c.len()?;
        c.addr()?;
        for _ in 0..8 {
            c.len()?;
        }
        let width = c.u16()? as u64;
        let start = c.len()?;
        let max_direct = c.len()?;
        let max_heap_bits = c.u16()? as u32;
        c.skip(2)?; // starting rows
        let root = c.addr()?;
        let rows = c.u16()? as u32;
        if filter_len > 0 {
            return Err(unsupported("filtered fractal heap"));
        }
        let (Some(sb), Some(md)) = (log2_exact(start), log2_exact(max_direct)) else {
            return Err(broken("fractal heap block sizes are not powers of two"));
        };
        if width == 0
            || !width.is_power_of_two()
            || md < sb
            || max_heap_bits == 0
            || max_heap_bits > 64
        {
            return Err(broken("fractal heap table"));
        }
        Ok(Heap {
            id_len,
            width,
            start,
            start_bits: sb,
            max_direct_rows: md - sb + 2,
            off_bytes: (max_heap_bits as usize).div_ceil(8),
            checksummed: flags & 0x02 != 0,
            root,
            rows,
        })
    }

    /// The bytes of the object a heap ID names.
    fn heap_object(&self, h: &Heap, id: &[u8]) -> Result<Vec<u8>, MeshError> {
        let first = *id.first().ok_or_else(|| broken("empty heap ID"))?;
        match (first >> 4) & 3 {
            0 => {
                let mut c = Cur {
                    p: 1,
                    ..self.local(id)
                };
                let off = c.uint(h.off_bytes)?;
                let rest = id
                    .len()
                    .min(h.id_len)
                    .saturating_sub(1 + h.off_bytes)
                    .min(8);
                let len = c.uint(rest)?;
                self.managed(h, off, len)
            }
            2 => {
                if h.id_len <= 18 {
                    let n = (first & 0x0F) as usize + 1;
                    id.get(1..1 + n)
                        .map(<[u8]>::to_vec)
                        .ok_or_else(|| broken("tiny heap object"))
                } else {
                    let n =
                        (((first & 0x0F) as usize) << 8 | *id.get(1).unwrap_or(&0) as usize) + 1;
                    id.get(2..2 + n)
                        .map(<[u8]>::to_vec)
                        .ok_or_else(|| broken("tiny heap object"))
                }
            }
            1 => Err(unsupported("huge fractal heap objects")),
            t => Err(broken(format!("heap ID type {t}"))),
        }
    }

    fn row_size(h: &Heap, r: u32) -> u64 {
        if r < 2 {
            h.start
        } else {
            h.start.checked_shl(r - 1).unwrap_or(u64::MAX)
        }
    }

    fn managed(&self, h: &Heap, off: u64, len: u64) -> Result<Vec<u8>, MeshError> {
        let root = h.root.ok_or_else(|| broken("empty fractal heap"))?;
        if h.rows == 0 {
            return Ok(self
                .slice(
                    root.checked_add(off).ok_or_else(|| broken("heap offset"))?,
                    len,
                )?
                .to_vec());
        }
        let (mut block, mut rows, mut base) = (root, h.rows, 0u64);
        for _ in 0..64 {
            let mut c = self.cur(block)?;
            c.sig(b"FHIB")?;
            c.skip(1)?;
            c.addr()?;
            c.skip(h.off_bytes)?;
            let mut at = base;
            let mut next = None;
            'rows: for r in 0..rows {
                let size = Self::row_size(h, r);
                for _ in 0..h.width {
                    let child = c.addr()?;
                    if off < at.saturating_add(size) && off >= at {
                        let child =
                            child.ok_or_else(|| broken("heap object in a missing block"))?;
                        if r < h.max_direct_rows {
                            let a = child + (off - at);
                            return Ok(self.slice(a, len)?.to_vec());
                        }
                        let bits = 63 - size.leading_zeros();
                        let sub = bits
                            .checked_sub(h.start_bits + h.width.trailing_zeros())
                            .ok_or_else(|| broken("indirect heap block size"))?
                            + 1;
                        next = Some((child, sub, at));
                        break 'rows;
                    }
                    at = at.saturating_add(size);
                }
            }
            let (b, r, s) = next.ok_or_else(|| broken(format!("heap offset {off} not found")))?;
            block = b;
            rows = r;
            base = s;
        }
        let _ = h.checksummed;
        Err(broken("fractal heap too deep"))
    }

    /// Every record of a version 2 B-tree, in key order.
    fn btree2_records(&self, addr: u64) -> Result<Vec<Vec<u8>>, MeshError> {
        let mut c = self.cur(addr)?;
        c.sig(b"BTHD")?;
        c.skip(2)?;
        let node_size = c.u32()? as u64;
        let rec = c.u16()? as usize;
        let depth = c.u16()? as usize;
        c.skip(2)?;
        let root = c.addr()?;
        let root_n = c.u16()? as usize;
        let Some(root) = root else {
            return Ok(Vec::new());
        };
        if rec == 0 || node_size < 16 || depth > 32 {
            return Err(broken("version 2 B-tree header"));
        }
        // Maximum records per node and their encoded sizes, by depth.
        let mut max = vec![(node_size - 10) / rec as u64];
        let mut cum = vec![max[0]];
        let mut ptr = vec![0usize];
        for d in 1..=depth {
            let p = self.o + enc_size(max[d - 1]) + if d > 1 { enc_size(cum[d - 1]) } else { 0 };
            let m = node_size.saturating_sub(10 + p as u64) / (rec + p) as u64;
            ptr.push(p);
            max.push(m);
            cum.push((m + 1).saturating_mul(cum[d - 1]).saturating_add(m));
        }
        let mut out = Vec::new();
        let mut visits = 0usize;
        self.btree2_node(root, root_n, depth, rec, &max, &cum, &mut out, &mut visits)?;
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn btree2_node(
        &self,
        addr: u64,
        n: usize,
        depth: usize,
        rec: usize,
        max: &[u64],
        cum: &[u64],
        out: &mut Vec<Vec<u8>>,
        visits: &mut usize,
    ) -> Result<(), MeshError> {
        *visits += 1;
        if *visits > MAX_VISITS {
            return Err(broken("version 2 B-tree loops"));
        }
        let mut c = self.cur(addr)?;
        if depth == 0 {
            c.sig(b"BTLF")?;
            c.skip(2)?;
            for _ in 0..n {
                out.push(c.bytes(rec)?.to_vec());
            }
            return Ok(());
        }
        c.sig(b"BTIN")?;
        c.skip(2)?;
        let mut recs = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            recs.push(c.bytes(rec)?.to_vec());
        }
        let mut kids = Vec::with_capacity(n + 1);
        for _ in 0..=n {
            let a = c
                .addr()?
                .ok_or_else(|| broken("B-tree child without address"))?;
            let k = c.uint(enc_size(max[depth - 1]))? as usize;
            if depth > 1 {
                c.uint(enc_size(cum[depth - 1]))?;
            }
            kids.push((a, k));
        }
        for (i, (a, k)) in kids.into_iter().enumerate() {
            self.btree2_node(a, k, depth - 1, rec, max, cum, out, visits)?;
            if let Some(r) = recs.get(i) {
                out.push(r.clone());
            }
        }
        Ok(())
    }
}

impl Cur<'_> {
    /// A local-heap name offset: stored with the size of lengths.
    fn len_as_offset(&mut self) -> Result<u64, MeshError> {
        self.len()
    }
}

struct Heap {
    id_len: usize,
    width: u64,
    start: u64,
    start_bits: u32,
    max_direct_rows: u32,
    off_bytes: usize,
    checksummed: bool,
    root: Option<u64>,
    rows: u32,
}

fn check_sizes(o: usize, l: usize) -> Result<(), MeshError> {
    if ![2, 4, 8].contains(&o) || ![2, 4, 8].contains(&l) {
        return Err(broken(format!("offset size {o} or length size {l}")));
    }
    Ok(())
}

fn parse_dtype(c: &mut Cur<'_>) -> Result<Dtype, MeshError> {
    let cv = c.u8()?;
    let class = cv & 0x0F;
    let b0 = c.u8()?;
    c.skip(2)?;
    let size = c.u32()? as usize;
    Ok(match class {
        0 => Dtype::Int {
            size,
            signed: b0 & 0x08 != 0,
            be: b0 & 0x01 != 0,
        },
        1 => {
            if b0 & 0x40 != 0 || (size != 4 && size != 8) {
                return Err(unsupported(format!("floating-point type of {size} bytes")));
            }
            Dtype::Float {
                size,
                be: b0 & 0x01 != 0,
            }
        }
        3 => Dtype::FixedString { size },
        9 if b0 & 0x0F == 1 => Dtype::VarString,
        k => Dtype::Other(k),
    })
}

fn parse_space(c: &mut Cur<'_>) -> Result<(Vec<u64>, bool), MeshError> {
    let version = c.u8()?;
    let rank = c.u8()? as usize;
    let flags = c.u8()?;
    let mut null = false;
    match version {
        1 => c.skip(5)?,
        2 => {
            let ty = c.u8()?;
            null = ty == 2;
        }
        v => return Err(broken(format!("dataspace version {v}"))),
    }
    if rank > 32 {
        return Err(broken("dataspace rank"));
    }
    let mut dims = Vec::with_capacity(rank);
    for _ in 0..rank {
        dims.push(c.len()?);
    }
    let _ = flags;
    Ok((dims, null))
}

fn parse_filters(c: &mut Cur<'_>) -> Result<Vec<Filter>, MeshError> {
    let version = c.u8()?;
    let n = c.u8()? as usize;
    if version == 1 {
        c.skip(6)?;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = c.u16()?;
        let name_len = if version == 1 || id >= 256 {
            c.u16()? as usize
        } else {
            0
        };
        c.skip(2)?; // flags
        let nv = c.u16()? as usize;
        if version == 1 {
            c.skip(name_len.div_ceil(8) * 8)?;
        } else {
            c.skip(name_len)?;
        }
        let mut data = Vec::with_capacity(nv.min(64));
        for _ in 0..nv {
            data.push(c.u32()?);
        }
        if version == 1 && nv % 2 == 1 {
            c.skip(4)?;
        }
        out.push(Filter { id, data });
    }
    Ok(out)
}

fn floats(dt: &Dtype, raw: &[u8]) -> Vec<f64> {
    match *dt {
        Dtype::Float { size: 4, be } => raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&a| {
                (if be {
                    f32::from_be_bytes(a)
                } else {
                    f32::from_le_bytes(a)
                }) as f64
            })
            .collect(),
        Dtype::Float { size: 8, be } => raw
            .as_chunks::<8>()
            .0
            .iter()
            .map(|&a| {
                if be {
                    f64::from_be_bytes(a)
                } else {
                    f64::from_le_bytes(a)
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn ints(dt: &Dtype, raw: &[u8]) -> Vec<i64> {
    let Dtype::Int { size, signed, be } = *dt else {
        return Vec::new();
    };
    if size == 0 || size > 8 {
        return Vec::new();
    }
    raw.chunks_exact(size)
        .map(|b| {
            let mut v = 0u64;
            for i in 0..size {
                let byte = if be { b[size - 1 - i] } else { b[i] };
                v |= (byte as u64) << (8 * i);
            }
            if signed && size < 8 && v & (1 << (8 * size - 1)) != 0 {
                v |= u64::MAX << (8 * size);
            }
            v as i64
        })
        .collect()
}

fn unshuffle(b: &[u8], es: usize) -> Vec<u8> {
    let n = b.len() / es;
    let mut out = vec![0u8; b.len()];
    for byte in 0..es {
        for i in 0..n {
            out[i * es + byte] = b[byte * n + i];
        }
    }
    out[n * es..].copy_from_slice(&b[n * es..]);
    out
}

/// Copy a decoded chunk at element offsets `off` into the row-major
/// output of dimensions `dims`; parts past the edge are dropped.
fn place(out: &mut [u8], chunk: &[u8], dims: &[u64], cdims: &[u64], off: &[u64], elem: usize) {
    let rank = dims.len();
    if rank == 0 {
        let n = out.len().min(chunk.len());
        out[..n].copy_from_slice(&chunk[..n]);
        return;
    }
    if off.iter().zip(dims).any(|(o, d)| o >= d) {
        return;
    }
    let last = rank - 1;
    let run = (cdims[last].min(dims[last] - off[last])) as usize;
    let rows: u64 = cdims[..last].iter().product();
    let mut idx = vec![0u64; last];
    for _ in 0..rows {
        // global position of this row
        let mut inside = true;
        let mut g = 0u64;
        let mut l = 0u64;
        for k in 0..last {
            let x = off[k] + idx[k];
            if x >= dims[k] {
                inside = false;
            }
            g = g * dims[k] + x;
            l = l * cdims[k] + idx[k];
        }
        if inside {
            let g = ((g * dims[last] + off[last]) as usize) * elem;
            let l = (l * cdims[last]) as usize * elem;
            let n = run * elem;
            if g + n <= out.len() && l + n <= chunk.len() {
                out[g..g + n].copy_from_slice(&chunk[l..l + n]);
            }
        }
        for k in (0..last).rev() {
            idx[k] += 1;
            if idx[k] < cdims[k] {
                break;
            }
            idx[k] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> H5 {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf")
            .join(name);
        H5::open(&p).unwrap()
    }

    /// Invariant: the FoehnCraft layout (superblock 0, symbol-table root,
    /// groups with tracked link order and more than 8 links in dense
    /// storage) lists every block, dataset and root attribute. A reader
    /// that only knew symbol tables would miss the CellData datasets.
    #[test]
    fn reads_the_foehncraft_layout() {
        let h = fixture("channel_fields.vtkhdf");
        let root = h.root().unwrap();
        let attrs = h.attrs(&root).unwrap();
        let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        assert_eq!(
            get("foehncraft_format").and_then(|v| v.text()).as_deref(),
            Some("foehncraft-fields")
        );
        assert_eq!(get("iteration").and_then(|v| v.number()), Some(250.0));
        assert!(
            get("units")
                .and_then(|v| v.text())
                .unwrap()
                .starts_with("coordinates m;")
        );
        let vtk = h.path("/VTKHDF").unwrap();
        let names: Vec<String> = h.links(&vtk).unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["internal", "Assembly", "inlet", "outlet", "walls"]);
        assert_eq!(
            h.attr(&vtk, "Type")
                .unwrap()
                .and_then(|v| v.text())
                .as_deref(),
            Some("PartitionedDataSetCollection")
        );
        let cd = h.path("/VTKHDF/internal/CellData").unwrap();
        let names: Vec<String> = h.links(&cd).unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(
            names,
            [
                "velocity_x",
                "velocity_y",
                "velocity_z",
                "speed",
                "pressure",
                "k",
                "omega",
                "nu_t",
                "zone"
            ]
        );
        let p = h.path("/VTKHDF/internal/CellData/pressure").unwrap();
        let ds = h.dataset(&p).unwrap();
        assert_eq!(ds.dims, [776]);
        let v = h.read_f64(&ds).unwrap();
        assert!(v.iter().all(|x| (-50.0..=50.0).contains(x)));
        let t = h.path("/VTKHDF/internal/Types").unwrap();
        let types = h.read_i64(&h.dataset(&t).unwrap()).unwrap();
        assert_eq!(types.len(), 776);
        // soft links of the assembly resolve to the blocks
        let a = h.path("/VTKHDF/Assembly/boundary/walls").unwrap();
        assert_eq!(
            h.attr(&a, "Type")
                .unwrap()
                .and_then(|v| v.text())
                .as_deref(),
            Some("PolyData")
        );
    }

    /// Invariant: chunked, deflated and shuffled datasets (B-tree v1
    /// index, chunks of 2 over 4 values) read back to the values written:
    /// `s<i>` is the x of the cell centre plus i. A wrong unshuffle or a
    /// chunk placed at the wrong offset changes these.
    #[test]
    fn reads_chunked_deflated_shuffled() {
        let h = fixture("box_chunked.vtkhdf");
        let read = |n: &str| {
            let o = h.path(&format!("/VTKHDF/internal/CellData/{n}")).unwrap();
            h.read_f64(&h.dataset(&o).unwrap()).unwrap()
        };
        let x = read("s0");
        assert_eq!(x.len(), 4);
        for i in 1..10 {
            let s = read(&format!("s{i}"));
            for (a, b) in s.iter().zip(&x) {
                assert!((a - b - i as f64).abs() < 1e-12);
            }
        }
        let pts = h.path("/VTKHDF/internal/Points").unwrap();
        let ds = h.dataset(&pts).unwrap();
        assert_eq!(ds.dims, [21, 3]);
        let p = h.read_f64(&ds).unwrap();
        assert!(p.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    /// Invariant: the newest file format (superblock 3, OHDR headers,
    /// dense attributes, single-chunk, fixed-array and compact layouts, a
    /// big-endian dataset) reads to the same values as the oldest. Each
    /// layout has its own code path; a slip in any of them shows here.
    #[test]
    fn newest_format_matches_oldest() {
        let old = fixture("box_chunked.vtkhdf");
        let new = fixture("box_latest.vtkhdf");
        let read = |h: &H5, n: &str| {
            let o = h.path(&format!("/VTKHDF/internal/{n}")).unwrap();
            h.read_f64(&h.dataset(&o).unwrap()).unwrap()
        };
        for n in [
            "Points",
            "CellData/s0",
            "CellData/s1",
            "CellData/s2",
            "CellData/s3",
            "CellData/pressure",
            "Offsets",
            "FaceConnectivity",
        ] {
            assert_eq!(read(&old, n), read(&new, n), "{n}");
        }
        let attrs = new.attrs(&new.root().unwrap()).unwrap();
        assert_eq!(attrs.len(), 13);
        assert!(
            attrs
                .iter()
                .any(|(k, v)| k == "note_5" && v.text().as_deref() == Some("attribute 5"))
        );
        assert!(
            attrs
                .iter()
                .any(|(k, v)| k == "time" && v.number() == Some(0.25))
        );
    }

    /// Invariant: broken files give a diagnostic and never panic: every
    /// truncation of a real file and a scatter of flipped bytes either
    /// reads or fails with MESH0xx.
    #[test]
    fn broken_files_never_panic() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/box_latest.vtkhdf");
        let bytes = std::fs::read(p).unwrap();
        let walk = |b: Vec<u8>| {
            let Ok(h) = H5::from_bytes(b) else { return };
            let Ok(o) = h.path("/VTKHDF/internal/CellData") else {
                return;
            };
            if let Ok(links) = h.links(&o) {
                for l in links {
                    if let Ok(Some(c)) = h.child(&o, &l.name)
                        && let Ok(ds) = h.dataset(&c)
                    {
                        let _ = h.read_f64(&ds);
                    }
                }
            }
            let _ = h.attrs(&h.root().unwrap_or(o));
        };
        for cut in (0..bytes.len()).step_by(997) {
            walk(bytes[..cut].to_vec());
        }
        let mut seed = 0x2545F4914F6CDD1Du64;
        for _ in 0..300 {
            let mut b = bytes.clone();
            for _ in 0..8 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed % b.len() as u64) as usize;
                b[i] = (seed >> 32) as u8;
            }
            walk(b);
        }
        assert_eq!(
            H5::from_bytes(b"not hdf5".to_vec()).err().map(|e| e.code),
            Some("MESH018")
        );
    }

    /// Invariant: [`Values`] give, value by value, what [`H5::read_i64`]
    /// and [`H5::read_f64`] give for every integer size (1, 2, 4, 8 bytes,
    /// signed or not, either byte order), both float sizes in either byte
    /// order, and sizes the readers do not convert (no values); integers
    /// convert to floats and floats truncate to integers. Breaks when a
    /// byte order is swapped, a signed integer is not sign-extended, or
    /// `len` counts bytes. The fixture tests read only the sizes FoehnCraft
    /// and our writer use (8-byte little-endian), so no other test reads
    /// the rest.
    #[test]
    fn values_convert_as_the_copies_do() {
        let raw: Vec<u8> = (0..64u32).map(|i| (i * 37 + 200) as u8).collect();
        let mut dtypes = Vec::new();
        for size in [1, 2, 3, 4, 8, 16] {
            for signed in [false, true] {
                for be in [false, true] {
                    dtypes.push(Dtype::Int { size, signed, be });
                }
            }
        }
        for size in [2, 4, 8] {
            for be in [false, true] {
                dtypes.push(Dtype::Float { size, be });
            }
        }
        for dt in dtypes {
            let v = Values::new(Cow::Borrowed(&raw[..]), &dt).unwrap();
            let (i, f) = match dt {
                Dtype::Int { .. } => {
                    let i = ints(&dt, &raw);
                    let f: Vec<f64> = i.iter().map(|&x| x as f64).collect();
                    (i, f)
                }
                _ => {
                    let f = floats(&dt, &raw);
                    (f.iter().map(|&x| x as i64).collect(), f)
                }
            };
            assert_eq!(v.len(), i.len(), "{dt:?}");
            for k in 0..v.len() {
                assert_eq!(v.int(k), i[k], "{dt:?} value {k}");
                assert_eq!(v.float(k).to_bits(), f[k].to_bits(), "{dt:?} value {k}");
            }
        }
        assert!(Values::new(Cow::Borrowed(&raw[..]), &Dtype::VarString).is_none());
    }

    /// Every link, attribute, dataset description and value of a file,
    /// in a form to compare.
    fn everything(h: &H5) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![("/".to_string(), h.root().unwrap())];
        while let Some((path, obj)) = stack.pop() {
            out.push(format!("{path} attrs {:?}", h.attrs(&obj)));
            if h.is_group(&obj) {
                for l in h.links(&obj).unwrap() {
                    out.push(format!("{path} link {} {:?}", l.name, l.target));
                    if matches!(l.target, Target::Hard(_))
                        && let Ok(Some(c)) = h.child(&obj, &l.name)
                    {
                        stack.push((format!("{path}{}/", l.name), c));
                    }
                }
            } else {
                let ds = h.dataset(&obj);
                out.push(format!(
                    "{path} {:?}",
                    ds.as_ref().map(|d| (&d.dtype, &d.dims))
                ));
                if let Ok(ds) = ds {
                    let floats = h
                        .read_f64(&ds)
                        .map(|v| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
                    out.push(format!(
                        "{path} {floats:?} {:?} {:?}",
                        h.read_i64(&ds),
                        h.read_strings(&ds)
                    ));
                }
            }
        }
        out
    }

    /// The HDF5 files of the fixtures.
    fn fixture_files() -> Vec<std::path::PathBuf> {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/vtkhdf");
        let mut out = Vec::new();
        let mut dirs = vec![root];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| x == "vtkhdf") {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// Invariant: a file opened by its path, which reads its structure in
    /// pages and its data from the file, gives every link, attribute,
    /// dataset description and value that the same file read whole gives:
    /// for every fixture (both superblock families, dense and compact
    /// links and attributes, compact, contiguous and chunked layouts,
    /// variable-length strings), with pages of 64 KiB and with pages so
    /// small that every piece of the structure is found over many rounds
    /// and runs are joined. Breaks when a position is taken from the
    /// wrong run, a piece of the structure is not walked when opening
    /// (it is then missing when read), or runs are joined wrongly. The
    /// other tests read the files either way, never both.
    #[test]
    fn opening_a_path_reads_what_the_bytes_read() {
        let files = fixture_files();
        assert!(files.len() >= 5);
        for path in files {
            let whole = everything(&H5::from_bytes(std::fs::read(&path).unwrap()).unwrap());
            for page in [PAGE, 512] {
                let h = H5::open_paged(&path, page).unwrap();
                assert_eq!(
                    everything(&h),
                    whole,
                    "{} in pages of {page}",
                    path.display()
                );
            }
        }
    }

    /// Invariant: a file opened by its path holds only its structure, a
    /// few pages, not its data: a dataset of 8 MiB is read from the file
    /// when it is asked for, on several threads in pieces, and gives the
    /// values written. Breaks when the opening reads the data (the reader
    /// would hold the file again) or the pieces land at the wrong place.
    #[test]
    fn opening_a_path_leaves_the_data_in_the_file() {
        use crate::h5write::{H5Writer, Link};
        let dir = std::env::temp_dir().join(format!("murmel-hdf5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.h5");
        let values: Vec<f64> = (0..1 << 20).map(|i| i as f64 * 0.5 - 3.0).collect();
        let mut w = H5Writer::create(&path).unwrap();
        let big = w.dataset(&[values.len() as u64], &values).unwrap();
        let root = w.group(&[Link::Hard("big".into(), big)], &[]).unwrap();
        w.finish(root).unwrap();
        let h = H5::open(&path).unwrap();
        let held: usize = h.parts.iter().map(|p| p.bytes.len()).sum();
        assert!(held <= 3 * PAGE, "{held} bytes held of {}", h.len());
        let ds = h.dataset(&h.path("/big").unwrap()).unwrap();
        let raw = crate::par::with_threads(4, || h.raw_in_place(&ds)).unwrap();
        assert!(matches!(raw, Cow::Owned(_)));
        let got = crate::par::with_threads(4, || h.read_f64(&ds)).unwrap();
        assert_eq!(got, values);
        std::fs::remove_file(&path).ok();
    }

    /// Every dataset of a file.
    fn datasets(h: &H5) -> Vec<Dataset> {
        let mut out = Vec::new();
        let mut stack = vec![h.root().unwrap()];
        while let Some(obj) = stack.pop() {
            if h.is_group(&obj) {
                for l in h.links(&obj).unwrap() {
                    if matches!(l.target, Target::Hard(_))
                        && let Ok(Some(c)) = h.child(&obj, &l.name)
                    {
                        stack.push(c);
                    }
                }
            } else if let Ok(ds) = h.dataset(&obj) {
                out.push(ds);
            }
        }
        out
    }

    /// Invariant: rows of a dataset read apart from the rest
    /// ([`H5::values_in`], [`H5::int_values_in`]) are, value by value,
    /// those rows of the whole dataset, keep its numbering and length,
    /// and hold no row outside the dataset, for rows inside it, at and
    /// past its end, empty and reversed; a contiguous dataset holds only
    /// the rows asked for (opened by path, read from the file), any other
    /// layout all of them. Breaks when a window starts at the wrong byte,
    /// numbers its values from 0 instead of its first row, or is not
    /// clamped to the dataset. The cell walk's tests give it windows made
    /// in memory, so only this test reads them from a file.
    #[test]
    fn rows_read_apart_are_those_of_the_whole() {
        let mut seen = [false; 2];
        for path in fixture_files() {
            let h = H5::open(&path).unwrap();
            for ds in datasets(&h) {
                let Ok(whole) = h.values(&ds) else {
                    assert!(h.values_in(&ds, 0..1).is_err());
                    continue;
                };
                let n = whole.len();
                let apart = h.reads_rows(&ds);
                seen[usize::from(apart)] = true;
                let windows = [
                    0..0,
                    0..1,
                    1..3,
                    n / 2..n,
                    n.saturating_sub(1)..n + 5,
                    n + 2..n + 9,
                    Range { start: 3, end: 2 },
                    0..usize::MAX,
                ];
                for rows in windows {
                    for got in [
                        h.values_in(&ds, rows.clone()),
                        h.int_values_in(&ds, rows.clone()),
                    ] {
                        let got = got.unwrap();
                        assert_eq!(got.len(), n, "{} {rows:?}", path.display());
                        let held = got.held();
                        if apart {
                            let start = rows.start.min(n);
                            assert_eq!(held, start..rows.end.clamp(start, n));
                        } else {
                            assert_eq!(held, whole.held());
                        }
                        for i in held {
                            assert_eq!(got.int(i), whole.int(i));
                            assert_eq!(got.float(i).to_bits(), whole.float(i).to_bits());
                        }
                    }
                }
            }
        }
        assert_eq!(seen, [true, true], "fixtures with both layouts");

        use crate::h5write::{H5Writer, Link};
        let dir = std::env::temp_dir().join(format!("murmel-rows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rows.h5");
        let values: Vec<i64> = (0..1 << 20).map(|i| i * 3 - 7).collect();
        let mut w = H5Writer::create(&path).unwrap();
        let big = w.dataset(&[values.len() as u64], &values).unwrap();
        let root = w.group(&[Link::Hard("big".into(), big)], &[]).unwrap();
        w.finish(root).unwrap();
        let h = H5::open(&path).unwrap();
        let ds = h.dataset(&h.path("/big").unwrap()).unwrap();
        let got = h.int_values_in(&ds, 1000..1010).unwrap();
        assert_eq!(got.held(), 1000..1010);
        assert_eq!(got.len(), values.len());
        assert!((1000..1010).all(|i| got.int(i) == values[i]));
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    /// Invariant: broken files opened by their path give a diagnostic and
    /// never panic, as broken bytes do (`broken_files_never_panic`): a
    /// walk of the structure that meets a broken piece keeps what it
    /// found. Breaks when opening fails on a hole it should read, or
    /// trusts a size it read. Opening by path has its own code (pages,
    /// rounds, positional reads) that the bytes test does not reach.
    #[test]
    fn broken_files_opened_by_path_never_panic() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/box_latest.vtkhdf");
        let bytes = std::fs::read(p).unwrap();
        let dir = std::env::temp_dir().join(format!("murmel-hdf5-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.h5");
        let walk = |b: &[u8]| {
            std::fs::write(&path, b).unwrap();
            if let Ok(h) = H5::open_paged(&path, 512) {
                if let Ok(root) = h.root() {
                    let _ = h.attrs(&root);
                }
                if let Ok(o) = h.path("/VTKHDF/internal/CellData")
                    && let Ok(links) = h.links(&o)
                {
                    for l in links {
                        if let Ok(Some(c)) = h.child(&o, &l.name)
                            && let Ok(ds) = h.dataset(&c)
                        {
                            let _ = h.read_f64(&ds);
                        }
                    }
                }
            }
        };
        for cut in (0..bytes.len()).step_by(1997) {
            walk(&bytes[..cut]);
        }
        let mut seed = 0x9E3779B97F4A7C15u64;
        for _ in 0..100 {
            let mut b = bytes.clone();
            for _ in 0..8 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed % b.len() as u64) as usize;
                b[i] = (seed >> 32) as u8;
            }
            walk(&b);
        }
        std::fs::write(&path, b"not hdf5").unwrap();
        assert_eq!(H5::open(&path).err().map(|e| e.code), Some("MESH018"));
        std::fs::remove_file(&path).ok();
    }

    /// Invariant: the shuffle filter puts byte b of element i back from
    /// position b·n + i, and leaves a ragged tail alone.
    #[test]
    fn unshuffle_restores_elements() {
        let v: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 9];
        // two 3-byte elements shuffled: bytes 0 of both, 1 of both, 2 of both
        let shuffled = vec![1, 4, 2, 5, 3, 6, 9];
        assert_eq!(unshuffle(&shuffled, 3), v);
    }
}
