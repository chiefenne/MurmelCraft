//! The pictures of the help pages, rendered from recipes (the maintainer's
//! card of 2026-11-25): a page with `shot = "shots/x.json"` and
//! `picture = "x.png"` gets its picture from that recipe, a JSON list of
//! `{"tool", "input"}` calls like `murmel run --script` that ends in an
//! `export.image` call without a path. `murmel help shots` runs each
//! recipe in a fresh session with the built-in settings, writes the PNG
//! to `assets/help/pictures/` and its recipe's BLAKE3 digest to
//! `recipes.toml` there, so a test sees a recipe changed without its
//! picture made again. Paths in a recipe are from the repository's root.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use murmel_engine::help::Page;
use serde_json::{Value, json};

/// The folder of the pictures and of `recipes.toml`, from the root.
pub const PICTURES: &str = "assets/help/pictures";
/// The digests of the recipes the pictures were made from.
pub const DIGESTS: &str = "recipes.toml";
/// The pictures' size in pixels: shown at half that in a tip and in the
/// Help panel, so sharp on a Retina screen.
pub const SIZE: (u32, u32) = (640, 400);

/// The digest a recipe's text is kept under.
pub fn digest(recipe: &str) -> String {
    murmel_mesh::FileHash::Blake3.of_bytes(recipe.as_bytes())
}

/// The calls of a recipe; `Err` says what is wrong with it.
pub fn calls(recipe: &str) -> Result<Vec<(String, Value)>, String> {
    let v: Value = serde_json::from_str(recipe).map_err(|e| e.to_string())?;
    let list = v.as_array().ok_or("a recipe is a JSON list of calls")?;
    let calls: Vec<(String, Value)> = list
        .iter()
        .map(|c| {
            let tool = c["tool"].as_str().ok_or("a call without \"tool\"")?;
            Ok((
                tool.to_string(),
                c.get("input").cloned().unwrap_or(json!({})),
            ))
        })
        .collect::<Result<_, String>>()?;
    match calls.last() {
        Some((t, input)) if t == "export.image" && input.get("path").is_none() => Ok(calls),
        _ => Err("a recipe ends in export.image without a path".into()),
    }
}

/// The digests in `recipes.toml`, by page.
pub fn digests(root: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(root.join(PICTURES).join(DIGESTS))
        .ok()
        .and_then(|t| toml_digests(&t))
        .unwrap_or_default()
}

/// `name = "digest"` lines (the file has nothing else).
fn toml_digests(text: &str) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (k, v) = l.split_once('=')?;
        out.insert(
            k.trim().trim_matches('"').to_string(),
            v.trim().trim_matches('"').to_string(),
        );
    }
    Some(out)
}

/// Run a page's recipe and write its picture; returns the picture's path
/// and the recipe's digest.
pub fn render(page: &Page, root: &Path) -> Result<(PathBuf, String), String> {
    let (Some(shot), Some(picture)) = (&page.shot, &page.picture) else {
        return Err(format!("{}: no shot and picture", page.name));
    };
    let text = std::fs::read_to_string(root.join("assets/help").join(shot))
        .map_err(|e| format!("{shot}: {e}"))?;
    let calls = calls(&text).map_err(|e| format!("{shot}: {e}"))?;
    let out = root.join(PICTURES).join(picture);
    // The built-in settings, so a picture does not show yours.
    let mut s = murmel_engine::Session::new(
        murmel_engine::theme::Tokens::builtin().map_err(|e| e.to_string())?,
    );
    s.viewport = [SIZE.0 as f64 / 2.0, SIZE.1 as f64 / 2.0];
    let last = calls.len() - 1;
    for (k, (tool, mut input)) in calls.into_iter().enumerate() {
        if let Some(p) = input.get("path").and_then(Value::as_str) {
            input["path"] = json!(root.join(p));
        }
        if k == last {
            input["path"] = json!(out);
            input["width"] = json!(SIZE.0);
            input["height"] = json!(SIZE.1);
        }
        let e = crate::call(&tool, input, &mut s);
        if !e.ok() {
            return Err(format!("{shot}: {tool}: {:?}", e.diagnostics));
        }
    }
    Ok((out, digest(&text)))
}

/// Make the pictures of the pages named in `only` (all with a recipe when
/// empty) and keep their digests; returns the pictures written.
pub fn render_all(root: &Path, only: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut kept = digests(root);
    let mut written = Vec::new();
    for p in murmel_engine::help::pages() {
        if p.shot.is_none() || !(only.is_empty() || only.contains(&p.name)) {
            continue;
        }
        let (path, d) = render(p, root)?;
        kept.insert(p.name.to_string(), d);
        written.push(path);
    }
    let mut text = String::from(
        "# The BLAKE3 digest of the recipe each picture was made from.\n\
         # Written by `murmel help shots`; a test compares them.\n",
    );
    for (k, v) in &kept {
        text.push_str(&format!("{k} = \"{v}\"\n"));
    }
    std::fs::write(root.join(PICTURES).join(DIGESTS), text).map_err(|e| e.to_string())?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Invariant: every page with a recipe has its picture, a recipe
    /// names only tools that exist and ends in a picture, and the picture
    /// was made from the recipe as it is (its digest in recipes.toml).
    /// Breaks when a recipe is edited and `murmel help shots` not run, or
    /// a page names a picture that is not there.
    #[test]
    fn pictures_match_their_recipes() {
        let root = root();
        let kept = digests(&root);
        let mut n = 0;
        for p in murmel_engine::help::pages() {
            assert_eq!(
                p.shot.is_some(),
                p.picture.is_some(),
                "{}: shot and picture go together",
                p.name
            );
            let (Some(shot), Some(picture)) = (&p.shot, &p.picture) else {
                continue;
            };
            n += 1;
            let text = std::fs::read_to_string(root.join("assets/help").join(shot)).unwrap();
            for (tool, _) in calls(&text).unwrap() {
                assert!(crate::find(&tool).is_some(), "{shot}: no tool {tool}");
            }
            assert!(
                root.join(PICTURES).join(picture).is_file(),
                "{}: no {picture}; run `cargo run -p murmel-cli -- help shots {}`",
                p.name,
                p.name
            );
            assert_eq!(
                kept.get(p.name),
                Some(&digest(&text)),
                "{}: the recipe changed; run `cargo run -p murmel-cli -- help shots {}`",
                p.name,
                p.name
            );
        }
        assert_eq!(kept.len(), n, "recipes.toml names a page without a recipe");
    }

    /// Invariant: a recipe must end in export.image without a path (the
    /// picture goes where the page says).
    #[test]
    fn a_recipe_ends_in_its_picture() {
        assert!(calls(r#"[{"tool": "export.image"}]"#).is_ok());
        assert!(calls(r#"[{"tool": "view.fit"}]"#).is_err());
        assert!(calls(r#"[{"tool": "export.image", "input": {"path": "x.png"}}]"#).is_err());
        assert!(calls("{}").is_err());
    }
}
