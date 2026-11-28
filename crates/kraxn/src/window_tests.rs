//! The window, driven headless through egui_kittest: the wizard from the
//! first page to a finished install from a folder of downloads, the
//! manage page, and the accessibility tree a screen reader sees.

use std::path::Path;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use kraxn_core::Scope;

use crate::tests::{kofl_release, runner, source_runner, tmp};
use crate::window::{KraxnApp, Page};

fn harness(home: &Path) -> Harness<'static, KraxnApp> {
    harness_with(runner(home))
}

fn harness_with(r: crate::runner::Runner) -> Harness<'static, KraxnApp> {
    let mut slot = Some(r);
    let mut h = Harness::builder()
        .with_size(egui::vec2(980.0, 700.0))
        .build_eframe(move |cc| KraxnApp::new(cc, slot.take().unwrap()).unwrap());
    h.run_steps(2);
    h
}

/// Steps the window until `done` holds (the jobs run on threads).
fn wait(h: &mut Harness<'_, KraxnApp>, done: impl Fn(&KraxnApp) -> bool) {
    for _ in 0..500 {
        h.step();
        if done(h.state()) {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the window did not get there");
}

fn click(h: &mut Harness<'_, KraxnApp>, name: &str) {
    h.get_by_label(name).click();
    h.run_steps(2);
}

fn shows(h: &Harness<'_, KraxnApp>, part: &str) -> bool {
    h.root().children_recursive().any(|n| {
        let a = n.accesskit_node();
        [a.label(), a.value()]
            .into_iter()
            .flatten()
            .any(|t| t.contains(part))
    })
}

/// Every control has a name, and no name holds an icon glyph (a screen
/// reader would read an unknown character).
fn problems(h: &Harness<'_, KraxnApp>) -> Vec<String> {
    let mut out = h.unnamed_widgets();
    for n in h.root().children_recursive() {
        let a = n.accesskit_node();
        if let Some(l) = a.label()
            && l.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c))
            && a.role() != egui::accesskit::Role::Label
        {
            out.push(format!("{:?} {l:?} holds an icon glyph", a.role()));
        }
    }
    out
}

/// Invariant: the wizard installs from a folder of downloads the way the
/// text mode does: Install → for you → a folder → KoflCraft only (the
/// others have no file there) → PATH → the steps → Install, ending with
/// the SHA-256 and start checks shown and the program in place; every
/// page's controls keep their names. Breaks if a page stops filling the
/// plan, Next stays disabled, the run is not started on its thread, or a
/// control loses its name.
#[cfg(unix)]
#[test]
fn wizard_installs_from_a_folder() {
    let d = tmp("window");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    kofl_release(&d.join("dl"), "0.3.1");
    let mut h = harness(&home);
    assert_eq!(problems(&h), Vec::<String>::new(), "welcome");
    click(&mut h, "Install or update programs");
    assert_eq!(h.state().page, Page::Scope);
    assert_eq!(problems(&h), Vec::<String>::new(), "scope");
    click(&mut h, "Next");
    click(&mut h, "A folder of downloaded release files (no internet)");
    h.state_mut().offline_dir = d.join("dl").display().to_string();
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "source");
    click(&mut h, "Next");
    wait(&mut h, |a| {
        a.page == Page::Products && !a.products_loading() && a.plan().installs.len() == 1
    });
    assert_eq!(h.state().plan().installs[0].product, "koflcraft");
    assert!(shows(&h, "(no release for this computer)"));
    assert_eq!(problems(&h), Vec::<String>::new(), "products");
    click(&mut h, "Next");
    assert_eq!(h.state().page, Page::Options);
    click(&mut h, "Next");
    wait(&mut h, |a| a.page == Page::Review && !a.review_busy());
    assert!(shows(&h, "Install KoflCraft 0.3.1 into"), "review");
    assert_eq!(problems(&h), Vec::<String>::new(), "review");
    click(&mut h, "Install");
    wait(&mut h, |a| a.page == Page::Running && a.run_finished());
    assert!(shows(&h, "SHA-256 checked against SHA256SUMS.txt"));
    assert!(shows(&h, "started and ended without an error"));
    assert!(home.join(".local/opt/KoflCraft/0.3.1/bin/kofl").is_file());
    assert_eq!(problems(&h), Vec::<String>::new(), "result");
    let _ = std::fs::remove_dir_all(&d);
}

/// Invariant: online, a program without a release file for this
/// computer can be chosen as a source build: the steps page lists the
/// tools, Install builds it with cargo (the output under "Build output")
/// and the result says it was built here. Breaks if such a program
/// stays disabled, the plan loses `from = "source"`, or the build's
/// output or result is not shown.
#[cfg(unix)]
#[test]
fn wizard_builds_from_source() {
    let d = tmp("window-source");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut h = harness_with(source_runner(&d, &home));
    click(&mut h, "Install or update programs");
    click(&mut h, "Next");
    click(&mut h, "Next");
    wait(&mut h, |a| {
        a.page == Page::Products && !a.products_loading()
    });
    assert!(shows(&h, "Kraxn can build 0.3.1 from the source"));
    click(&mut h, "KoflCraft");
    assert_eq!(
        h.state().plan().installs[0].from,
        kraxn_core::Origin::Source
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "products");
    click(&mut h, "Next");
    assert!(shows(&h, "Tune source builds"));
    click(&mut h, "Next");
    wait(&mut h, |a| a.page == Page::Review && !a.review_busy());
    assert!(shows(&h, "Tools for the build:"), "review");
    assert!(shows(&h, "Build KoflCraft 0.3.1 with cargo"), "review");
    assert_eq!(problems(&h), Vec::<String>::new(), "review");
    click(&mut h, "Install");
    wait(&mut h, |a| a.page == Page::Running && a.run_finished());
    assert!(shows(&h, "built here from the source"), "result");
    assert!(shows(&h, "Build output"), "result");
    assert!(home.join(".local/opt/KoflCraft/0.3.1/bin/kofl").is_file());
    let _ = std::fs::remove_dir_all(&d);
}

/// Invariant: the manage page lists what is installed for the chosen
/// scope and removes a product through `version.remove`. Breaks if its
/// buttons call nothing or the list does not refresh after the change.
#[cfg(unix)]
#[test]
fn manage_page_removes() {
    let d = tmp("manage");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    kofl_release(&d.join("dl"), "0.3.1");
    let mut r = runner(&home);
    let plan = kraxn_core::Plan {
        offline_dir: Some(d.join("dl")),
        installs: vec![kraxn_core::Want::new("koflcraft")],
        ..kraxn_core::Plan::default()
    };
    assert!(
        r.quiet("plan.apply", &serde_json::json!({"plan": plan}))
            .ok()
    );
    let mut h = harness(&home);
    click(&mut h, "Manage what is installed");
    assert_eq!(h.state().scope, Scope::User);
    assert!(shows(&h, "KoflCraft 0.3.1 (default)"));
    assert_eq!(problems(&h), Vec::<String>::new(), "manage");
    click(&mut h, "All versions of KoflCraft");
    wait(&mut h, |_| !home.join(".local/opt/KoflCraft").exists());
    wait(&mut h, |a| !a.manage_busy());
    assert!(shows(&h, "Nothing."));
    let _ = std::fs::remove_dir_all(&d);
}

/// Pictures of the pages for a review (not a check): run with
/// `KRAXN_SHOTS=folder cargo test -p kraxn shots -- --nocapture`; needs a
/// GPU or Mesa lavapipe. Without the variable it does nothing.
#[cfg(unix)]
#[test]
fn shots() {
    let Some(out) = std::env::var_os("KRAXN_SHOTS").map(std::path::PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let d = tmp("shots");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    kofl_release(&d.join("dl"), "0.3.1");
    kofl_release(&d.join("dl/older"), "0.3.0");
    let mut h = harness(&home);
    let snap = |h: &mut Harness<'_, KraxnApp>, name: &str| {
        h.run_steps(3);
        let img = h.render().unwrap();
        img.save(out.join(format!("{name}.png"))).unwrap();
    };
    snap(&mut h, "1-welcome");
    click(&mut h, "Install or update programs");
    snap(&mut h, "2-for-whom");
    click(&mut h, "Next");
    click(&mut h, "A folder of downloaded release files (no internet)");
    h.state_mut().offline_dir = d.join("dl").display().to_string();
    snap(&mut h, "3-where-from");
    click(&mut h, "Next");
    wait(&mut h, |a| {
        a.page == Page::Products && !a.products_loading() && a.plan().installs.len() == 1
    });
    snap(&mut h, "4-programs");
    click(&mut h, "Next");
    snap(&mut h, "5-options");
    click(&mut h, "Next");
    wait(&mut h, |a| !a.review_busy());
    snap(&mut h, "6-steps");
    click(&mut h, "Install");
    wait(&mut h, |a| a.run_finished());
    snap(&mut h, "7-done");
    h.ctx.set_theme(egui::Theme::Light);
    snap(&mut h, "8-done-light");
    h.ctx.set_theme(egui::Theme::Dark);
    h.state_mut().page = Page::Welcome;
    h.run_steps(2);
    click(&mut h, "Manage what is installed");
    wait(&mut h, |a| !a.manage_busy());
    snap(&mut h, "9-manage");
    let _ = std::fs::remove_dir_all(&d);
}
