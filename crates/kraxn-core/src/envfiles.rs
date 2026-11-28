//! The environment files of a version on macOS and Linux, the same as
//! `tools/install-shared.sh` writes them: `env.sh` (bash, zsh), `env.csh`
//! (csh, tcsh), a module file (Environment Modules, Lmod) and `use.sh`,
//! which defines `<name>-use` to switch versions in a shell.

use std::path::Path;

use crate::catalogue::Product;

/// What else a version folder holds that the files put on the PATH.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    /// `<version>/venv`, Python for the example scripts.
    pub venv: bool,
    /// `<version>/src`, the source tree.
    pub src: bool,
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

pub fn env_sh(p: &Product, version: &str, prefix: &Path, dest: &Path, x: Extras) -> String {
    let (title, home, prefix, dest) = (&p.title, p.home_var(), s(prefix), s(dest));
    let venv = if x.venv {
        format!("${home}/venv/bin:")
    } else {
        String::new()
    };
    let src = if x.src {
        format!("export {}_SRC=\"${home}/src\"\n", title.to_uppercase())
    } else {
        String::new()
    };
    format!(
        r#"# {title} {version} for bash and zsh: source {dest}/env.sh
# (or {prefix}/current/env.sh for the version the 'current' link points to)
# Takes every other {title} version under {prefix} off PATH first, so that
# sourcing another version's env.sh switches cleanly.
# (A loop over PATH's parts, not a glob: it works the same in sh, bash
# and zsh, and leaves folders outside {prefix} alone.)
_fc_rest="$PATH:"; _fc_new=""
while [ -n "$_fc_rest" ]; do
  _fc_d="${{_fc_rest%%:*}}"; _fc_rest="${{_fc_rest#*:}}"
  case "$_fc_d" in "{prefix}"/*) ;; *) _fc_new="${{_fc_new:+$_fc_new:}}$_fc_d" ;; esac
done
PATH="$_fc_new"; unset _fc_rest _fc_new _fc_d
export {home}="{dest}"
export PATH="${home}/bin:{venv}$PATH"
{src}"#
    )
}

pub fn env_csh(p: &Product, version: &str, dest: &Path, x: Extras) -> String {
    let (title, home, dest) = (&p.title, p.home_var(), s(dest));
    let venv = if x.venv {
        format!("${{{home}}}/venv/bin:")
    } else {
        String::new()
    };
    let src = if x.src {
        format!("setenv {}_SRC \"${{{home}}}/src\"\n", title.to_uppercase())
    } else {
        String::new()
    };
    format!(
        r#"# {title} {version} for csh and tcsh: source {dest}/env.csh
setenv {home} "{dest}"
setenv PATH "${{{home}}}/bin:{venv}${{PATH}}"
{src}"#
    )
}

pub fn modulefile(p: &Product, version: &str, prefix: &Path, dest: &Path, x: Extras) -> String {
    let (title, name, home, prefix, dest) =
        (&p.title, &p.archive, p.home_var(), s(prefix), s(dest));
    let venv = if x.venv {
        "prepend-path PATH $root/venv/bin\n"
    } else {
        ""
    };
    let src = if x.src {
        format!("setenv {}_SRC $root/src\n", title.to_uppercase())
    } else {
        String::new()
    };
    format!(
        r#"#%Module1.0
## {title} {version} (Environment Modules and Lmod)
## module use {prefix}/modulefiles ; module load {name}/{version}
proc ModulesHelp {{ }} {{ puts stderr "{title} {version} in {dest}" }}
module-whatis "{title} {version}"
conflict {name}
set root {dest}
setenv {home} $root
prepend-path PATH $root/bin
{venv}{src}"#
    )
}

/// The default version of the module files.
pub fn module_version(version: &str) -> String {
    format!("#%Module1.0\nset ModulesVersion \"{version}\"\n")
}

pub fn use_sh(p: &Product, prefix: &Path) -> String {
    let (title, name, home, prefix) = (&p.title, &p.archive, p.home_var(), s(prefix));
    let f = format!("{name}-use");
    format!(
        r#"# {title} version switch for bash and zsh. Source it once (for everybody:
# ln -s {prefix}/use.sh /etc/profile.d/{name}.sh); it switches on the default
# version unless one is on already, and defines:
#   {f}            list the installed versions
#   {f} 1.1.0      switch this shell to 1.1.0
#   {f} default    switch to the default ({prefix}/current)
{f}() {{
  _fc_root="{prefix}"
  if [ $# -eq 0 ] || [ "$1" = list ]; then
    _fc_cur="$(readlink "$_fc_root/current" 2>/dev/null)"
    for _fc_d in "$_fc_root"/*/env.sh; do
      _fc_v="$(basename "$(dirname "$_fc_d")")"
      [ "$_fc_v" = current ] && continue
      _fc_m=""
      [ "$_fc_v" = "$_fc_cur" ] && _fc_m=" (default)"
      [ "$_fc_root/$_fc_v" = "${{{home}:-}}" ] && _fc_m="$_fc_m (active)"
      echo "$_fc_v$_fc_m"
    done
    unset _fc_root _fc_cur _fc_d _fc_v _fc_m
    return 0
  fi
  _fc_v="${{1#v}}"; [ "$_fc_v" = default ] && _fc_v=current
  if [ ! -f "$_fc_root/$_fc_v/env.sh" ]; then
    echo "{f}: $1 is not installed; installed:" >&2; {f} list >&2
    unset _fc_root _fc_v; return 1
  fi
  . "$_fc_root/$_fc_v/env.sh"
  unset _fc_root _fc_v
  echo "{title} $(basename "${home}") (${home})"
}}
if [ -z "${{{home}:-}}" ] && [ -f "{prefix}/current/env.sh" ]; then
  . "{prefix}/current/env.sh"
fi
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    /// Invariant: env.sh sets <TITLE>_HOME and puts the version's bin
    /// first, after taking the other versions off PATH, and the module
    /// file loads the same folder — the files install-shared.sh writes.
    /// Checked by running env.sh in sh, bash and zsh (where installed).
    /// Breaks if a brace of the format strings is wrong (the shell would
    /// fail), the names drift from the script's (its use.sh would not find
    /// Kraxn's versions), or the PATH clean-up went back to a glob, which
    /// in bash eats the folders between two matches and in zsh stops with
    /// "no matches found" when there is no venv.
    #[test]
    fn env_files_match_install_shared() {
        let c = Catalogue::builtin().unwrap();
        let p = c.get("foehncraft").unwrap();
        let prefix = Path::new("/opt/Foehncraft");
        let dest = prefix.join("1.3.0");
        let sh = env_sh(
            p,
            "1.3.0",
            prefix,
            &dest,
            Extras {
                venv: true,
                src: false,
            },
        );
        assert!(
            sh.contains("export FOEHNCRAFT_HOME=\"/opt/Foehncraft/1.3.0\""),
            "{sh}"
        );
        assert!(
            sh.contains("export PATH=\"$FOEHNCRAFT_HOME/bin:$FOEHNCRAFT_HOME/venv/bin:$PATH\""),
            "{sh}"
        );
        let m = modulefile(p, "1.3.0", prefix, &dest, Extras::default());
        assert!(
            m.contains("set root /opt/Foehncraft/1.3.0") && m.contains("conflict foehncraft"),
            "{m}"
        );
        let csh = env_csh(p, "1.3.0", &dest, Extras::default());
        assert!(
            csh.contains("setenv PATH \"${FOEHNCRAFT_HOME}/bin:${PATH}\""),
            "{csh}"
        );
        let u = use_sh(p, prefix);
        assert!(
            u.contains("foehncraft-use() {") && u.contains("${FOEHNCRAFT_HOME:-}"),
            "{u}"
        );
        // in every shell there is: the other version and its venv go, the
        // folders around them (another project's venv too) stay
        #[cfg(unix)]
        for shell in ["/bin/sh", "/bin/bash", "/bin/zsh", "/usr/bin/zsh"] {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            let script = format!("{sh}\necho \"$PATH\"");
            let o = std::process::Command::new(shell)
                .args(["-c", &script])
                .env(
                    "PATH",
                    "/opt/Foehncraft/1.2.0/bin:/usr/bin:/opt/Foehncraft/1.2.0/venv/bin:/home/a/proj/venv/bin",
                )
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&o.stdout).trim(),
                "/opt/Foehncraft/1.3.0/bin:/opt/Foehncraft/1.3.0/venv/bin:/usr/bin:/home/a/proj/venv/bin",
                "{shell}"
            );
        }
        #[cfg(unix)]
        if std::path::Path::new("/bin/bash").exists() {
            let o = std::process::Command::new("/bin/bash")
                .args(["-n", "-c", &u])
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
    }
}
