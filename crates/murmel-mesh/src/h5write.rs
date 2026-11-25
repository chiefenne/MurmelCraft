//! A writer for the part of HDF5 that blocked VTKHDF files need, written
//! from the published "HDF5 File Format Specification Version 3.0" (The
//! HDF Group), as the reader ([`crate::hdf5`]) is; no HDF5 library source
//! was read.
//!
//! Written: superblock version 2; version 2 object headers; groups with
//! their links in the header (hard and soft links); attributes in the
//! header; contiguous datasets of little-endian fixed-point and IEEE
//! floating-point numbers and of fixed-length strings. The checksums the
//! format asks for are Bob Jenkins' lookup3 hash (`hashlittle`, 2006,
//! public domain), as the specification names it.
//!
//! The raw data of a dataset is written first and its object header
//! after it, and a group after its members, so the file is written
//! front to back in one pass and a dataset of any size streams through:
//! nothing is held in memory but the bytes of one call. The superblock,
//! which points at the root group, is written last at the start.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// The undefined address.
const UNDEF: u64 = u64::MAX;
const SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";
const SUPERBLOCK: u64 = 48;

/// Bob Jenkins' lookup3 `hashlittle` of `data` with seed `init`.
pub fn lookup3(data: &[u8], init: u32) -> u32 {
    fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(4);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(6);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(8);
        *b = b.wrapping_add(*a);
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(16);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(19);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(4);
        *b = b.wrapping_add(*a);
    }
    fn fin(a: &mut u32, b: &mut u32, c: &mut u32) {
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(14));
        *a ^= *c;
        *a = a.wrapping_sub(c.rotate_left(11));
        *b ^= *a;
        *b = b.wrapping_sub(a.rotate_left(25));
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(16));
        *a ^= *c;
        *a = a.wrapping_sub(c.rotate_left(4));
        *b ^= *a;
        *b = b.wrapping_sub(a.rotate_left(14));
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(24));
    }
    let word = |k: &[u8]| -> u32 {
        k.iter()
            .enumerate()
            .fold(0u32, |w, (i, &x)| w | (u32::from(x) << (8 * i)))
    };
    let start = 0xdead_beef_u32
        .wrapping_add(data.len() as u32)
        .wrapping_add(init);
    let (mut a, mut b, mut c) = (start, start, start);
    let mut k = data;
    while k.len() > 12 {
        a = a.wrapping_add(word(&k[0..4]));
        b = b.wrapping_add(word(&k[4..8]));
        c = c.wrapping_add(word(&k[8..12]));
        mix(&mut a, &mut b, &mut c);
        k = &k[12..];
    }
    if k.is_empty() {
        return c;
    }
    a = a.wrapping_add(word(&k[..k.len().min(4)]));
    if k.len() > 4 {
        b = b.wrapping_add(word(&k[4..k.len().min(8)]));
    }
    if k.len() > 8 {
        c = c.wrapping_add(word(&k[8..]));
    }
    fin(&mut a, &mut b, &mut c);
    c
}

/// The type of a dataset's or an attribute's elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    U8,
    U32,
    I32,
    I64,
    F64,
    /// An ASCII string of this many bytes, null-padded.
    Str(usize),
    /// A UTF-8 string of this many bytes, null-padded: for text that is
    /// not ASCII. VTK 9.4's reader refuses a `Type` attribute marked
    /// UTF-8, so text that is ASCII is marked ASCII.
    Utf8(usize),
}

impl Ty {
    pub fn size(self) -> usize {
        match self {
            Ty::U8 => 1,
            Ty::U32 | Ty::I32 => 4,
            Ty::I64 | Ty::F64 => 8,
            Ty::Str(n) | Ty::Utf8(n) => n.max(1),
        }
    }

    /// The datatype message (version 1).
    fn message(self) -> Vec<u8> {
        let mut m = Vec::with_capacity(24);
        let fixed = |m: &mut Vec<u8>, signed: bool, bytes: u32| {
            m.push(0x10);
            m.extend([if signed { 0x08 } else { 0x00 }, 0, 0]);
            m.extend(bytes.to_le_bytes());
            m.extend(0u16.to_le_bytes());
            m.extend((8 * bytes as u16).to_le_bytes());
        };
        match self {
            Ty::U8 => fixed(&mut m, false, 1),
            Ty::U32 => fixed(&mut m, false, 4),
            Ty::I32 => fixed(&mut m, true, 4),
            Ty::I64 => fixed(&mut m, true, 8),
            Ty::F64 => {
                m.push(0x11);
                // little endian, mantissa normalised with an implied
                // leading bit, the sign at bit 63
                m.extend([0x20, 63, 0]);
                m.extend(8u32.to_le_bytes());
                m.extend(0u16.to_le_bytes());
                m.extend(64u16.to_le_bytes());
                m.extend([52, 11, 0, 52]);
                m.extend(1023u32.to_le_bytes());
            }
            Ty::Str(n) | Ty::Utf8(n) => {
                m.push(0x13);
                // null-padded; character set 0 ASCII, 1 UTF-8
                let set = if matches!(self, Ty::Utf8(_)) { 0x10 } else { 0 };
                m.extend([0x01 | set, 0, 0]);
                m.extend((n.max(1) as u32).to_le_bytes());
            }
        }
        m
    }
}

/// The dataspace message (version 2) of `dims`; scalar when `None`.
fn dataspace(dims: Option<&[u64]>) -> Vec<u8> {
    let mut m = vec![2];
    match dims {
        None => m.extend([0, 0, 0]),
        Some(d) => {
            m.extend([d.len() as u8, 0, 1]);
            for &n in d {
                m.extend(n.to_le_bytes());
            }
        }
    }
    m
}

/// An attribute's value.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    /// One 64-bit integer (a scalar).
    Int(i64),
    /// 64-bit integers (a one-dimensional array).
    Ints(Vec<i64>),
    /// A string (a scalar).
    Text(String),
    /// One 64-bit float (a scalar).
    Float(f64),
}

/// A link of a group.
#[derive(Clone, Debug)]
pub enum Link {
    /// To the object header at this address.
    Hard(String, u64),
    /// To a path in the file.
    Soft(String, String),
}

/// Plain numbers that go into a dataset as their little-endian bytes.
pub trait Element: Copy {
    const TY: Ty;
    fn put(self, out: &mut Vec<u8>);
}

impl Element for u8 {
    const TY: Ty = Ty::U8;
    fn put(self, out: &mut Vec<u8>) {
        out.push(self);
    }
}

impl Element for u32 {
    const TY: Ty = Ty::U32;
    fn put(self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
}

impl Element for i32 {
    const TY: Ty = Ty::I32;
    fn put(self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
}

impl Element for i64 {
    const TY: Ty = Ty::I64;
    fn put(self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
}

impl Element for f64 {
    const TY: Ty = Ty::F64;
    fn put(self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
}

/// An HDF5 file being written.
pub struct H5Writer {
    out: BufWriter<File>,
    pos: u64,
    buf: Vec<u8>,
    /// The file's position was moved by [`Rows`]: the next write goes
    /// back to `pos` first.
    moved: bool,
}

/// The raw data of a contiguous dataset set aside by
/// [`H5Writer::reserve`], to be filled through [`Rows`].
#[derive(Clone, Debug, PartialEq)]
pub struct Reserved {
    pub start: u64,
    pub ty: Ty,
    pub dims: Vec<u64>,
}

impl Reserved {
    /// The address of row `row` (a row is the product of the dims after
    /// the first).
    pub fn at(&self, row: u64) -> u64 {
        let width: u64 = self.dims.iter().skip(1).product();
        self.start + row * width * self.ty.size() as u64
    }
}

/// Writes into reserved space at any address, from any thread (one at a
/// time); while it lives the writer itself writes nothing.
pub struct Rows<'a> {
    file: std::sync::Mutex<&'a File>,
}

impl Rows<'_> {
    /// Write `values` from address `addr`.
    pub fn put<T: Element>(&self, addr: u64, values: &[T]) -> io::Result<()> {
        let mut buf = Vec::with_capacity(values.len() * T::TY.size());
        for &v in values {
            v.put(&mut buf);
        }
        let mut f = self
            .file
            .lock()
            .map_err(|_| io::Error::other("a writing thread failed"))?;
        f.seek(SeekFrom::Start(addr))?;
        f.write_all(&buf)
    }
}

impl H5Writer {
    /// Create (or replace) the file at `path`.
    pub fn create(path: &Path) -> io::Result<H5Writer> {
        let mut w = H5Writer {
            out: BufWriter::with_capacity(1 << 20, File::create(path)?),
            pos: 0,
            buf: Vec::new(),
            moved: false,
        };
        w.raw(&[0; SUPERBLOCK as usize])?;
        Ok(w)
    }

    /// The address the next bytes go to.
    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// Write bytes as they are.
    pub fn raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.moved {
            self.out.seek(SeekFrom::Start(self.pos))?;
            self.moved = false;
        }
        self.out.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    /// Write numbers as the raw data of a dataset (part of it: a dataset
    /// may be written in any number of calls between [`H5Writer::pos`]
    /// and [`H5Writer::dataset_header`]).
    pub fn put<T: Element>(&mut self, values: &[T]) -> io::Result<()> {
        let mut buf = std::mem::take(&mut self.buf);
        for chunk in values.chunks(1 << 16) {
            buf.clear();
            for &v in chunk {
                v.put(&mut buf);
            }
            self.raw(&buf)?;
        }
        self.buf = buf;
        Ok(())
    }

    /// The object header of a contiguous dataset of `ty` and `dims` whose
    /// data are the bytes from `start` to the current position; returns
    /// the header's address.
    pub fn dataset_header(&mut self, ty: Ty, dims: &[u64], start: u64) -> io::Result<u64> {
        let bytes = self.pos - start;
        let want: u64 = dims.iter().product::<u64>() * ty.size() as u64;
        if bytes != want {
            return Err(io::Error::other(format!(
                "dataset of {dims:?} × {} bytes has {bytes} bytes",
                ty.size()
            )));
        }
        self.contiguous(ty, dims, start, bytes)
    }

    fn contiguous(&mut self, ty: Ty, dims: &[u64], start: u64, bytes: u64) -> io::Result<u64> {
        let mut layout = vec![3, 1];
        layout.extend(if bytes == 0 { UNDEF } else { start }.to_le_bytes());
        layout.extend(bytes.to_le_bytes());
        // fill value: allocated late, written if set, none defined
        let fill = vec![3, 0x02 | (0x02 << 2)];
        self.header(&[
            (1, dataspace(Some(dims))),
            (3, ty.message()),
            (5, fill),
            (8, layout),
        ])
    }

    /// Set aside the raw data of a contiguous dataset of `ty` and `dims`
    /// from the current position, for [`Rows`] to fill in any order; its
    /// header follows with [`H5Writer::reserved_header`]. Nothing is
    /// written: the file has a hole there until the rows come.
    pub fn reserve(&mut self, ty: Ty, dims: &[u64]) -> io::Result<Reserved> {
        let start = self.pos;
        let bytes = dims.iter().product::<u64>() * ty.size() as u64;
        self.pos += bytes;
        self.out.seek(SeekFrom::Start(self.pos))?;
        Ok(Reserved {
            start,
            ty,
            dims: dims.to_vec(),
        })
    }

    /// Writes into the reserved datasets.
    pub fn rows(&mut self) -> io::Result<Rows<'_>> {
        self.out.flush()?;
        self.moved = true;
        Ok(Rows {
            file: std::sync::Mutex::new(self.out.get_ref()),
        })
    }

    /// The object header of a reserved dataset; returns its address.
    pub fn reserved_header(&mut self, r: &Reserved) -> io::Result<u64> {
        let bytes: u64 = r.dims.iter().product::<u64>() * r.ty.size() as u64;
        self.contiguous(r.ty, &r.dims, r.start, bytes)
    }

    /// A whole dataset of numbers; returns its header's address.
    pub fn dataset<T: Element>(&mut self, dims: &[u64], values: &[T]) -> io::Result<u64> {
        let start = self.pos;
        self.put(values)?;
        self.dataset_header(T::TY, dims, start)
    }

    /// A one-dimensional dataset of fixed-length strings, as long as the
    /// longest (at least one byte).
    pub fn strings(&mut self, values: &[String]) -> io::Result<u64> {
        let n = values.iter().map(String::len).max().unwrap_or(0).max(1);
        let start = self.pos;
        let mut buf = Vec::with_capacity(n * values.len());
        for v in values {
            buf.extend(v.as_bytes());
            buf.resize(buf.len() + n - v.len(), 0);
        }
        self.raw(&buf)?;
        self.dataset_header(text(n, values), &[values.len() as u64], start)
    }

    /// A group with `links` and `attrs`; returns its header's address.
    pub fn group(&mut self, links: &[Link], attrs: &[(&str, AttrValue)]) -> io::Result<u64> {
        let mut msgs: Vec<(u8, Vec<u8>)> = Vec::new();
        // link info: creation order tracked and indexed (VTK lists a
        // group's links by creation order, and the HDF5 library refuses
        // that order for a group that does not track it), no dense
        // storage
        let mut info = vec![0, 0x03];
        info.extend((links.len() as u64).to_le_bytes());
        info.extend(UNDEF.to_le_bytes());
        info.extend(UNDEF.to_le_bytes());
        info.extend(UNDEF.to_le_bytes());
        msgs.push((2, info));
        // group info: the defaults
        msgs.push((10, vec![0, 0]));
        for (order, link) in links.iter().enumerate() {
            let (name, soft) = match link {
                Link::Hard(n, _) => (n, None),
                Link::Soft(n, p) => (n, Some(p)),
            };
            let len = name.len();
            let size_bits: u8 = match len {
                0..=0xff => 0,
                0x100..=0xffff => 1,
                _ => 2,
            };
            let mut m = vec![
                1,
                size_bits | if soft.is_some() { 0x08 } else { 0 } | 0x04 | 0x10,
            ];
            if soft.is_some() {
                m.push(1);
            }
            m.extend((order as u64).to_le_bytes()); // creation order
            m.push(1); // UTF-8 name
            match size_bits {
                0 => m.push(len as u8),
                1 => m.extend((len as u16).to_le_bytes()),
                _ => m.extend((len as u32).to_le_bytes()),
            }
            m.extend(name.as_bytes());
            match link {
                Link::Hard(_, addr) => m.extend(addr.to_le_bytes()),
                Link::Soft(_, path) => {
                    m.extend((path.len() as u16).to_le_bytes());
                    m.extend(path.as_bytes());
                }
            }
            msgs.push((6, m));
        }
        // attribute info: no creation order, no dense storage. Without
        // it the HDF5 library counts no attributes in a version 2 header
        // (H5Aget_num_attrs = 0), although it finds each one by name.
        let mut info = vec![0, 0];
        info.extend(UNDEF.to_le_bytes());
        info.extend(UNDEF.to_le_bytes());
        msgs.push((21, info));
        for (name, value) in attrs {
            msgs.push((12, attribute(name, value)));
        }
        self.header(&msgs)
    }

    /// A version 2 object header holding `msgs` (type, data); returns its
    /// address.
    fn header(&mut self, msgs: &[(u8, Vec<u8>)]) -> io::Result<u64> {
        let body: usize = msgs.iter().map(|(_, d)| 4 + d.len()).sum();
        let mut h = Vec::with_capacity(16 + body);
        h.extend(b"OHDR");
        h.push(2);
        // chunk 0's size in 4 bytes; no times, no attribute order
        h.push(0x02);
        h.extend((body as u32).to_le_bytes());
        for (ty, data) in msgs {
            if data.len() > u16::MAX as usize {
                return Err(io::Error::other("an object header message over 64 KiB"));
            }
            h.push(*ty);
            h.extend((data.len() as u16).to_le_bytes());
            h.push(0);
            h.extend(data);
        }
        let sum = lookup3(&h, 0);
        h.extend(sum.to_le_bytes());
        let at = self.pos;
        self.raw(&h)?;
        Ok(at)
    }

    /// Write the superblock pointing at the root group `root` and close
    /// the file.
    pub fn finish(mut self, root: u64) -> io::Result<()> {
        let end = self.pos;
        let mut s = Vec::with_capacity(SUPERBLOCK as usize);
        s.extend(SIGNATURE);
        s.extend([2, 8, 8, 0]);
        s.extend(0u64.to_le_bytes());
        s.extend(UNDEF.to_le_bytes());
        s.extend(end.to_le_bytes());
        s.extend(root.to_le_bytes());
        let sum = lookup3(&s, 0);
        s.extend(sum.to_le_bytes());
        if self.moved {
            self.out.seek(SeekFrom::Start(self.pos))?;
        }
        self.out.flush()?;
        let mut f = self.out.into_inner().map_err(|e| e.into_error())?;
        // a reserved dataset at the very end is a hole the file must hold
        if f.metadata()?.len() < end {
            f.set_len(end)?;
        }
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&s)?;
        f.flush()?;
        f.sync_all()
    }
}

/// Strings of `n` bytes: ASCII when all of `values` are, else UTF-8.
fn text(n: usize, values: &[String]) -> Ty {
    if values.iter().all(|v| v.is_ascii()) {
        Ty::Str(n)
    } else {
        Ty::Utf8(n)
    }
}

/// The attribute message (version 3) of `name` = `value`.
fn attribute(name: &str, value: &AttrValue) -> Vec<u8> {
    let (ty, space, data) = match value {
        AttrValue::Int(v) => (Ty::I64, dataspace(None), v.to_le_bytes().to_vec()),
        AttrValue::Ints(v) => (
            Ty::I64,
            dataspace(Some(&[v.len() as u64])),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        ),
        AttrValue::Float(v) => (Ty::F64, dataspace(None), v.to_le_bytes().to_vec()),
        AttrValue::Text(t) => {
            let n = t.len().max(1);
            let mut d = t.as_bytes().to_vec();
            d.resize(n, 0);
            (text(n, std::slice::from_ref(t)), dataspace(None), d)
        }
    };
    let ty = ty.message();
    let mut m = vec![3, 0];
    m.extend(((name.len() + 1) as u16).to_le_bytes());
    m.extend((ty.len() as u16).to_le_bytes());
    m.extend((space.len() as u16).to_le_bytes());
    m.push(1); // UTF-8 name
    m.extend(name.as_bytes());
    m.push(0);
    m.extend(ty);
    m.extend(space);
    m.extend(data);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdf5::{H5, Target};

    /// Invariant: lookup3 gives the published values of Jenkins' own
    /// self-test (`driver5` of lookup3.c): the empty string hashes to
    /// 0xdeadbeef with seed 0, "Four score and seven years ago" to
    /// 0x17770551 with seed 0 and to 0xcd628161 with seed 1. A wrong
    /// round makes every checksum of the file wrong, and HDF5 refuses it.
    #[test]
    fn lookup3_matches_the_published_values() {
        assert_eq!(lookup3(b"", 0), 0xdead_beef);
        let s = b"Four score and seven years ago";
        assert_eq!(lookup3(s, 0), 0x1777_0551);
        assert_eq!(lookup3(s, 1), 0xcd62_8161);
    }

    /// Invariant: a reserved dataset reads back as the rows written into
    /// it, whatever their order and thread, with ordinary datasets
    /// written between the reservation and the rows, and a reservation
    /// last in the file (a hole the file must still hold). 32-bit signed
    /// integers keep their sign. Breaks when the writer's position and
    /// the file's part ways after the positioned writes.
    #[test]
    fn reserved_datasets_take_their_rows_in_any_order() {
        let dir = std::env::temp_dir().join(format!("murmel-h5reserve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.h5");
        let mut w = H5Writer::create(&path).unwrap();
        let ids = w.reserve(Ty::I32, &[6]).unwrap();
        let before = w.dataset(&[2], &[5i64, 6]).unwrap();
        let xyz = w.reserve(Ty::F64, &[4, 3]).unwrap();
        assert_eq!(xyz.at(2), xyz.start + 2 * 24);
        {
            let rows = w.rows().unwrap();
            std::thread::scope(|s| {
                s.spawn(|| rows.put(xyz.at(2), &[7.0f64, 8.0, 9.0, 10.0, 11.0, 12.0]));
                s.spawn(|| rows.put(ids.at(3), &[-4i32, 5, -6]));
            });
            rows.put(ids.at(0), &[1i32, -2, 3]).unwrap();
            rows.put(xyz.at(0), &[1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0])
                .unwrap();
        }
        let ids_at = w.reserved_header(&ids).unwrap();
        let xyz_at = w.reserved_header(&xyz).unwrap();
        let root = w
            .group(
                &[
                    Link::Hard("ids".into(), ids_at),
                    Link::Hard("before".into(), before),
                    Link::Hard("xyz".into(), xyz_at),
                ],
                &[],
            )
            .unwrap();
        // a reservation nothing follows
        let tail = w.reserve(Ty::U8, &[3]).unwrap();
        w.rows().unwrap().put(tail.at(0), &[1u8, 2]).unwrap();
        w.finish(root).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), tail.start + 3);

        let h = H5::open(&path).unwrap();
        let g = h.path("/").unwrap();
        let read = |name: &str| h.dataset(&h.child(&g, name).unwrap().unwrap()).unwrap();
        assert_eq!(h.read_i64(&read("ids")).unwrap(), [1, -2, 3, -4, 5, -6]);
        assert_eq!(h.read_i64(&read("before")).unwrap(), [5, 6]);
        let x = read("xyz");
        assert_eq!(x.dims, [4, 3]);
        let want: Vec<f64> = (1..=12).map(f64::from).collect();
        assert_eq!(h.read_f64(&x).unwrap(), want);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Invariant: text that is ASCII is marked ASCII (character set 0)
    /// and other text UTF-8 (1), in datasets and attributes alike. VTK
    /// 9.4's reader refuses the `Type` attribute of every file marked
    /// UTF-8, and the round trip below reads either the same, so only
    /// this test sees the mark.
    #[test]
    fn ascii_text_is_marked_ascii() {
        let set = |t: Ty| t.message()[1] >> 4;
        assert_eq!(set(text(4, &["Type".into(), "abc".into()])), 0);
        assert_eq!(set(text(6, &["1=rotör".into()])), 1);
        assert_eq!(set(text(1, &[])), 0);
        // in an attribute message (version 3) the datatype follows the
        // 9 bytes of its header and the name with its null
        let m = attribute("Type", &AttrValue::Text("UnstructuredGrid".into()));
        assert_eq!((m[9 + 5], m[9 + 5 + 1] >> 4), (0x13, 0));
        let m = attribute("case", &AttrValue::Text("Föhn".into()));
        assert_eq!((m[9 + 5], m[9 + 5 + 1] >> 4), (0x13, 1));
    }

    /// Invariant: what the writer writes, MurmelCraft's own reader reads
    /// back: groups, hard and soft links, integer, float and string
    /// datasets of one and two dimensions, empty datasets, and integer,
    /// integer-array and string attributes. Breaks on any wrong message
    /// layout.
    #[test]
    fn the_reader_reads_what_the_writer_writes() {
        let dir = std::env::temp_dir().join(format!("murmel-h5write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.h5");
        let mut w = H5Writer::create(&path).unwrap();
        let xyz = w
            .dataset(&[2, 3], &[0.5f64, 1.0, 1.5, 2.0, 2.5, 3.0])
            .unwrap();
        let ids = w.dataset(&[3], &[7i64, -1, 1 << 40]).unwrap();
        let kinds = w.dataset(&[2], &[12u8, 42]).unwrap();
        let zone = w.dataset(&[2], &[0u32, 3]).unwrap();
        let none = w.dataset::<i64>(&[0], &[]).unwrap();
        let names = w
            .strings(&["0=fluid".to_string(), "1=rotör".to_string()])
            .unwrap();
        let inner = w
            .group(
                &[
                    Link::Hard("Points".into(), xyz),
                    Link::Hard("Ids".into(), ids),
                    Link::Hard("Types".into(), kinds),
                    Link::Hard("zone".into(), zone),
                    Link::Hard("Empty".into(), none),
                    Link::Hard("zone_names".into(), names),
                ],
                &[
                    ("Index", AttrValue::Int(0)),
                    ("Type", AttrValue::Text("UnstructuredGrid".into())),
                ],
            )
            .unwrap();
        let top = w
            .group(
                &[
                    Link::Hard("internal".into(), inner),
                    Link::Soft("alias".into(), "/VTKHDF/internal".into()),
                ],
                &[("Version", AttrValue::Ints(vec![2, 4]))],
            )
            .unwrap();
        let root = w.group(&[Link::Hard("VTKHDF".into(), top)], &[]).unwrap();
        w.finish(root).unwrap();

        let h = H5::open(&path).unwrap();
        let g = h.path("/VTKHDF/internal").unwrap();
        let read = |name: &str| h.dataset(&h.child(&g, name).unwrap().unwrap()).unwrap();
        let p = read("Points");
        assert_eq!(p.dims, [2, 3]);
        assert_eq!(h.read_f64(&p).unwrap(), [0.5, 1.0, 1.5, 2.0, 2.5, 3.0]);
        assert_eq!(h.read_i64(&read("Ids")).unwrap(), [7, -1, 1 << 40]);
        assert_eq!(h.read_i64(&read("Types")).unwrap(), [12, 42]);
        assert_eq!(h.read_i64(&read("zone")).unwrap(), [0, 3]);
        assert!(h.read_i64(&read("Empty")).unwrap().is_empty());
        let t = h.attr(&g, "Type").unwrap().unwrap();
        assert_eq!(t.text().as_deref(), Some("UnstructuredGrid"));
        assert_eq!(h.attr(&g, "Index").unwrap().unwrap().number(), Some(0.0));
        let top = h.path("/VTKHDF").unwrap();
        let links = h.links(&top).unwrap();
        assert!(
            links
                .iter()
                .any(|l| l.name == "alias" && matches!(l.target, Target::Soft(_)))
        );
        let names = h.read_strings(&read("zone_names")).unwrap();
        assert_eq!(names, ["0=fluid", "1=rotör"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
