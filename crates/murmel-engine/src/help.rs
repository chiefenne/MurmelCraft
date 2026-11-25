//! The help pages behind the tooltips, the Help panel and the manual:
//! one Markdown file per topic in `assets/help/en/`, built into the
//! program so help works offline (the maintainer's card of 2026-11-25).
//! The short name and the one-line tip of a topic stay in the Fluent file
//! (`help-<page>-title`, `help-<page>-line`), so translators work in one
//! place; a page holds the rest.
//!
//! A page starts with a TOML header between `+++` lines:
//!
//! ```text
//! +++
//! covers = ["view.fit", "view.zoom*"]   # command or tool ids; `*` ends a prefix
//! related = ["views", "projection"]     # other pages, by file name
//! chapter = "navigation"                # where the manual puts it
//! picture = "fit.webp"                  # optional, rendered from `shot`
//! shot = "shots/fit.json"
//! +++
//! The rich tip: two or three short paragraphs.
//!
//! <!-- more -->
//! The rest of the manual section.
//! ```

use std::sync::OnceLock;

use serde::Deserialize;

/// The marker between a page's rich tip and the rest of it.
pub const MORE: &str = "<!-- more -->";

/// The pages, by file name (without `.md`), built in.
macro_rules! pages {
    ($($name:literal),* $(,)?) => {
        /// The file name and text of every page.
        pub const FILES: &[(&str, &str)] = &[
            $(($name, include_str!(concat!("../../../assets/help/en/", $name, ".md")))),*
        ];
    };
}

pages!(
    "open",
    "sessions",
    "export",
    "quit",
    "palette",
    "shortcuts",
    "panels",
    "settings",
    "defaults",
    "fit-zoom",
    "views",
    "projection",
    "saved-views",
    "render-mode",
    "color-by",
    "quality",
    "visibility",
    "selection",
    "clip",
    "field-style",
    "derived",
    "slices",
    "iso",
    "arrows",
    "streamlines",
    "parcels",
    "series",
    "charts",
    "regions",
    "mesh-look",
    "measure",
    "probes",
    "undo",
    "help",
);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    #[serde(default)]
    covers: Vec<String>,
    #[serde(default)]
    related: Vec<String>,
    chapter: String,
    #[serde(default)]
    picture: Option<String>,
    #[serde(default)]
    shot: Option<String>,
}

/// One help page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// The file name without `.md`.
    pub name: &'static str,
    /// The ids it explains: exact, or a prefix ending in `*`.
    pub covers: Vec<String>,
    /// Other pages worth reading next.
    pub related: Vec<String>,
    /// The manual's chapter.
    pub chapter: String,
    pub picture: Option<String>,
    pub shot: Option<String>,
    /// The rich tip: the text above [`MORE`].
    pub tip: String,
    /// The whole text, tip included.
    pub text: String,
}

impl Page {
    /// The Fluent key of its name.
    pub fn title_key(&self) -> String {
        format!("help-{}-title", self.name)
    }

    /// The Fluent key of its one-line tip.
    pub fn line_key(&self) -> String {
        format!("help-{}-line", self.name)
    }

    /// Whether it explains `id`, and how closely: an exact id beats a
    /// prefix, and a longer prefix a shorter one.
    fn score(&self, id: &str) -> Option<usize> {
        self.covers
            .iter()
            .filter_map(|c| match c.strip_suffix('*') {
                Some(prefix) => id.starts_with(prefix).then_some(prefix.len()),
                None => (c == id).then_some(usize::MAX),
            })
            .max()
    }
}

/// Read a page; `Err` says what is wrong with it.
pub fn parse(name: &'static str, source: &str) -> Result<Page, String> {
    let rest = source
        .strip_prefix("+++\n")
        .ok_or_else(|| format!("{name}.md: no +++ header"))?;
    let (header, body) = rest
        .split_once("\n+++\n")
        .ok_or_else(|| format!("{name}.md: the header has no end"))?;
    let h: Header = toml::from_str(header).map_err(|e| format!("{name}.md: {e}"))?;
    let text = body.trim().to_string();
    let tip = text.split(MORE).next().unwrap_or("").trim().to_string();
    Ok(Page {
        name,
        covers: h.covers,
        related: h.related,
        chapter: h.chapter,
        picture: h.picture,
        shot: h.shot,
        tip,
        text: text.replace(MORE, "").replace("\n\n\n", "\n\n"),
    })
}

/// Every page; one that cannot be read is left out (a test reads them
/// all).
pub fn pages() -> &'static [Page] {
    static PAGES: OnceLock<Vec<Page>> = OnceLock::new();
    PAGES.get_or_init(|| FILES.iter().filter_map(|(n, s)| parse(n, s).ok()).collect())
}

/// The page by its file name.
pub fn page(name: &str) -> Option<&'static Page> {
    pages().iter().find(|p| p.name == name)
}

/// The page that explains `id` (a command, a tool or a topic such as
/// `ui.tree`), the closest match first; `None` when no page does.
pub fn page_for(id: &str) -> Option<&'static Page> {
    pages()
        .iter()
        .filter_map(|p| p.score(id).map(|s| (s, p)))
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: every page in assets/help/en is built in, reads, names
    /// a chapter, covers at least one id, has a rich tip, and its related
    /// pages exist. Breaks when a page is added to the folder but not to
    /// `pages!` (it would never show), or its header is mistyped (it
    /// would be left out without a word). The tools test checks that
    /// every command has a page.
    #[test]
    fn every_page_reads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/help/en");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| {
                let n = e.ok()?.file_name().to_string_lossy().into_owned();
                n.strip_suffix(".md").map(String::from)
            })
            .collect();
        on_disk.sort();
        let mut built: Vec<String> = FILES.iter().map(|(n, _)| n.to_string()).collect();
        built.sort();
        assert_eq!(on_disk, built, "assets/help/en and pages! differ");
        for (n, s) in FILES {
            let p = parse(n, s).unwrap();
            assert!(!p.covers.is_empty(), "{n} covers nothing");
            assert!(!p.tip.is_empty(), "{n} has no tip");
            assert!(!p.chapter.is_empty(), "{n} has no chapter");
            for r in &p.related {
                assert!(page(r).is_some(), "{n}: related {r} is no page");
            }
        }
        assert_eq!(pages().len(), FILES.len());
    }

    /// Invariant: an id goes to the page that names it exactly, else the
    /// one with the longest prefix. Breaks if `display.color_by:quality-…`
    /// went to the colour-by page instead of the quality one.
    #[test]
    fn closest_page_wins() {
        let p = |covers: &[&str]| Page {
            name: "x",
            covers: covers.iter().map(|s| s.to_string()).collect(),
            related: vec![],
            chapter: "c".into(),
            picture: None,
            shot: None,
            tip: String::new(),
            text: String::new(),
        };
        let wide = p(&["display.color_by*"]);
        let narrow = p(&["display.color_by:quality*"]);
        let id = "display.color_by:quality-skewness";
        assert!(narrow.score(id) > wide.score(id));
        assert_eq!(p(&["view.fit"]).score("view.fit"), Some(usize::MAX));
        assert_eq!(p(&["view.fit"]).score("view.fit:selection"), None);
    }
}
