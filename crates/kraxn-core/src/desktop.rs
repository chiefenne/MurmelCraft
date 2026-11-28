//! Menu entries and file types for a product with a window (MurmelCraft):
//! a desktop file, its icon and the file types on Linux; an app in
//! Applications on macOS; a Start menu shortcut and the file types in
//! the registry on Windows. The family's own format (`.murmel`) opens in
//! the window by default; the shared ones (`.vtu`, `.vtkhdf`) only offer
//! it under "Open with", so another program that owns them keeps them.

use std::path::PathBuf;

use crate::actions::{Action, Step};
use crate::catalogue::Product;
use crate::layout::{Env, Layout, Scope};
use crate::platform::Os;

/// The icon of the window program (Linux, 256 px; macOS, 1024 px).
const ICON_256: &[u8] = include_bytes!("../../../assets/logo/murmelcraft-256.png");
const ICON_1024: &[u8] = include_bytes!("../../../assets/logo/murmelcraft-macos-1024.png");

/// An `.icns` file that holds one 1024 px PNG (`ic10`); Finder and the
/// Dock scale it down.
pub fn icns(png: &[u8]) -> Vec<u8> {
    let entry = 8 + png.len() as u32;
    let total = 8 + entry;
    let mut v = Vec::with_capacity(total as usize);
    v.extend_from_slice(b"icns");
    v.extend_from_slice(&total.to_be_bytes());
    v.extend_from_slice(b"ic10");
    v.extend_from_slice(&entry.to_be_bytes());
    v.extend_from_slice(png);
    v
}

/// Where the menu entry and its files go, per system and scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// Linux: `share` (`~/.local/share`, `/usr/local/share`).
    pub share: Option<PathBuf>,
    /// macOS: the app bundle.
    pub app: Option<PathBuf>,
    /// Windows: the shortcut.
    pub shortcut: Option<PathBuf>,
}

pub fn places(layout: &Layout, env: &Env, p: &Product) -> Places {
    let user = layout.scope == Scope::User;
    let mut out = Places {
        share: None,
        app: None,
        shortcut: None,
    };
    match layout.platform.os {
        Os::Linux => {
            out.share = if user {
                env.home.as_ref().map(|h| h.join(".local/share"))
            } else {
                Some("/usr/local/share".into())
            }
        }
        Os::Macos => {
            let apps = if user {
                env.home.as_ref().map(|h| h.join("Applications"))
            } else {
                Some("/Applications".into())
            };
            out.app = apps.map(|a| a.join(format!("{}.app", p.title)));
        }
        Os::Windows => {
            let base = if user {
                env.app_data.clone()
            } else {
                env.program_data.clone()
            };
            out.shortcut = base.map(|b| {
                b.join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs")
                    .join(format!("{}.lnk", p.title))
            });
        }
    }
    out
}

/// The desktop file (Linux).
pub fn desktop_file(p: &Product, exe: &std::path::Path, file_types: bool) -> String {
    let mut s = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec=\"{}\" %f\nIcon={}\nTerminal=false\nCategories=Science;Engineering;Viewer;\n",
        p.title,
        p.summary,
        exe.display(),
        p.app_id
    );
    if file_types && !p.file_types.is_empty() {
        let m: Vec<&str> = p.file_types.iter().map(|f| f.mime.as_str()).collect();
        s.push_str(&format!("MimeType={};\n", m.join(";")));
    }
    s.push_str("# written by Kraxn\n");
    s
}

/// The shared-mime-info file that names the file types (Linux).
pub fn mime_xml(p: &Product) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- written by Kraxn -->\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for f in &p.file_types {
        s.push_str(&format!(
            "  <mime-type type=\"{}\">\n    <comment>{}</comment>\n    <glob pattern=\"*.{}\"/>\n  </mime-type>\n",
            f.mime, f.name, f.ext
        ));
    }
    s.push_str("</mime-info>\n");
    s
}

/// The app bundle's Info.plist (macOS).
pub fn info_plist(p: &Product, exe_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{t}</string>
  <key>CFBundleDisplayName</key><string>{t}</string>
  <key>CFBundleIdentifier</key><string>{id}</string>
  <key>CFBundleExecutable</key><string>{exe_name}</string>
  <key>CFBundleIconFile</key><string>{t}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>KraxnWritten</key><true/>
</dict>
</plist>
"#,
        t = p.title,
        id = p.app_id
    )
}

fn run(program: &str, args: &[String], optional: bool) -> Action {
    Action::Run {
        program: program.into(),
        args: args.to_vec(),
        env: Vec::new(),
        optional,
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// The steps that make the menu entry (and, with `file_types`, the file
/// types) of `p`, starting `exe`. Empty for a product without a window.
/// On macOS and Linux `exe` is the program under `current`, so a new
/// default version needs no new entry; on Windows it names one version,
/// and a new default writes the entry again.
pub fn menu_steps(
    layout: &Layout,
    env: &Env,
    p: &Product,
    exe: &std::path::Path,
    file_types: bool,
) -> Vec<Step> {
    let Some(window) = &p.window else {
        return Vec::new();
    };
    let admin = layout.scope == Scope::System;
    let user = !admin;
    let at = places(layout, env, p);
    let mut a = Vec::new();
    match layout.platform.os {
        Os::Linux => {
            let Some(share) = at.share else {
                return Vec::new();
            };
            let apps = share.join("applications");
            let desktop = apps.join(format!("{}.desktop", p.app_id));
            a.push(Action::Write {
                path: desktop.clone(),
                text: desktop_file(p, exe, file_types),
                executable: false,
            });
            a.push(Action::WriteBytes {
                path: share
                    .join("icons/hicolor/256x256/apps")
                    .join(format!("{}.png", p.app_id)),
                bytes: ICON_256.to_vec(),
            });
            if file_types && !p.file_types.is_empty() {
                let mime = share.join("mime");
                a.push(Action::Write {
                    path: mime.join("packages").join(format!("{}.xml", p.app_id)),
                    text: mime_xml(p),
                    executable: false,
                });
                a.push(run(
                    "update-mime-database",
                    &[mime.display().to_string()],
                    true,
                ));
                if user {
                    // xdg-mime writes the user's mimeapps.list below HOME
                    let home: Vec<(String, String)> = env
                        .home
                        .iter()
                        .map(|h| ("HOME".to_string(), h.display().to_string()))
                        .collect();
                    for f in p.file_types.iter().filter(|f| f.own) {
                        a.push(Action::Run {
                            program: "xdg-mime".into(),
                            args: vec![
                                "default".into(),
                                format!("{}.desktop", p.app_id),
                                f.mime.clone(),
                            ],
                            env: home.clone(),
                            optional: true,
                        });
                    }
                }
            }
            a.push(run(
                "update-desktop-database",
                &[apps.display().to_string()],
                true,
            ));
        }
        Os::Macos => {
            let Some(app) = at.app else {
                return Vec::new();
            };
            let c = app.join("Contents");
            a.push(Action::RemoveDir { path: app.clone() });
            a.push(Action::Write {
                path: c.join("Info.plist"),
                text: info_plist(p, window),
                executable: false,
            });
            a.push(Action::WriteBytes {
                path: c.join("Resources").join(format!("{}.icns", p.title)),
                bytes: icns(ICON_1024),
            });
            a.push(Action::MakeDir {
                path: c.join("MacOS"),
            });
            a.push(Action::Link {
                target: exe.to_path_buf(),
                link: c.join("MacOS").join(window),
            });
            if admin {
                a.push(Action::Readable { path: app.clone() });
            }
            // so that Finder and Spotlight see it at once
            a.push(run(
                "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
                &["-f".into(), app.display().to_string()],
                true,
            ));
        }
        Os::Windows => {
            let Some(lnk) = at.shortcut else {
                return Vec::new();
            };
            a.push(Action::Run {
                program: "powershell".into(),
                args: strings(&[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:KRAXN_LNK); $s.TargetPath = $env:KRAXN_EXE; $s.Description = $env:KRAXN_TEXT; $s.Save()",
                ]),
                env: vec![
                    ("KRAXN_LNK".into(), lnk.display().to_string()),
                    ("KRAXN_EXE".into(), exe.display().to_string()),
                    ("KRAXN_TEXT".into(), p.summary.clone()),
                ],
                optional: false,
            });
            if file_types {
                let root = if user {
                    r"HKCU\Software\Classes"
                } else {
                    r"HKLM\Software\Classes"
                };
                let exe_s = exe.display().to_string();
                for f in &p.file_types {
                    let prog = format!("{}.{}", p.title, f.ext);
                    let key = |k: &str| format!(r"{root}\{k}");
                    a.push(run(
                        "reg",
                        &strings(&["add", &key(&prog), "/ve", "/d", &f.name, "/f"]),
                        false,
                    ));
                    a.push(run(
                        "reg",
                        &strings(&[
                            "add",
                            &key(&format!(r"{prog}\DefaultIcon")),
                            "/ve",
                            "/d",
                            &format!("{exe_s},0"),
                            "/f",
                        ]),
                        false,
                    ));
                    a.push(run(
                        "reg",
                        &strings(&[
                            "add",
                            &key(&format!(r"{prog}\shell\open\command")),
                            "/ve",
                            "/d",
                            &format!("\"{exe_s}\" \"%1\""),
                            "/f",
                        ]),
                        false,
                    ));
                    a.push(run(
                        "reg",
                        &strings(&[
                            "add",
                            &key(&format!(r".{}\OpenWithProgids", f.ext)),
                            "/v",
                            &prog,
                            "/t",
                            "REG_NONE",
                            "/f",
                        ]),
                        false,
                    ));
                    if f.own {
                        a.push(run(
                            "reg",
                            &strings(&[
                                "add",
                                &key(&format!(".{}", f.ext)),
                                "/ve",
                                "/d",
                                &prog,
                                "/f",
                            ]),
                            false,
                        ));
                    }
                }
            }
        }
    }
    let what = if file_types && !p.file_types.is_empty() && layout.platform.os != Os::Macos {
        "a menu entry and the file types"
    } else {
        "a menu entry"
    };
    vec![Step {
        title: format!("Give {} {what}", p.title),
        admin,
        actions: a,
    }]
}

/// The steps that take the menu entry and the file types away again.
pub fn remove_steps(layout: &Layout, env: &Env, p: &Product) -> Vec<Step> {
    if p.window.is_none() {
        return Vec::new();
    }
    let admin = layout.scope == Scope::System;
    let at = places(layout, env, p);
    let mut a = Vec::new();
    match layout.platform.os {
        Os::Linux => {
            let Some(share) = at.share else {
                return Vec::new();
            };
            let desktop = share
                .join("applications")
                .join(format!("{}.desktop", p.app_id));
            if !desktop.exists() {
                return Vec::new();
            }
            a.push(Action::RemoveFile { path: desktop });
            a.push(Action::RemoveFile {
                path: share
                    .join("icons/hicolor/256x256/apps")
                    .join(format!("{}.png", p.app_id)),
            });
            let xml = share
                .join("mime/packages")
                .join(format!("{}.xml", p.app_id));
            if xml.exists() {
                a.push(Action::RemoveFile { path: xml });
                a.push(run(
                    "update-mime-database",
                    &[share.join("mime").display().to_string()],
                    true,
                ));
            }
            a.push(run(
                "update-desktop-database",
                &[share.join("applications").display().to_string()],
                true,
            ));
        }
        Os::Macos => {
            let Some(app) = at.app.filter(|a| a.join("Contents/Info.plist").exists()) else {
                return Vec::new();
            };
            let ours = std::fs::read_to_string(app.join("Contents/Info.plist"))
                .is_ok_and(|t| t.contains("KraxnWritten"));
            if !ours {
                return Vec::new();
            }
            a.push(Action::RemoveDir { path: app });
        }
        Os::Windows => {
            let Some(lnk) = at.shortcut.filter(|l| l.exists()) else {
                return Vec::new();
            };
            a.push(Action::RemoveFile { path: lnk });
            let root = if admin {
                r"HKLM\Software\Classes"
            } else {
                r"HKCU\Software\Classes"
            };
            for f in &p.file_types {
                let prog = format!("{}.{}", p.title, f.ext);
                a.push(run(
                    "reg",
                    &strings(&["delete", &format!(r"{root}\{prog}"), "/f"]),
                    true,
                ));
                a.push(run(
                    "reg",
                    &strings(&[
                        "delete",
                        &format!(r"{root}\.{}\OpenWithProgids", f.ext),
                        "/v",
                        &prog,
                        "/f",
                    ]),
                    true,
                ));
            }
        }
    }
    vec![Step {
        title: format!("Remove the menu entry of {}", p.title),
        admin,
        actions: a,
    }]
}

/// Whether `p` has a menu entry from Kraxn in this scope.
pub fn has_entry(layout: &Layout, env: &Env, p: &Product) -> bool {
    !remove_steps(layout, env, p).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;
    use crate::platform::Platform;

    /// Invariant: the icns file is a valid container around the PNG (the
    /// header's length is the file's), and on Linux the desktop file
    /// starts the program under `current` with the file, lists the three
    /// file types, makes only `.murmel` MurmelCraft's default, and writes
    /// into the user's `~/.local/share` without administrator rights.
    /// Breaks if Finder got a broken icon, a menu entry pointed at one
    /// version, or Kraxn took `.vtu` away from another program.
    #[test]
    fn menu_entry_and_file_types() {
        let png = b"\x89PNG fake";
        let i = icns(png);
        assert_eq!(&i[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(i[4..8].try_into().unwrap()) as usize,
            i.len()
        );

        let c = Catalogue::builtin().unwrap();
        let m = c.get("murmelcraft").unwrap();
        let env = Env {
            home: Some("/home/ann".into()),
            ..Env::default()
        };
        let linux = Platform::parse("linux", "x86_64").unwrap();
        let l = Layout::new(Scope::User, linux, &env, None).unwrap();
        let exe = l.current_link(m).join("bin/murmelcraft");
        let steps = menu_steps(&l, &env, m, &exe, true);
        assert_eq!(steps.len(), 1);
        assert!(!steps[0].admin);
        let d = desktop_file(m, &exe, true);
        assert!(
            d.contains("Exec=\"/home/ann/.local/opt/MurmelCraft/current/bin/murmelcraft\" %f"),
            "{d}"
        );
        assert!(d.contains("MimeType=application/x-murmelcraft-session;application/x-vtk-unstructured-grid;application/x-vtkhdf;"), "{d}");
        let defaults: Vec<String> = steps[0]
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::Run { program, args, .. } if program.ends_with("xdg-mime") => {
                    Some(args[2].clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(defaults, ["application/x-murmelcraft-session"]);
        assert!(mime_xml(m).contains("<glob pattern=\"*.vtkhdf\"/>"));
        // no window, no entry
        let k = c.get("koflcraft").unwrap();
        assert!(menu_steps(&l, &env, k, &exe, true).is_empty());
        // Windows: the .vtu key only gains an "Open with" entry
        let win = Platform::parse("windows", "x86_64").unwrap();
        let wenv = Env {
            local_app_data: Some(r"C:\Users\ann\AppData\Local".into()),
            app_data: Some(r"C:\Users\ann\AppData\Roaming".into()),
            ..Env::default()
        };
        let wl = Layout::new(Scope::User, win, &wenv, None).unwrap();
        let ws = menu_steps(
            &wl,
            &wenv,
            m,
            std::path::Path::new(r"C:\x\murmelcraft.exe"),
            true,
        );
        let lines: Vec<String> = ws[0].actions.iter().map(crate::actions::describe).collect();
        assert!(
            lines.iter().any(|l| l.contains(r".vtu\OpenWithProgids")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains(r"Classes\.vtu /ve")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains(r"Classes\.murmel /ve")),
            "{lines:?}"
        );
    }
}
