//! What reading a mesh whole would take, from the head of its files
//! alone (large-mesh post-processing, the maintainer's card 4 of
//! 2026-11-24: a large file that is not blocked is offered for
//! conversion before it is read): its cells and about the memory a whole
//! read peaks at.
//!
//! - **VTKHDF**: the structure ([`crate::vtkhdf::size_of`]), fields
//!   included.
//! - **`.vtu`**: the `NumberOfCells` of its `Piece`, in the first MiB.
//! - **OpenFOAM `polyMesh`**: the `nCells` OpenFOAM notes in the header
//!   of `owner` (or `owner.gz`).
//!
//! Fields of a `.vtu` are not counted (8 bytes a value, little beside the
//! cells' [`OPEN_BYTES_PER_CELL`]); a `polyMesh` whose `owner` has no
//! note gives no estimate.

use std::io::Read;
use std::path::Path;

use crate::vtkhdf::{self, OPEN_BYTES_PER_CELL};

/// How much of a file's head is read for its counts.
const HEAD: u64 = 1 << 20;

/// What a whole read of a mesh would build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Estimate {
    /// The cells: the volume's, and the boundary faces a `.vtu` or an
    /// unblocked VTKHDF file stores as cells too (the estimate errs
    /// high).
    pub cells: u64,
    /// About the memory a whole read peaks at.
    pub open_bytes: u64,
    /// A blocked VTKHDF file (it can open as an overview).
    pub blocked: bool,
}

/// The estimate for the mesh at `path`, or `None` when its head does not
/// say (or it is not a mesh).
pub fn estimate(path: &Path) -> Option<Estimate> {
    if path.is_file() && vtkhdf::is_vtkhdf(path) {
        let s = vtkhdf::size_of(path).ok()?;
        return Some(Estimate {
            cells: s.cells,
            open_bytes: s.open_bytes(),
            blocked: s.blocked,
        });
    }
    let vtu = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vtu"));
    let cells = if !vtu && crate::polymesh::is_openfoam(path) {
        polymesh_cells(path)?
    } else {
        vtu_cells(path)?
    };
    Some(Estimate {
        cells,
        open_bytes: cells.saturating_mul(OPEN_BYTES_PER_CELL),
        blocked: false,
    })
}

/// The first [`HEAD`] bytes of `r`.
fn head(r: impl Read) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    r.take(HEAD).read_to_end(&mut v).ok()?;
    Some(v)
}

/// The number after `key` in `text` (digits, spaces before them aside).
fn number_after(text: &[u8], key: &[u8]) -> Option<u64> {
    let at = memchr::memmem::find(text, key)? + key.len();
    let rest = &text[at..];
    let start = rest.iter().position(|b| !b.is_ascii_whitespace())?;
    let digits: Vec<u8> = rest[start..]
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

/// The cells of the `.vtu` at `path`, from its `Piece`.
fn vtu_cells(path: &Path) -> Option<u64> {
    let text = head(std::fs::File::open(path).ok()?)?;
    let piece = memchr::memmem::find(&text, b"<Piece")?;
    number_after(&text[piece..], b"NumberOfCells=\"")
}

/// The cells of the `polyMesh` at `path`, from the note in `owner`.
fn polymesh_cells(path: &Path) -> Option<u64> {
    let dir = crate::polymesh::polymesh_dir(path).ok()?;
    let plain = dir.join("owner");
    let text = if plain.is_file() {
        head(std::fs::File::open(plain).ok()?)?
    } else {
        let gz = std::fs::File::open(dir.join("owner.gz")).ok()?;
        head(flate2::read::GzDecoder::new(gz))?
    };
    // The header is the first dictionary: the note is in it or nowhere.
    let end = memchr::memchr(b'}', &text)?;
    number_after(&text[..end], b"nCells:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    /// Invariant: the estimate's cells are the cells a whole read gives
    /// (with a `.vtu`'s boundary faces stored as cells, never fewer), for
    /// a `.vtu`, a `polyMesh` (ascii, binary, by its case, `constant`
    /// or `polyMesh` folder or a file of it, and gzip-compressed), a
    /// VTKHDF file and a blocked one, which says it is blocked; the
    /// memory is [`OPEN_BYTES_PER_CELL`] a cell at least; a missing file
    /// or a `polyMesh` without the note gives none. Breaks when a header
    /// count is read wrong or from the wrong place (another element's
    /// attribute, the face list). No other test estimates before reading.
    #[test]
    fn estimates_agree_with_the_read() {
        let poly = fixture("openfoam/channel_ascii");
        let mut paths = vec![
            fixture("channel/foehncraft.vtu"),
            fixture("box/foehncraft.vtu"),
            fixture("vtkhdf/channel_fields.vtkhdf"),
            poly.clone(),
            poly.join("constant"),
            poly.join("constant/polyMesh"),
            poly.join("constant/polyMesh/faces"),
            fixture("openfoam/channel_binary"),
        ];
        let dir = std::env::temp_dir().join(format!("murmel-estimate-{}", std::process::id()));
        // gzip-compressed, as OpenFOAM writes with writeCompression on
        let gz = dir.join("constant/polyMesh");
        std::fs::create_dir_all(&gz).unwrap();
        for e in std::fs::read_dir(poly.join("constant/polyMesh")).unwrap() {
            let e = e.unwrap();
            let bytes = std::fs::read(e.path()).unwrap();
            let name = format!("{}.gz", e.file_name().to_string_lossy());
            let mut w = flate2::write::GzEncoder::new(
                std::fs::File::create(gz.join(name)).unwrap(),
                flate2::Compression::fast(),
            );
            std::io::Write::write_all(&mut w, &bytes).unwrap();
            w.finish().unwrap();
        }
        paths.push(dir.clone());
        let (mesh, results) =
            crate::read_results(&fixture("vtkhdf/channel_fields.vtkhdf")).unwrap();
        let blocked = dir.join("blocked.vtkhdf");
        crate::blocks::write_blocked(&blocked, &mesh, &results, 100).unwrap();
        paths.push(blocked.clone());
        for p in &paths {
            let e = estimate(p).unwrap_or_else(|| panic!("{}", p.display()));
            let (m, _) = crate::read_results(p).unwrap();
            let vtu = p.extension().is_some_and(|x| x == "vtu");
            if vtu {
                assert!(e.cells >= m.cell_count() as u64, "{}", p.display());
            } else {
                assert_eq!(e.cells, m.cell_count() as u64, "{}", p.display());
            }
            assert!(e.open_bytes >= e.cells * OPEN_BYTES_PER_CELL);
            assert_eq!(e.blocked, *p == blocked, "{}", p.display());
        }
        assert_eq!(estimate(&dir.join("nothing.vtu")), None);
        // An owner without the note.
        let owner = gz.join("owner.gz");
        std::fs::remove_file(&owner).unwrap();
        let text = std::fs::read_to_string(poly.join("constant/polyMesh/owner")).unwrap();
        let start = text.find("note").unwrap();
        let end = start + text[start..].find('\n').unwrap();
        std::fs::write(
            gz.join("owner"),
            format!("{}{}", &text[..start], &text[end..]),
        )
        .unwrap();
        assert_eq!(estimate(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
