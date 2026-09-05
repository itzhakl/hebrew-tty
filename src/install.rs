#![forbid(unsafe_code)]

//! `--install` makes every interactive shell exec into the proxy from its rc
//! file, so nothing has to be launched through it. The block goes first in
//! the file: the outer shell leaves for the proxy before it pays for the rest
//! of its startup, and the inner shell pays once. The proxy marks its child
//! with `HEBREW_TTY`, which is what stops the inner shell from doing it again.

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const BEGIN: &str = "# >>> hebrew-tty >>>";
pub const END: &str = "# <<< hebrew-tty <<<";

pub fn block(binary: &Path) -> String {
    let quoted = shell_quote(binary);
    format!(
        "{BEGIN}\n\
         if [ -z \"${{HEBREW_TTY:-}}\" ] && [ -t 0 ] && [ -t 1 ] && [ -x {quoted} ]; then\n\
         \x20 case $- in *i*) exec {quoted} ;; esac\n\
         fi\n\
         {END}\n"
    )
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

pub fn with_block(text: &str, block: &str) -> String {
    let rest = without_block(text);
    if rest.is_empty() {
        block.to_owned()
    } else {
        format!("{block}\n{rest}")
    }
}

pub fn without_block(text: &str) -> String {
    let Some(start) = text.find(BEGIN) else {
        return text.to_owned();
    };
    let Some(end_offset) = text[start..].find(END) else {
        return text.to_owned();
    };
    let mut end = start + end_offset + END.len();
    for _ in 0..2 {
        if text[end..].starts_with('\n') {
            end += 1;
        }
    }
    let mut result = String::with_capacity(text.len());
    result.push_str(&text[..start]);
    result.push_str(&text[end..]);
    result
}

pub fn rc_file(
    shell: Option<&OsStr>,
    home: Option<&OsStr>,
    zdotdir: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let shell_name = shell
        .and_then(|shell| Path::new(shell).file_name())
        .and_then(OsStr::to_str)
        .unwrap_or("");
    let home = home
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    match shell_name {
        "zsh" => Ok(zdotdir
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or(home)
            .join(".zshrc")),
        "bash" => Ok(home.join(".bashrc")),
        other => Err(format!(
            "no rc file is known for shell {other:?}; supported: zsh, bash"
        )),
    }
}

/// The launcher on PATH keeps working across rebuilds and packaging, so it is
/// preferred over the executable that happens to be running now.
pub fn binary_path() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("hebrew-tty"))
                .find(|candidate| is_executable(candidate))
        })
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("hebrew-tty"))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn rc_from_environment() -> Result<PathBuf, String> {
    rc_file(
        std::env::var_os("SHELL").as_deref(),
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("ZDOTDIR").as_deref(),
    )
}

fn read_or_empty(path: &Path) -> Result<String, Box<dyn Error>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn install() -> Result<String, Box<dyn Error>> {
    let rc = rc_from_environment()?;
    let block = block(&binary_path());
    let current = read_or_empty(&rc)?;
    let updated = with_block(&current, &block);
    if updated != current {
        fs::write(&rc, updated)?;
    }
    Ok(format!(
        "every interactive shell now starts under hebrew-tty from {}; open a new terminal",
        rc.display()
    ))
}

pub fn uninstall() -> Result<String, Box<dyn Error>> {
    let rc = rc_from_environment()?;
    let current = read_or_empty(&rc)?;
    let updated = without_block(&current);
    if updated == current {
        return Ok(format!("nothing to remove from {}", rc.display()));
    }
    fs::write(&rc, updated)?;
    Ok(format!("removed from {}", rc.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_block() -> String {
        block(Path::new("/home/me/.local/bin/hebrew-tty"))
    }

    #[test]
    fn the_block_guards_on_interactivity_the_tty_and_the_marker_variable() {
        let block = sample_block();
        assert!(block.starts_with(BEGIN));
        assert!(block.ends_with(&format!("{END}\n")));
        assert!(block.contains("[ -z \"${HEBREW_TTY:-}\" ]"));
        assert!(block.contains("[ -t 0 ] && [ -t 1 ]"));
        assert!(block.contains("case $- in *i*) exec '/home/me/.local/bin/hebrew-tty' ;; esac"));
    }

    #[test]
    fn a_quote_in_the_path_survives_the_shell() {
        let block = block(Path::new("/home/o'neil/bin/hebrew-tty"));
        assert!(block.contains("exec '/home/o'\\''neil/bin/hebrew-tty'"));
    }

    #[test]
    fn install_goes_first_and_is_idempotent() {
        let block = sample_block();
        let once = with_block("export PATH=~/bin:$PATH\n", &block);
        assert_eq!(once, format!("{block}\nexport PATH=~/bin:$PATH\n"));
        assert_eq!(with_block(&once, &block), once);
        assert_eq!(with_block("", &block), block);
    }

    #[test]
    fn reinstall_replaces_a_block_that_points_elsewhere() {
        let old = block(Path::new("/old/hebrew-tty"));
        let new = sample_block();
        let text = with_block("alias ll='ls -l'\n", &old);
        let updated = with_block(&text, &new);
        assert_eq!(updated, format!("{new}\nalias ll='ls -l'\n"));
        assert_eq!(updated.matches(BEGIN).count(), 1);
    }

    #[test]
    fn uninstall_restores_the_file_wherever_the_block_sits() {
        let block = sample_block();
        assert_eq!(without_block(&with_block("a\nb\n", &block)), "a\nb\n");
        assert_eq!(without_block(&format!("a\n{block}b\n")), "a\nb\n");
        assert_eq!(without_block(&block), "");
        assert_eq!(without_block("untouched\n"), "untouched\n");
    }

    #[test]
    fn the_rc_file_follows_the_shell_and_zdotdir() {
        let home = Some(OsStr::new("/home/me"));
        assert_eq!(
            rc_file(Some(OsStr::new("/bin/zsh")), home, None).unwrap(),
            PathBuf::from("/home/me/.zshrc")
        );
        assert_eq!(
            rc_file(Some(OsStr::new("/usr/bin/zsh")), home, Some(OsStr::new("/home/me/.zsh"))).unwrap(),
            PathBuf::from("/home/me/.zsh/.zshrc")
        );
        assert_eq!(
            rc_file(Some(OsStr::new("/bin/bash")), home, None).unwrap(),
            PathBuf::from("/home/me/.bashrc")
        );
        assert!(rc_file(Some(OsStr::new("/usr/bin/fish")), home, None)
            .unwrap_err()
            .contains("fish"));
        assert_eq!(
            rc_file(Some(OsStr::new("/bin/zsh")), None, None).unwrap_err(),
            "HOME is not set"
        );
    }
}
