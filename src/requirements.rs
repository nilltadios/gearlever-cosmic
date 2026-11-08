// SPDX-License-Identifier: GPL-3.0-or-later
//! Will this AppImage start on this system? Checks what its runtime needs
//! (FUSE 2, or a static runtime that only needs fusermount and /dev/fuse) and
//! which shared libraries its main program needs that it neither bundles nor
//! finds in the system's library folders.
//!
//! Inside the Flatpak the host's /usr is read through /run/host (the
//! host-os:ro permission). Results name what was looked for and where.
#[cfg(test)]
use crate::appimage;
use crate::{desktop::Exec, elf};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// The original AppImageKit runtime: loads libfuse.so.2 (FUSE 2).
    Fuse2,
    /// A static runtime with FUSE 3 built in (AppImage type2-runtime): needs fusermount3 or fusermount.
    StaticFuse3,
    /// uruntime (static; mounts with fusermount, or extracts when FUSE is missing).
    Uruntime,
    /// Type 1 or an unrecognized runtime.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Everything that was checked is present.
    Ok,
    /// Something needed is missing; the app will likely not start.
    Missing,
    /// Could not tell (for example an unreadable image).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Requirements {
    pub verdict: Verdict,
    pub runtime: Runtime,
    /// Plain-language problems, each with what to do.
    pub problems: Vec<String>,
    pub fuse2_library: Option<PathBuf>,
    pub fusermount: Option<PathBuf>,
    pub fusermount3: Option<PathBuf>,
    pub dev_fuse: bool,
    /// The program inside the image whose libraries were checked.
    pub program: Option<String>,
    pub needed_libraries: Vec<String>,
    /// Needed, not bundled, and not found in the system library folders.
    pub missing_libraries: Vec<String>,
    /// System folders that were searched.
    pub searched: Vec<PathBuf>,
    /// What could not be checked.
    pub notes: Vec<String>,
}

/// Where the host's files are visible from this process.
pub struct System {
    pub root: PathBuf,
}
impl System {
    /// The host system. GEARLEVER_COSMIC_SYSTEM_ROOT points the check at
    /// another tree (used by verification/check-requirements.sh).
    pub fn host() -> Self {
        if let Some(root) =
            std::env::var_os("GEARLEVER_COSMIC_SYSTEM_ROOT").filter(|v| !v.is_empty())
        {
            return Self { root: root.into() };
        }
        Self {
            root: if crate::host::sandboxed() {
                PathBuf::from("/run/host")
            } else {
                PathBuf::from("/")
            },
        }
    }
    fn join(&self, p: &str) -> PathBuf {
        self.root.join(p.trim_start_matches('/'))
    }
    fn lib_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<String> = [
            "/usr/lib",
            "/usr/lib64",
            "/lib",
            "/lib64",
            "/usr/local/lib",
            "/usr/lib/x86_64-linux-gnu",
            "/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/lib/aarch64-linux-gnu",
        ]
        .map(String::from)
        .to_vec();
        // ld.so.conf and its includes add distribution-specific folders.
        let mut confs = vec![self.join("/etc/ld.so.conf")];
        if let Ok(rd) = fs::read_dir(self.join("/etc/ld.so.conf.d")) {
            confs.extend(rd.flatten().map(|e| e.path()));
        }
        for c in confs {
            for line in fs::read_to_string(c).unwrap_or_default().lines() {
                let l = line.trim();
                if l.starts_with('/') {
                    dirs.push(l.to_string());
                }
            }
        }
        let mut seen = BTreeSet::new();
        dirs.into_iter()
            .filter(|d| seen.insert(d.clone()))
            .map(|d| self.join(&d))
            .filter(|p| p.is_dir())
            .collect()
    }
    fn find_lib(&self, dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
        dirs.iter().map(|d| d.join(name)).find(|p| p.exists())
    }
    fn find_bin(&self, name: &str) -> Option<PathBuf> {
        ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin"]
            .iter()
            .map(|d| self.join(d).join(name))
            .find(|p| p.is_file())
    }
}

/// DT_NEEDED entries of an ELF image held in memory (via .dynamic/.dynstr).
pub fn needed(bytes: &[u8]) -> Vec<String> {
    let Some(e) = parse_sections(bytes) else {
        return vec![];
    };
    let Some(&(_, dyn_off, dyn_size, entsize)) = e.iter().find(|s| s.0 == ".dynamic") else {
        return vec![];
    };
    let Some(&(_, str_off, str_size, _)) = e.iter().find(|s| s.0 == ".dynstr") else {
        return vec![];
    };
    let wide = bytes[4] == 2;
    let ent = if entsize > 0 {
        entsize
    } else if wide {
        16
    } else {
        8
    };
    let strs = bytes
        .get(str_off as usize..(str_off + str_size) as usize)
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut o = dyn_off as usize;
    while o + ent as usize <= (dyn_off + dyn_size) as usize && o + ent as usize <= bytes.len() {
        let (tag, val) = if wide {
            (
                u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()),
                u64::from_le_bytes(bytes[o + 8..o + 16].try_into().unwrap()),
            )
        } else {
            (
                u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as u64,
                u32::from_le_bytes(bytes[o + 4..o + 8].try_into().unwrap()) as u64,
            )
        };
        if tag == 0 {
            break;
        }
        if tag == 1
            && let Some(s) = strs
                .get(val as usize..)
                .and_then(|s| s.split(|b| *b == 0).next())
        {
            out.push(String::from_utf8_lossy(s).into_owned());
        }
        o += ent as usize;
    }
    out
}

/// (name, offset, size, entsize) for each section of a little-endian ELF.
fn parse_sections(b: &[u8]) -> Option<Vec<(String, u64, u64, u64)>> {
    if b.len() < 64 || &b[..4] != b"\x7fELF" || b[5] != 1 {
        return None;
    }
    let wide = b[4] == 2;
    let u16v = |o: usize| -> Option<u64> {
        Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?).into())
    };
    let u32_ = |o: usize| -> Option<u64> {
        Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?).into())
    };
    let u64_ =
        |o: usize| -> Option<u64> { Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?)) };
    let (shoff, shent, shnum, shstr) = if wide {
        (u64_(0x28)?, u16v(0x3a)?, u16v(0x3c)?, u16v(0x3e)?)
    } else {
        (u32_(0x20)?, u16v(0x2e)?, u16v(0x30)?, u16v(0x32)?)
    };
    let hdr = |i: u64| -> Option<(u64, u64, u64, u64)> {
        let o = (shoff + i * shent) as usize;
        if wide {
            Some((u32_(o)?, u64_(o + 24)?, u64_(o + 32)?, u64_(o + 56)?))
        } else {
            Some((u32_(o)?, u32_(o + 16)?, u32_(o + 20)?, u32_(o + 36)?))
        }
    };
    let (_, so, ss, _) = hdr(shstr)?;
    let names = b.get(so as usize..(so + ss) as usize)?;
    (0..shnum)
        .map(|i| {
            let (n, off, size, ent) = hdr(i)?;
            let name = names.get(n as usize..)?.split(|c| *c == 0).next()?;
            Some((String::from_utf8_lossy(name).into_owned(), off, size, ent))
        })
        .collect()
}

pub fn runtime_kind(runtime: &[u8], sections: &[(String, u64, u64)]) -> Runtime {
    let has = |s: &[u8]| runtime.windows(s.len()).any(|w| w == s);
    if has(b"libfuse.so.2") {
        Runtime::Fuse2
    } else if has(b"uruntime") {
        Runtime::Uruntime
    } else if !sections.iter().any(|(n, ..)| n == ".interp") {
        Runtime::StaticFuse3
    } else {
        Runtime::Unknown
    }
}

/// Libraries the host always provides through the dynamic loader itself.
fn always_present(name: &str) -> bool {
    name.starts_with("ld-linux") || name.starts_with("linux-vdso")
}

pub fn check(path: &Path, system: &System) -> Result<Requirements, String> {
    let elf = elf::read(path)?;
    let mut runtime_bytes = vec![0; elf.payload_offset.min(16 << 20) as usize];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut runtime_bytes))
        .map_err(|e| e.to_string())?;
    let runtime = if elf.appimage_type == 1 {
        Runtime::Unknown
    } else {
        runtime_kind(&runtime_bytes, &elf.sections)
    };
    let dirs = system.lib_dirs();
    let mut r = Requirements {
        verdict: Verdict::Ok,
        runtime: runtime.clone(),
        problems: vec![],
        fuse2_library: system.find_lib(&dirs, "libfuse.so.2"),
        fusermount: system.find_bin("fusermount"),
        fusermount3: system.find_bin("fusermount3"),
        dev_fuse: Path::new("/dev/fuse").exists(),
        program: None,
        needed_libraries: vec![],
        missing_libraries: vec![],
        searched: dirs.clone(),
        notes: vec![],
    };
    if elf.arch != elf::host_arch() {
        r.problems.push(format!(
            "Built for {}, but this computer is {}.",
            elf.arch,
            elf::host_arch()
        ));
    }
    match runtime {
        Runtime::Fuse2 if r.fuse2_library.is_none() => r.problems.push(
            "Needs FUSE 2 (libfuse.so.2), which is not installed; FUSE 3 does not replace it. Install your distribution's fuse2 package (Arch: fuse2, Fedora: fuse-libs, Debian/Ubuntu: libfuse2t64 or libfuse2), or add --appimage-extract-and-run to the app's arguments.".into(),
        ),
        Runtime::Fuse2 if r.fusermount.is_none() && r.fusermount3.is_none() => r.problems.push("Needs fusermount to mount itself; install fuse2 (or fuse3).".into()),
        Runtime::StaticFuse3 if r.fusermount3.is_none() && r.fusermount.is_none() => r.problems.push(
            "Its runtime needs fusermount3 (or fusermount), which is not installed. Install fuse3, or add --appimage-extract-and-run to the app's arguments.".into(),
        ),
        Runtime::Uruntime if r.fusermount3.is_none() && r.fusermount.is_none() => r.notes.push("No fusermount found: uruntime will extract the app on each start instead of mounting it (slower).".into()),
        _ => {}
    }
    if !r.dev_fuse && runtime != Runtime::Uruntime {
        r.notes.push("/dev/fuse is not visible here, so mounting may fail (inside the Flatpak this is expected and does not affect the host).".into());
    }
    if r.fuse2_library.is_none()
        && r.fusermount.is_none()
        && r.fusermount3.is_none()
        && dirs.is_empty()
    {
        r.verdict = Verdict::Unknown;
        r.notes.push(
            "The system's library folders are not visible from here; nothing could be checked."
                .into(),
        );
        return Ok(r);
    }
    // The main program's own libraries.
    if elf.payload(path) != elf::Payload::Unknown {
        match program_libraries(path) {
            Ok(Some((program, needed, bundled))) => {
                r.program = Some(program);
                r.missing_libraries = needed
                    .iter()
                    .filter(|n| !always_present(n) && !bundled.contains(*n) && system.find_lib(&dirs, n).is_none())
                    .cloned()
                    .collect();
                r.needed_libraries = needed;
                if !r.missing_libraries.is_empty() {
                    r.problems.push(format!(
                        "Needs libraries that are neither inside the AppImage nor installed: {}. Install the packages that provide them.",
                        r.missing_libraries.join(", ")
                    ));
                }
            }
            Ok(None) => r.notes.push("The main program is a script or could not be found, so its libraries were not checked.".into()),
            Err(e) => r.notes.push(format!("Libraries were not checked: {e}")),
        }
    } else {
        r.notes.push(
            "Libraries were not checked: the image format is not readable by the built-in reader."
                .into(),
        );
    }
    if !r.problems.is_empty() {
        r.verdict = Verdict::Missing;
    }
    Ok(r)
}

/// (program path in the image, its DT_NEEDED list, bundled library names).
type ProgramLibraries = (String, Vec<String>, BTreeSet<String>);

/// The program the AppImage starts (Exec of its desktop file, else AppRun,
/// else its executable ELF programs), its DT_NEEDED list, and every shared
/// library file name bundled inside. Works for SquashFS and DwarFS images.
fn program_libraries(path: &Path) -> Result<Option<ProgramLibraries>, String> {
    let tree = crate::appimage::open_tree(path)?;
    let files = tree.files();
    let bundled: BTreeSet<String> = files
        .iter()
        .filter_map(|e| {
            Path::new(&e.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .filter(|n| n.contains(".so"))
        .collect();
    let found = crate::appimage::find_desktop(tree.as_ref());
    let desktop = found.as_ref().map(|(n, _)| n.clone());
    let desktop_stem = desktop
        .as_deref()
        .and_then(|d| Path::new(d).file_stem())
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut candidates = Vec::new();
    if let Some(d) = desktop.and_then(|d| tree.read(&format!("/{d}"), 256 << 10))
        && let Some(exec) =
            crate::desktop::DesktopFile::parse(&String::from_utf8_lossy(&d)).main("Exec")
        && let Ok(exec) = Exec::parse(&exec)
    {
        let prog = exec.program.trim_start_matches("./").to_string();
        if prog.starts_with('/') {
            candidates.push(prog);
        } else {
            candidates.extend([
                format!("/{prog}"),
                format!("/usr/bin/{prog}"),
                format!("/bin/{prog}"),
            ]);
        }
    }
    candidates.push("/AppRun".into());
    // AppRun is often a script: fall back to the image's executable programs,
    // preferring one named like the desktop file, then the largest.
    let mut programs: Vec<(bool, u64, String)> = files
        .iter()
        .filter(|e| e.executable && e.size >= 4096)
        .filter(|e| {
            let name = Path::new(&e.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            !name.contains(".so") && !name.starts_with("chrome") && !name.contains("crashpad")
        })
        .map(|e| {
            let name = Path::new(&e.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            (name == desktop_stem, e.size, e.path.clone())
        })
        .collect();
    programs.sort();
    candidates.extend(programs.into_iter().rev().map(|(_, _, p)| p).take(12));
    for c in candidates {
        if let Some(bytes) = tree.read(&c, 512 << 20)
            && bytes.starts_with(b"\x7fELF")
        {
            return Ok(Some((c, needed(&bytes), bundled)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_fuse2_runtime_without_libfuse2_is_reported_with_a_fix() {
        let dir = tempfile::tempdir().unwrap();
        let path = appimage::tests::fixture(
            dir.path(),
            "a.AppImage",
            "libfuse.so.2",
            &[("/a.desktop", b"[Desktop Entry]\nName=A\nExec=a\n")],
            &[],
        );
        let root = dir.path().join("sys");
        fs::create_dir_all(root.join("usr/lib")).unwrap();
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        fs::write(root.join("usr/lib/libfuse3.so.4"), b"").unwrap();
        fs::write(root.join("usr/bin/fusermount3"), b"").unwrap();
        let r = check(&path, &System { root: root.clone() }).unwrap();
        assert_eq!(r.runtime, Runtime::Fuse2);
        assert_eq!(r.verdict, Verdict::Missing);
        assert!(r.problems[0].contains("FUSE 2"), "{:?}", r.problems);
        fs::write(root.join("usr/lib/libfuse.so.2"), b"").unwrap();
        let r = check(&path, &System { root }).unwrap();
        assert_eq!(r.verdict, Verdict::Ok, "{:?}", r.problems);
    }
    #[test]
    fn a_static_runtime_needs_only_fusermount() {
        let dir = tempfile::tempdir().unwrap();
        let path = appimage::tests::fixture(
            dir.path(),
            "b.AppImage",
            "",
            &[("/b.desktop", b"[Desktop Entry]\nName=B\nExec=b\n")],
            &[],
        );
        let root = dir.path().join("sys");
        fs::create_dir_all(root.join("usr/lib")).unwrap();
        let r = check(&path, &System { root: root.clone() }).unwrap();
        assert_eq!(r.runtime, Runtime::StaticFuse3);
        assert_eq!(r.verdict, Verdict::Missing);
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        fs::write(root.join("usr/bin/fusermount3"), b"").unwrap();
        assert_eq!(check(&path, &System { root }).unwrap().verdict, Verdict::Ok);
    }
}
