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
//! The whole file is held in memory. Every address and size is checked
//! against the file before it is used, so a broken file gives `MESH016`
//! and never a panic or an allocation larger than the file allows.

use std::collections::HashSet;
use std::io::Read as _;
use std::ops::Range;

use crate::MeshError;

const SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";

/// Largest number of continuation blocks, B-tree nodes or heap blocks
/// one walk visits; a cycle in a broken file stops here.
const MAX_VISITS: usize = 1 << 20;

fn broken(message: impl Into<String>) -> MeshError {
    MeshError::new("MESH016", message)
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

/// A read position in the file.
#[derive(Clone)]
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
    o: usize,
    l: usize,
}

impl<'a> Cur<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], MeshError> {
        let end = self
            .p
            .checked_add(n)
            .filter(|&e| e <= self.d.len())
            .ok_or_else(|| broken(format!("read of {n} bytes at {} runs past the end", self.p)))?;
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
    data: Vec<u8>,
    o: usize,
    l: usize,
    base: u64,
    root: u64,
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
        let at = at.ok_or_else(|| MeshError::new("MESH018", "the file is not an HDF5 file"))?;
        let mut c = Cur {
            d: &data,
            p: at + 8,
            o: 8,
            l: 8,
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
            data,
            o,
            l,
            base,
            root,
        })
    }

    pub fn open(path: &std::path::Path) -> Result<H5, MeshError> {
        let data = std::fs::read(path).map_err(|e| {
            MeshError::new("MESH001", format!("cannot read {}: {e}", path.display()))
        })?;
        H5::from_bytes(data)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// The bytes of the file.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn cur(&self, addr: u64) -> Result<Cur<'_>, MeshError> {
        let p = self
            .base
            .checked_add(addr)
            .and_then(|a| usize::try_from(a).ok())
            .filter(|&a| a <= self.data.len())
            .ok_or_else(|| broken(format!("address {addr} is outside the file")))?;
        Ok(Cur {
            d: &self.data,
            p,
            o: self.o,
            l: self.l,
        })
    }

    /// A cursor over a message's data.
    fn msg_cur(&self, r: &Range<usize>) -> Cur<'_> {
        Cur {
            d: &self.data[..r.end],
            p: r.start,
            o: self.o,
            l: self.l,
        }
    }

    /// The bytes at `addr`, `n` long.
    fn slice(&self, addr: u64, n: u64) -> Result<&[u8], MeshError> {
        let n = usize::try_from(n).map_err(|_| broken("size too large"))?;
        let mut c = self.cur(addr)?;
        c.bytes(n)
    }

    pub fn root(&self) -> Result<Object, MeshError> {
        self.object(self.root)
    }

    /// The object at a header address.
    pub fn object(&self, addr: u64) -> Result<Object, MeshError> {
        let mut c = self.cur(addr)?;
        let mut msgs = Vec::new();
        if c.d.get(c.p..c.p + 4) == Some(b"OHDR") {
            self.header_v2(&mut c, &mut msgs)?;
        } else {
            self.header_v1(&mut c, &mut msgs)?;
        }
        Ok(Object { addr, msgs })
    }

    fn header_v1(&self, c: &mut Cur<'_>, msgs: &mut Vec<Msg>) -> Result<(), MeshError> {
        let start = c.p;
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
        let mut blocks = vec![(c.p, size)];
        let mut seen = HashSet::new();
        let mut count = 0usize;
        while let Some((p, size)) = blocks.pop() {
            if !seen.insert(p) || seen.len() > MAX_VISITS {
                return Err(broken("object header continuation loops"));
            }
            let end = p
                .checked_add(size)
                .filter(|&e| e <= self.data.len())
                .ok_or_else(|| broken("object header runs past the end"))?;
            let mut m = Cur {
                d: &self.data[..end],
                p,
                o: self.o,
                l: self.l,
            };
            while m.p + 8 <= end && count < total {
                let ty = m.u16()?;
                let len = m.u16()? as usize;
                m.skip(4)?; // flags and reserved
                let at = m.p;
                m.skip(len)?;
                count += 1;
                if ty == 0x10 {
                    let mut k = self.msg_cur(&(at..at + len));
                    let a = k
                        .addr()?
                        .ok_or_else(|| broken("continuation without address"))?;
                    let n = k.len()?;
                    let a = self.cur(a)?.p;
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
        let mut blocks = vec![(c.p, size)];
        let mut seen = HashSet::new();
        while let Some((p, size)) = blocks.pop() {
            if !seen.insert(p) || seen.len() > MAX_VISITS {
                return Err(broken("object header continuation loops"));
            }
            let end = p
                .checked_add(size)
                .filter(|&e| e <= self.data.len())
                .ok_or_else(|| broken("object header runs past the end"))?;
            let mut m = Cur {
                d: &self.data[..end],
                p,
                o: self.o,
                l: self.l,
            };
            let head = if order { 6 } else { 4 };
            while m.p + head <= end {
                let ty = m.u8()? as u16;
                let len = m.u16()? as usize;
                m.skip(1)?; // flags
                if order {
                    m.skip(2)?;
                }
                let at = m.p;
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
                    blocks.push((b.p, n.saturating_sub(8)));
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
            out.push(self.link_msg(&self.data[m.data.clone()])?);
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
        let mut c = Cur {
            d,
            p: 0,
            o: self.o,
            l: self.l,
        };
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
            if let Some(a) = self.attr_msg(&self.data[m.data.clone()])? {
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
        let mut c = Cur {
            d,
            p: 0,
            o: self.o,
            l: self.l,
        };
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
                let mut c = Cur {
                    d: raw,
                    p: 0,
                    o: self.o,
                    l: self.l,
                };
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
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| broken("global heap runs past the end"))?;
        let mut c = Cur {
            d: &self.data[..end],
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
                        let at = c.p;
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
                        let at = c.p;
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
        let elem = ds.dtype.size();
        if elem == 0 {
            return Err(unsupported("reading a dataset of this datatype"));
        }
        let total = ds
            .count()
            .checked_mul(elem as u64)
            .ok_or_else(|| broken("dataset size overflows"))?;
        // Compressed chunks may expand: allow deflate's largest ratio.
        let limit = (self.data.len() as u64).saturating_mul(1100).max(1 << 20);
        if total > limit {
            return Err(MeshError::new(
                "MESH004",
                format!(
                    "a dataset of {total} bytes cannot come from a file of {} bytes",
                    self.data.len()
                ),
            ));
        }
        let total = total as usize;
        match &ds.layout {
            Layout::Compact(r) => {
                let b = self
                    .data
                    .get(r.clone())
                    .ok_or_else(|| broken("compact data outside the file"))?;
                if b.len() < total {
                    return Err(broken("compact data too short"));
                }
                Ok(b[..total].to_vec())
            }
            Layout::Contiguous { addr, size } => match addr {
                None => Ok(vec![0; total]),
                Some(a) => {
                    if *size < total as u64 {
                        return Err(broken("contiguous data shorter than its dataspace"));
                    }
                    Ok(self.slice(*a, total as u64)?.to_vec())
                }
            },
            Layout::Chunked { dims, index } => self.chunked(ds, dims, index, elem, total),
        }
    }

    fn chunked(
        &self,
        ds: &Dataset,
        cdims: &[u64],
        index: &ChunkIndex,
        elem: usize,
        total: usize,
    ) -> Result<Vec<u8>, MeshError> {
        let rank = ds.dims.len();
        if cdims.len() != rank || cdims.contains(&0) {
            return Err(broken("chunk rank differs from the dataspace"));
        }
        let chunk_elems: u64 = cdims.iter().product();
        let chunk_bytes = usize::try_from(chunk_elems * elem as u64)
            .ok()
            .filter(|&b| b as u64 <= (self.data.len() as u64).saturating_mul(1100).max(1 << 20))
            .ok_or_else(|| broken("chunk too large"))?;
        let mut out = vec![0u8; total];
        // chunk offsets (in elements) and raw chunk bytes with filter mask
        let mut chunks: Vec<(Vec<u64>, Vec<u8>, u32)> = Vec::new();
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
            | ChunkIndex::FixedArray(None) => return Ok(out),
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
                            chunks.push((off, self.slice(child, size)?.to_vec(), mask));
                        }
                    }
                }
            }
            ChunkIndex::Single {
                addr: Some(a),
                filtered_size,
                mask,
            } => {
                let size = filtered_size.unwrap_or(chunk_bytes as u64);
                chunks.push((vec![0; rank], self.slice(*a, size)?.to_vec(), *mask));
            }
            ChunkIndex::Implicit(Some(a)) => {
                let n: u64 = grid.iter().product();
                for i in 0..n {
                    let at = a + i * chunk_bytes as u64;
                    chunks.push((
                        offsets_of(i),
                        self.slice(at, chunk_bytes as u64)?.to_vec(),
                        0,
                    ));
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
                let Some(block) = block else { return Ok(out) };
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
                        chunks.push((offsets_of(i), self.slice(addr, size)?.to_vec(), mask));
                    }
                }
            }
        }
        for (off, raw, mask) in chunks {
            let bytes = self.unfilter(&ds.filters, raw, mask, chunk_bytes)?;
            if bytes.len() < chunk_bytes {
                return Err(broken("chunk shorter than its dimensions"));
            }
            place(&mut out, &bytes, &ds.dims, cdims, &off, elem);
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
                    d: id,
                    p: 1,
                    o: self.o,
                    l: self.l,
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
