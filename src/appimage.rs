// SPDX-License-Identifier: GPL-3.0-or-later
//! Read an AppImage's own metadata (name, version, icon, update information)
//! straight from its embedded SquashFS image, in-process. Nothing is executed
//! unless the user turned off "safe metadata", and then only for formats the
//! built-in reader cannot open (DwarFS).
//!
//! Metadata always comes from the file on disk. A menu entry written earlier
//! can be stale when the AppImage was replaced or updated itself (Gear Lever
//! issue #497); callers compare both and report the difference.
use crate::{desktop::DesktopFile, elf, paths::Paths};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufReader, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Identity of a file's current contents without hashing it. Any rewrite,
/// replacement or self-update changes at least one field. The device number
/// is left out: btrfs assigns it per mount, so it can change across reboots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Fingerprint {
    pub size: u64,
    pub modified_ns: i64,
    pub inode: u64,
}
impl Fingerprint {
    pub fn of(path: &Path) -> Result<Self, String> {
        let m = fs::metadata(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        Ok(Self {
            size: m.len(),
            modified_ns: m.mtime() * 1_000_000_000 + m.mtime_nsec(),
            inode: m.ino(),
        })
    }
    pub fn key(&self) -> String {
        format!("{:x}-{:x}-{:x}", self.size, self.modified_ns, self.inode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VersionSource {
    /// X-AppImage-Version in the AppImage's own desktop file.
    DesktopEntry,
    /// The newest <release> in the AppImage's AppStream metadata.
    AppStream,
}

/// An updater built into the app itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelfUpdater {
    /// electron-updater (resources/app-update.yml). Its feed can often be read.
    ElectronUpdater {
        provider: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Metadata {
    pub path: PathBuf,
    pub fingerprint: Fingerprint,
    /// 1 (ISO 9660), 2 (SquashFS or DwarFS) or 0 when the type marker is missing.
    pub appimage_type: u8,
    pub arch: String,
    /// squashfs, dwarfs or unknown.
    pub payload: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub version_source: Option<VersionSource>,
    pub comment: Option<String>,
    pub terminal: bool,
    /// The AppImage's own desktop file, verbatim.
    pub desktop_entry: Option<String>,
    /// Arguments of the embedded Exec line (without the program).
    pub default_arguments: Vec<String>,
    /// Icon extracted to the metadata cache (png or svg).
    pub icon: Option<PathBuf>,
    /// Embedded update information (.upd_info), e.g. gh-releases-zsync|…
    pub update_info: Option<String>,
    pub self_updater: Option<SelfUpdater>,
    /// Why some metadata could not be read; the rest is still valid.
    pub error: Option<String>,
    /// How the metadata was read: built-in, appimage-extract or none.
    pub reader: String,
}

/// Is this a file this app can manage? Checks the ELF and AppImage markers,
/// not the name: renamed files work, and *.AppImage scripts are refused.
pub fn is_appimage(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
        && elf::read(path).is_ok_and(|e| e.appimage_type > 0)
}

/// One file in an image listing.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub executable: bool,
}

/// Files inside the image, behind a common interface for every reader
/// (SquashFS, DwarFS, or a folder extracted by running the AppImage).
pub trait Tree {
    fn root_names(&self) -> Vec<String> {
        self.list_dir("/")
    }
    /// Read a regular file, following symlinks inside the image. `path` is
    /// absolute within the image.
    fn read(&self, path: &str, limit: usize) -> Option<Vec<u8>>;
    fn list_dir(&self, dir: &str) -> Vec<String>;
    /// Every regular file with its size and whether it is executable.
    fn files(&self) -> Vec<Entry>;
}

pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

fn join_link(from: &str, target: &str) -> String {
    if target.starts_with('/') {
        normalize(target)
    } else {
        let parent = Path::new(from).parent().unwrap_or(Path::new("/"));
        normalize(&format!("{}/{target}", parent.display()))
    }
}

struct Squash<'a> {
    fs: backhand::FilesystemReader<'a>,
}

impl Tree for Squash<'_> {
    fn list_dir(&self, dir: &str) -> Vec<String> {
        let dir = normalize(dir);
        self.fs
            .files()
            .filter_map(|n| {
                let full = n.fullpath.to_string_lossy();
                let parent = Path::new(full.as_ref())
                    .parent()?
                    .to_string_lossy()
                    .to_string();
                (normalize(&parent) == dir && full != "/")
                    .then(|| {
                        n.fullpath
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                    })
                    .flatten()
            })
            .collect()
    }
    fn read(&self, path: &str, limit: usize) -> Option<Vec<u8>> {
        let mut path = normalize(path);
        for _ in 0..8 {
            let node = self
                .fs
                .files()
                .find(|n| normalize(&n.fullpath.to_string_lossy()) == path)?;
            match &node.inner {
                backhand::InnerNode::File(file) => {
                    if file.file_len() > limit {
                        return None;
                    }
                    let mut out = Vec::with_capacity(file.file_len());
                    self.fs.file(file).reader().read_to_end(&mut out).ok()?;
                    return Some(out);
                }
                backhand::InnerNode::Symlink(link) => {
                    path = join_link(&path, &link.link.to_string_lossy())
                }
                _ => return None,
            }
        }
        None
    }
    fn files(&self) -> Vec<Entry> {
        self.fs
            .files()
            .filter_map(|n| match &n.inner {
                backhand::InnerNode::File(f) => Some(Entry {
                    path: normalize(&n.fullpath.to_string_lossy()),
                    size: f.file_len() as u64,
                    executable: n.header.permissions & 0o111 != 0,
                }),
                _ => None,
            })
            .collect()
    }
}

type DwarfsReader = dwarfs::positioned_io::Slice<dwarfs::positioned_io::RandomAccessFile>;
struct Dwarf {
    index: dwarfs::ArchiveIndex,
    archive: std::cell::RefCell<dwarfs::Archive<DwarfsReader>>,
}

impl Dwarf {
    fn open(path: &Path, offset: u64) -> Result<Self, String> {
        let file =
            dwarfs::positioned_io::RandomAccessFile::open(path).map_err(|e| e.to_string())?;
        let slice =
            dwarfs::positioned_io::Slice::new_to_end(file, offset).map_err(|e| e.to_string())?;
        let (index, archive) = dwarfs::Archive::new(slice).map_err(|e| e.to_string())?;
        Ok(Self {
            index,
            archive: std::cell::RefCell::new(archive),
        })
    }
    fn inode(&self, path: &str) -> Option<dwarfs::Inode<'_>> {
        self.index
            .get_path(normalize(path).split('/').filter(|p| !p.is_empty()))
    }
}

impl Tree for Dwarf {
    fn list_dir(&self, dir: &str) -> Vec<String> {
        self.inode(dir)
            .and_then(|i| i.as_dir())
            .map(|d| d.entries().map(|e| e.name().to_string()).collect())
            .unwrap_or_default()
    }
    fn read(&self, path: &str, limit: usize) -> Option<Vec<u8>> {
        use dwarfs::AsChunks;
        let mut path = normalize(path);
        for _ in 0..8 {
            match self.inode(&path)?.classify() {
                dwarfs::InodeKind::File(f) => {
                    if f.as_chunks().total_size() > limit as u64 {
                        return None;
                    }
                    return f.read_to_vec(&mut self.archive.borrow_mut()).ok();
                }
                dwarfs::InodeKind::Symlink(l) => path = join_link(&path, l.target()),
                _ => return None,
            }
        }
        None
    }
    fn files(&self) -> Vec<Entry> {
        use dwarfs::AsChunks;
        let mut out = Vec::new();
        let mut stack = vec![("/".to_string(), self.index.root())];
        while let Some((prefix, dir)) = stack.pop() {
            for e in dir.entries() {
                let path = format!("{}/{}", prefix.trim_end_matches('/'), e.name());
                match e.inode().classify() {
                    dwarfs::InodeKind::Directory(d) => stack.push((path, d)),
                    dwarfs::InodeKind::File(f) => out.push(Entry {
                        size: f.as_chunks().total_size(),
                        executable: e.inode().metadata().file_type_mode().permission_bits() & 0o111
                            != 0,
                        path,
                    }),
                    _ => {}
                }
            }
        }
        out
    }
}

struct Dir(PathBuf);
impl Tree for Dir {
    fn list_dir(&self, dir: &str) -> Vec<String> {
        fs::read_dir(self.0.join(normalize(dir).trim_start_matches('/')))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }
    fn read(&self, path: &str, limit: usize) -> Option<Vec<u8>> {
        let mut rel = normalize(path);
        for _ in 0..8 {
            let full = self.0.join(rel.trim_start_matches('/'));
            let meta = fs::symlink_metadata(&full).ok()?;
            if meta.file_type().is_symlink() {
                rel = join_link(&rel, &fs::read_link(&full).ok()?.to_string_lossy());
                continue;
            }
            if !meta.is_file() || meta.len() as usize > limit {
                return None;
            }
            return fs::read(full).ok();
        }
        None
    }
    fn files(&self) -> Vec<Entry> {
        use std::os::unix::fs::PermissionsExt;
        let mut out = Vec::new();
        let mut stack = vec![self.0.clone()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).into_iter().flatten().flatten() {
                let Ok(m) = fs::symlink_metadata(e.path()) else {
                    continue;
                };
                if m.is_dir() {
                    stack.push(e.path());
                } else if m.is_file() {
                    out.push(Entry {
                        path: format!(
                            "/{}",
                            e.path()
                                .strip_prefix(&self.0)
                                .unwrap_or(&e.path())
                                .display()
                        ),
                        size: m.len(),
                        executable: m.permissions().mode() & 0o111 != 0,
                    });
                }
            }
        }
        out
    }
}

/// Open the filesystem image embedded in an AppImage with the built-in
/// readers (SquashFS or DwarFS). Nothing is executed.
pub fn open_tree(path: &Path) -> Result<Box<dyn Tree>, String> {
    let elf = elf::read(path)?;
    match elf.payload(path) {
        elf::Payload::SquashFs => {
            let file = fs::File::open(path).map_err(|e| e.to_string())?;
            backhand::FilesystemReader::from_reader_with_offset(
                BufReader::new(file),
                elf.payload_offset,
            )
            .map(|fs| Box::new(Squash { fs }) as Box<dyn Tree>)
            .map_err(|e| format!("Cannot read the embedded SquashFS image: {e}"))
        }
        elf::Payload::DwarFs => Dwarf::open(path, elf.payload_offset)
            .map(|d| Box::new(d) as Box<dyn Tree>)
            .map_err(|e| format!("Cannot read the embedded DwarFS image: {e}")),
        elf::Payload::Unknown if elf.appimage_type == 1 => {
            Err("Type 1 (ISO 9660) AppImages are not read by the built-in reader".into())
        }
        elf::Payload::Unknown => Err("The embedded filesystem image was not recognized".into()),
    }
}

/// The AppImage's own desktop file at the image root: the first name ending
/// in .desktop that really is a desktop entry. A program may itself be named
/// like one (OpenCode's binary is `ai.opencode.desktop`).
pub fn find_desktop(tree: &dyn Tree) -> Option<(String, String)> {
    let mut names: Vec<String> = tree
        .root_names()
        .into_iter()
        .filter(|n| n.ends_with(".desktop"))
        .collect();
    names.sort();
    names.into_iter().find_map(|n| {
        let bytes = tree.read(&format!("/{n}"), 256 << 10)?;
        let text = String::from_utf8(bytes).ok()?;
        text.lines()
            .any(|l| l.trim() == "[Desktop Entry]")
            .then_some((n, text))
    })
}

pub fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_lowercase();
    (head.contains("<svg") || (head.starts_with("<?xml") && head.contains("svg"))).then_some("svg")
}

fn find_icon(tree: &dyn Tree, icon: Option<&str>) -> Option<(Vec<u8>, &'static str)> {
    const LIMIT: usize = 8 << 20;
    let mut candidates = vec!["/.DirIcon".to_string()];
    if let Some(icon) = icon {
        let icon = icon
            .trim_end_matches(".png")
            .trim_end_matches(".svg")
            .trim_start_matches('/');
        candidates.extend([format!("/{icon}.svg"), format!("/{icon}.png")]);
        // usr/share in classic AppDirs, share/ in sharun (Anylinux) AppDirs.
        for base in ["/usr/share/icons/hicolor", "/share/icons/hicolor"] {
            candidates.push(format!("{base}/scalable/apps/{icon}.svg"));
            for size in ["512x512", "256x256", "128x128", "96x96", "64x64", "48x48"] {
                candidates.push(format!("{base}/{size}/apps/{icon}.png"));
            }
        }
        candidates.push(format!("/{icon}"));
    }
    for c in candidates {
        if let Some(bytes) = tree.read(&c, LIMIT)
            && let Some(kind) = image_kind(&bytes)
        {
            return Some((bytes, kind));
        }
    }
    None
}

/// The newest release version in AppStream XML: the first <release version=…>.
pub fn appstream_version(xml: &str) -> Option<String> {
    let start = xml.find("<release ")?;
    let tag = &xml[start..start + xml[start..].find('>')?];
    let at = tag.find("version=")? + "version=".len();
    let quote = tag[at..].chars().next()?;
    let rest = &tag[at + 1..];
    let value = &rest[..rest.find(quote)?];
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

/// Top-level `key: value` pairs of a simple YAML document (electron-updater
/// writes flat files). Quotes are removed; nested lists are ignored.
pub fn flat_yaml(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('-') && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

pub fn electron_updater(yaml: &str) -> Option<SelfUpdater> {
    let pairs = flat_yaml(yaml);
    let get = |k: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    Some(SelfUpdater::ElectronUpdater {
        provider: get("provider")?,
        owner: get("owner"),
        repo: get("repo"),
        url: get("url"),
        channel: get("channel"),
    })
}

fn from_tree(meta: &mut Metadata, tree: &dyn Tree, cache_dir: Option<&Path>) {
    let desktop = find_desktop(tree).map(|(_, text)| text);
    let entry = desktop.as_deref().map(DesktopFile::parse);
    if let Some(entry) = &entry {
        meta.name = entry.main("Name");
        meta.comment = entry.main("Comment");
        meta.terminal = entry.bool("Terminal");
        // Packagers that never set a version leave placeholders such as
        // "UNKNOWN" (pkgforge appimagetool without VERSION); those are not versions.
        if let Some(v) = entry.main("X-AppImage-Version").filter(|v| {
            !matches!(
                v.trim().to_lowercase().as_str(),
                "unknown" | "none" | "null" | "0" | "version"
            )
        }) {
            meta.version = Some(v);
            meta.version_source = Some(VersionSource::DesktopEntry);
        }
        if let Some(exec) = entry.main("Exec")
            && let Ok(exec) = crate::desktop::Exec::parse(&exec)
        {
            meta.default_arguments = exec.args;
        }
    } else {
        meta.error = Some("The AppImage contains no desktop file".into());
    }
    meta.desktop_entry = desktop;
    if meta.version.is_none() {
        for dir in [
            "/usr/share/metainfo",
            "/usr/share/appdata",
            "/share/metainfo",
            "/share/appdata",
        ] {
            for name in tree.list_dir(dir) {
                if (name.ends_with(".xml"))
                    && let Some(xml) = tree.read(&format!("{dir}/{name}"), 1 << 20)
                    && let Some(v) = appstream_version(&String::from_utf8_lossy(&xml))
                {
                    meta.version = Some(v);
                    meta.version_source = Some(VersionSource::AppStream);
                }
            }
            if meta.version.is_some() {
                break;
            }
        }
    }
    if let Some(yaml) = tree.read("/resources/app-update.yml", 64 << 10) {
        meta.self_updater = electron_updater(&String::from_utf8_lossy(&yaml));
    }
    let icon_name = entry.as_ref().and_then(|e| e.main("Icon"));
    if let (Some((bytes, kind)), Some(dir)) = (find_icon(tree, icon_name.as_deref()), cache_dir) {
        let path = dir.join(format!("icon.{kind}"));
        if fs::create_dir_all(dir).is_ok() && fs::write(&path, bytes).is_ok() {
            meta.icon = Some(path);
        }
    }
}

pub struct Options<'a> {
    /// Where to keep the extracted icon. None reads metadata without an icon.
    pub cache_dir: Option<&'a Path>,
    /// Allow running the AppImage with --appimage-extract when the built-in
    /// reader cannot open it.
    pub allow_extract: bool,
}

pub fn inspect(path: &Path, options: &Options) -> Result<Metadata, String> {
    let fingerprint = Fingerprint::of(path)?;
    let elf = elf::read(path)?;
    if elf.appimage_type == 0 {
        return Err(format!(
            "{} is an executable but not an AppImage (no AppImage type marker)",
            path.display()
        ));
    }
    let payload = elf.payload(path);
    let mut meta = Metadata {
        path: path.to_path_buf(),
        fingerprint,
        appimage_type: elf.appimage_type,
        arch: elf.arch.into(),
        payload: match payload {
            elf::Payload::SquashFs => "squashfs",
            elf::Payload::DwarFs => "dwarfs",
            elf::Payload::Unknown => "unknown",
        }
        .into(),
        name: None,
        version: None,
        version_source: None,
        comment: None,
        terminal: false,
        desktop_entry: None,
        default_arguments: vec![],
        icon: None,
        update_info: elf.section_text(path, ".upd_info"),
        self_updater: None,
        error: None,
        reader: "none".into(),
    };
    match open_tree(path) {
        Ok(tree) => {
            meta.reader = "built-in".into();
            from_tree(&mut meta, tree.as_ref(), options.cache_dir);
            return Ok(meta);
        }
        Err(e) => meta.error = Some(e),
    }
    if options.allow_extract {
        match extract_by_running(path) {
            Ok(dir) => {
                meta.reader = "appimage-extract".into();
                meta.error = None;
                from_tree(
                    &mut meta,
                    &Dir(dir.path().join("squashfs-root")),
                    options.cache_dir,
                );
            }
            Err(e) => {
                meta.error = Some(format!(
                    "{}; running --appimage-extract failed: {e}",
                    meta.error.unwrap_or_default()
                ));
            }
        }
    } else if meta.error.is_some() {
        meta.error = Some(format!(
            "{}. Metadata stays unknown because \"Use safe mechanisms to load metadata\" is on.",
            meta.error.unwrap_or_default()
        ));
    }
    Ok(meta)
}

/// Run a copy of the AppImage with --appimage-extract for a few patterns.
/// This executes the file, so it is only used when the user allowed it.
fn extract_by_running(path: &Path) -> Result<TempDir, String> {
    let dir = TempDir::new()?;
    let copy = dir.path().join("app.AppImage");
    fs::copy(path, &copy).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    for pattern in [
        "*.desktop",
        ".DirIcon",
        "*.png",
        "*.svg",
        "usr/share/icons/hicolor/*",
        "usr/share/metainfo/*",
        "resources/app-update.yml",
    ] {
        let status = std::process::Command::new(&copy)
            .arg("--appimage-extract")
            .arg(pattern)
            .current_dir(dir.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() && pattern == "*.desktop" {
            return Err(format!("exit status {status}"));
        }
    }
    Ok(dir)
}

/// A private temporary folder removed on drop.
pub struct TempDir(PathBuf);
impl TempDir {
    pub fn new() -> Result<Self, String> {
        let base = std::env::temp_dir().join(format!(
            "gearlever-cosmic-{}-{}",
            std::process::id(),
            crate::now_nanos()
        ));
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        Ok(Self(base))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Bump when the readers can read more than before, so metadata cached by an
/// older build (including failed reads) is read again. 2: DwarFS sparse
/// files. 3: placeholder versions such as "UNKNOWN" are ignored. 4: sharun
/// (Anylinux) share/ layout for AppStream data and icons. 5: a program named
/// like a desktop file is skipped.
pub const READER_VERSION: u32 = 5;

/// Metadata for a file, reusing the cached copy while the file is unchanged
/// and was read by this reader version.
pub fn inspect_cached(paths: &Paths, path: &Path, allow_extract: bool) -> Result<Metadata, String> {
    let fingerprint = Fingerprint::of(path)?;
    let dir = paths
        .cache()
        .join(format!("r{READER_VERSION}-{}", fingerprint.key()));
    let record = dir.join("metadata.json");
    if let Ok(bytes) = fs::read(&record)
        && let Ok(meta) = serde_json::from_slice::<Metadata>(&bytes)
        && meta.fingerprint == fingerprint
        && meta.path == path
        && meta.icon.as_ref().is_none_or(|i| i.is_file())
        && !(allow_extract && meta.reader == "none")
    {
        return Ok(meta);
    }
    let meta = inspect(
        path,
        &Options {
            cache_dir: Some(&dir),
            allow_extract,
        },
    )?;
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::write(
            &record,
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        );
    }
    Ok(meta)
}

/// Remove cache entries older than 30 days.
pub fn prune_cache(paths: &Paths) {
    let Ok(entries) = fs::read_dir(paths.cache()) else {
        return;
    };
    let limit = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86400);
    for e in entries.flatten() {
        if e.metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < limit)
        {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hashes {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

pub fn hashes(path: &Path) -> Result<Hashes, String> {
    use md5::Digest;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let (mut a, mut b, mut c) = (md5::Md5::new(), sha1::Sha1::new(), sha2::Sha256::new());
    let mut buf = vec![0; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        a.update(&buf[..n]);
        b.update(&buf[..n]);
        c.update(&buf[..n]);
    }
    Ok(Hashes {
        md5: hex(&a.finalize()),
        sha1: hex(&b.finalize()),
        sha256: hex(&c.finalize()),
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Build a real type-2 AppImage-shaped file: a tiny ELF header followed by
    /// a SquashFS image holding the given files (path, contents) and symlinks.
    pub fn fixture(
        dir: &Path,
        name: &str,
        update_info: &str,
        files: &[(&str, &[u8])],
        links: &[(&str, &str)],
    ) -> PathBuf {
        use backhand::{FilesystemWriter, NodeHeader};
        let mut writer = FilesystemWriter::default();
        let header = NodeHeader::new(0o755, 0, 0, 0);
        let mut dirs = std::collections::BTreeSet::new();
        let all: Vec<&str> = files
            .iter()
            .map(|(p, _)| *p)
            .chain(links.iter().map(|(p, _)| *p))
            .collect();
        for path in all {
            let mut acc = PathBuf::from("/");
            let parent = Path::new(path).parent().unwrap();
            for c in parent.components().skip(1) {
                acc.push(c);
                if dirs.insert(acc.clone()) {
                    writer.push_dir(&acc, header).unwrap();
                }
            }
        }
        for (path, data) in files {
            writer
                .push_file(std::io::Cursor::new(data.to_vec()), path, header)
                .unwrap();
        }
        for (path, target) in links {
            writer.push_symlink(*target, path, header).unwrap();
        }
        let mut image = std::io::Cursor::new(Vec::new());
        writer.write(&mut image).unwrap();
        let bytes = crate::elf::tests::tiny_elf(update_info, &image.into_inner());
        let out = dir.join(name);
        fs::write(&out, bytes).unwrap();
        out
    }

    pub const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000IHDR";

    #[test]
    fn reads_name_version_icon_and_updaters_from_the_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(
            dir.path(),
            "Demo.AppImage",
            "zsync|https://example.com/Demo.AppImage.zsync",
            &[
                (
                    "/demo.desktop",
                    b"[Desktop Entry]\nName=Demo App\nExec=demo --flag %U\nIcon=demo\nX-AppImage-Version=2.1.0\nTerminal=false\n",
                ),
                ("/demo.png", PNG),
                ("/resources/app-update.yml", b"owner: acme\nrepo: demo\nprovider: github\n"),
            ],
            &[("/.DirIcon", "demo.png")],
        );
        let cache = dir.path().join("cache");
        let meta = inspect(
            &path,
            &Options {
                cache_dir: Some(&cache),
                allow_extract: false,
            },
        )
        .unwrap();
        assert_eq!(meta.reader, "built-in");
        assert_eq!(meta.name.as_deref(), Some("Demo App"));
        assert_eq!(meta.version.as_deref(), Some("2.1.0"));
        assert_eq!(meta.default_arguments, ["--flag", "%U"]);
        assert_eq!(
            meta.update_info.as_deref(),
            Some("zsync|https://example.com/Demo.AppImage.zsync")
        );
        assert_eq!(fs::read(meta.icon.unwrap()).unwrap(), PNG);
        assert!(
            matches!(meta.self_updater, Some(SelfUpdater::ElectronUpdater { ref provider, ref owner, .. }) if provider == "github" && owner.as_deref() == Some("acme"))
        );
        assert!(is_appimage(&path));
    }
    #[test]
    fn falls_back_to_appstream_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(
            dir.path(),
            "b.AppImage",
            "",
            &[
                ("/b.desktop", b"[Desktop Entry]\nName=B\nExec=b\n"),
                ("/usr/share/metainfo/b.appdata.xml", b"<component><releases><release version='3.4' date='x'/><release version=\"3.3\"/></releases></component>"),
            ],
            &[],
        );
        let meta = inspect(
            &path,
            &Options {
                cache_dir: None,
                allow_extract: false,
            },
        )
        .unwrap();
        assert_eq!(meta.version.as_deref(), Some("3.4"));
        assert_eq!(meta.version_source, Some(VersionSource::AppStream));
    }
    #[test]
    fn a_program_named_like_a_desktop_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(
            dir.path(),
            "oc.AppImage",
            "",
            &[
                (
                    "/ai.example.desktop",
                    b"\x7fELF binary that only looks like a desktop file",
                ),
                (
                    "/ai.example.desktop.desktop",
                    b"[Desktop Entry]\nName=Example\nExec=AppRun %U\nX-AppImage-Version=1.19.5\n",
                ),
            ],
            &[],
        );
        let meta = inspect(
            &path,
            &Options {
                cache_dir: None,
                allow_extract: false,
            },
        )
        .unwrap();
        assert_eq!(meta.name.as_deref(), Some("Example"));
        assert_eq!(meta.version.as_deref(), Some("1.19.5"));
        assert!(meta.error.is_none());
    }

    #[test]
    fn anylinux_layout_and_placeholder_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(
            dir.path(),
            "any.AppImage",
            "",
            &[
                (
                    "/any.desktop",
                    b"[Desktop Entry]\nName=Any\nExec=any\nIcon=any\nX-AppImage-Version=UNKNOWN\n",
                ),
                (
                    "/share/metainfo/any.metainfo.xml",
                    b"<component><releases><release version=\"1.2.3\"/></releases></component>",
                ),
                (
                    "/share/icons/hicolor/scalable/apps/any.svg",
                    b"<svg xmlns='http://www.w3.org/2000/svg'/>",
                ),
            ],
            &[],
        );
        let cache = dir.path().join("cache");
        let meta = inspect(
            &path,
            &Options {
                cache_dir: Some(&cache),
                allow_extract: false,
            },
        )
        .unwrap();
        assert_eq!(meta.version.as_deref(), Some("1.2.3"));
        assert_eq!(meta.version_source, Some(VersionSource::AppStream));
        assert!(meta.icon.unwrap().to_string_lossy().ends_with("icon.svg"));
    }

    #[test]
    fn cached_metadata_follows_the_file_not_a_stale_record() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::fixture(dir.path());
        let make = |v: &str| {
            let text = format!("[Desktop Entry]\nName=C\nExec=c\nX-AppImage-Version={v}\n");
            fixture(
                dir.path(),
                "c.AppImage",
                "",
                &[("/c.desktop", text.as_bytes())],
                &[],
            )
        };
        let path = make("1.0");
        assert_eq!(
            inspect_cached(&paths, &path, false)
                .unwrap()
                .version
                .as_deref(),
            Some("1.0")
        );
        let _ = fs::remove_file(&path);
        let path = make("1.1");
        assert_eq!(
            inspect_cached(&paths, &path, false)
                .unwrap()
                .version
                .as_deref(),
            Some("1.1")
        );
    }
    #[test]
    fn non_appimages_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.AppImage");
        fs::write(&path, b"#!/bin/sh\necho hi\n").unwrap();
        assert!(!is_appimage(&path));
        assert!(
            inspect(
                &path,
                &Options {
                    cache_dir: None,
                    allow_extract: false
                }
            )
            .is_err()
        );
    }
}
