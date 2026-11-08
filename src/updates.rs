// SPDX-License-Identifier: GPL-3.0-or-later
//! Update sources and checks. Every result says what was compared and why;
//! "no update" is only reported after an actual comparison succeeded. Apps
//! with their own updater are reported as such, with what could be checked.
use crate::appimage::{Metadata, SelfUpdater};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// A user-chosen update source. Field names match Gear Lever's so its
/// `--set-update-source KEY=VALUE` options and saved settings carry over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "manager", deny_unknown_fields)]
pub enum Source {
    /// A fixed download URL; compared by size (or SHA-1 from a .zsync file).
    #[serde(rename = "StaticFileUpdater")]
    StaticUrl { url: String },
    /// GitHub releases. repo is OWNER/REPO, repo_filename a glob such as App-*-x86_64.AppImage.
    #[serde(rename = "GithubUpdater")]
    Github {
        repo: String,
        repo_filename: String,
        #[serde(default)]
        allow_prereleases: bool,
    },
    /// GitLab releases. repo_url is the project URL, e.g. https://gitlab.com/group/project.
    #[serde(rename = "GitlabUpdater")]
    Gitlab {
        repo_url: String,
        repo_filename: String,
    },
    /// Codeberg releases. repo is OWNER/REPO.
    #[serde(rename = "CodebergUpdater")]
    Codeberg {
        repo: String,
        repo_filename: String,
        #[serde(default)]
        allow_prereleases: bool,
    },
    /// Any Forgejo or Gitea instance. repo_url is https://HOST/OWNER/REPO.
    #[serde(rename = "ForgejoUpdater")]
    Forgejo {
        repo_url: String,
        repo_filename: String,
        #[serde(default)]
        allow_prereleases: bool,
    },
    /// Anonymous FTP. url is ftp://HOST; filename a path pattern such as /pub/app/*/App-*.AppImage.
    #[serde(rename = "FTPUpdater")]
    Ftp { url: String, filename: String },
}

pub struct Manager {
    pub name: &'static str,
    pub label: &'static str,
    pub fields: &'static [(&'static str, &'static str)],
}
pub const MANAGERS: [Manager; 6] = [
    Manager {
        name: "StaticFileUpdater",
        label: "Static URL",
        fields: &[("url", "Download URL (http or https)")],
    },
    Manager {
        name: "GithubUpdater",
        label: "GitHub",
        fields: &[
            ("repo", "OWNER/REPO"),
            ("repo_filename", "Release file name; * matches anything"),
            ("allow_prereleases", "true or false"),
        ],
    },
    Manager {
        name: "GitlabUpdater",
        label: "GitLab",
        fields: &[
            (
                "repo_url",
                "Project URL, e.g. https://gitlab.com/group/project",
            ),
            ("repo_filename", "Release file name; * matches anything"),
        ],
    },
    Manager {
        name: "CodebergUpdater",
        label: "Codeberg",
        fields: &[
            ("repo", "OWNER/REPO"),
            ("repo_filename", "Release file name; * matches anything"),
            ("allow_prereleases", "true or false"),
        ],
    },
    Manager {
        name: "FTPUpdater",
        label: "FTP",
        fields: &[
            ("url", "ftp://HOST"),
            ("filename", "Path pattern, e.g. /pub/app/*/App-*.AppImage"),
        ],
    },
    Manager {
        name: "ForgejoUpdater",
        label: "Forgejo / Gitea",
        fields: &[
            ("repo_url", "https://HOST/OWNER/REPO"),
            ("repo_filename", "Release file name; * matches anything"),
            ("allow_prereleases", "true or false"),
        ],
    },
];

impl Source {
    pub fn manager(&self) -> &'static Manager {
        let name = serde_json::to_value(self).unwrap()["manager"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        MANAGERS
            .iter()
            .find(|m| m.name == name)
            .expect("known manager")
    }
    /// Build from Gear Lever-style KEY=VALUE pairs.
    pub fn from_pairs(manager: &str, pairs: &[(String, String)]) -> Result<Self, String> {
        let m = MANAGERS
            .iter()
            .find(|m| m.name == manager || m.label.eq_ignore_ascii_case(manager))
            .ok_or_else(|| {
                format!(
                    "Unknown update manager {manager}; use one of: {}",
                    MANAGERS.map(|m| m.name).join(", ")
                )
            })?;
        let mut obj = serde_json::Map::new();
        obj.insert("manager".into(), Value::String(m.name.into()));
        for (k, v) in pairs {
            if !m.fields.iter().any(|(f, _)| f == k) {
                return Err(format!(
                    "{} does not take {k}; its fields: {}",
                    m.name,
                    m.fields
                        .iter()
                        .map(|(f, _)| *f)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let value = if k == "allow_prereleases" {
                Value::Bool(match v.to_lowercase().as_str() {
                    "1" | "true" | "yes" => true,
                    "0" | "false" | "no" | "" => false,
                    _ => return Err(format!("{k} must be true or false")),
                })
            } else {
                Value::String(v.trim().to_string())
            };
            obj.insert(k.clone(), value);
        }
        let source: Self = serde_json::from_value(Value::Object(obj))
            .map_err(|e| format!("Missing or invalid fields for {}: {e}", m.name))?;
        source.validate()?;
        Ok(source)
    }
    pub fn validate(&self) -> Result<(), String> {
        let repo_ok = |r: &str| {
            let parts: Vec<_> = r.split('/').collect();
            parts.len() == 2 && parts.iter().all(|p| !p.is_empty())
        };
        let http = |u: &str| u.starts_with("https://") || u.starts_with("http://");
        match self {
            Self::StaticUrl { url } if !http(url) => Err("Enter an http or https URL".into()),
            Self::Github {
                repo,
                repo_filename,
                ..
            }
            | Self::Codeberg {
                repo,
                repo_filename,
                ..
            } => {
                if !repo_ok(repo) {
                    Err("Enter the repository as OWNER/REPO".into())
                } else if repo_filename.trim().is_empty() {
                    Err("Enter the release file name, for example App-*-x86_64.AppImage".into())
                } else {
                    Ok(())
                }
            }
            Self::Gitlab {
                repo_url,
                repo_filename,
            }
            | Self::Forgejo {
                repo_url,
                repo_filename,
                ..
            } => {
                if !repo_url.starts_with("https://") || url::Url::parse(repo_url).is_err() {
                    Err("Enter the project's https:// URL".into())
                } else if repo_filename.trim().is_empty() {
                    Err("Enter the release file name".into())
                } else {
                    Ok(())
                }
            }
            Self::Ftp { url, filename } => {
                if !url.starts_with("ftp://") {
                    Err("Enter an ftp:// URL".into())
                } else if !filename.starts_with('/') {
                    Err("Enter the file path pattern starting with /".into())
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
    pub fn summary(&self) -> String {
        match self {
            Self::StaticUrl { url } => url.clone(),
            Self::Github {
                repo,
                repo_filename,
                ..
            }
            | Self::Codeberg {
                repo,
                repo_filename,
                ..
            } => {
                format!("{repo} · {repo_filename}")
            }
            Self::Gitlab {
                repo_url,
                repo_filename,
            }
            | Self::Forgejo {
                repo_url,
                repo_filename,
                ..
            } => {
                format!("{repo_url} · {repo_filename}")
            }
            Self::Ftp { url, filename } => format!("{url}{filename}"),
        }
    }
}

/// Where the source used for a check came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Set by the user in this app.
    Custom,
    /// Read from Gear Lever's settings for the same file.
    GearLever,
    /// Update information the developer embedded in the AppImage (.upd_info).
    Embedded,
    /// The feed of the app's own built-in updater.
    SelfUpdater,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// A newer file was found and can be installed here.
    Available,
    /// Compared with the source; the installed file is the current one.
    UpToDate,
    /// The app has its own updater. `available` says what could be learned.
    SelfUpdating,
    /// No update source is known. Add one to check.
    NoSource,
    /// The source has no file matching the configured name.
    NotFound,
    /// The check itself failed (network, server, rate limit). Nothing is known.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Download {
    pub url: String,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    pub status: Status,
    /// Plain-language explanation, safe to show as is.
    pub message: String,
    pub origin: Option<Origin>,
    /// Manager label, e.g. GitHub, or "electron-updater".
    pub source: Option<String>,
    pub source_detail: Option<String>,
    pub current_version: Option<String>,
    pub available_version: Option<String>,
    /// How the files were compared: sha1, sha256, sha512, size or none.
    pub compared_by: Option<String>,
    pub download: Option<Download>,
    /// Whether this app can download and install the update itself.
    pub can_install: bool,
    /// For SelfUpdating: whether a newer version is known to exist (null = could not tell).
    pub update_known: Option<bool>,
    /// The app also has its own updater, besides the source used here.
    pub self_updater: Option<String>,
}

impl Check {
    fn new(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            origin: None,
            source: None,
            source_detail: None,
            current_version: None,
            available_version: None,
            compared_by: None,
            download: None,
            can_install: false,
            update_known: None,
            self_updater: None,
        }
    }
}

// ---------- HTTP ----------

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("gearlever-cosmic/", env!("CARGO_PKG_VERSION")))
            .build(),
    )
}

fn get_text(url: &str) -> Result<String, String> {
    let mut req = agent().get(url);
    if url.starts_with("https://api.github.com/")
        && let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.is_empty()
    {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let mut resp = req
        .call()
        .map_err(|e| format!("Cannot reach {}: {e}", host(url)))?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|e| format!("Cannot read the reply from {}: {e}", host(url)))?;
    // .zsync files are a text header followed by binary block checksums.
    let body = String::from_utf8_lossy(&body).into_owned();
    match status {
        200..=299 => Ok(body),
        403 | 429 if body.to_lowercase().contains("rate limit") => Err(format!(
            "{} refused the request: API rate limit exceeded. Try again later, or set GITHUB_TOKEN.",
            host(url)
        )),
        404 => Err(format!("{} answered 404 Not Found for {url}", host(url))),
        _ => Err(format!("{} answered HTTP {status}", host(url))),
    }
}

fn get_json(url: &str) -> Result<Value, String> {
    serde_json::from_str(&get_text(url)?)
        .map_err(|e| format!("Unexpected reply from {}: {e}", host(url)))
}

/// Content length from a HEAD request (falling back to a streamed GET).
fn remote_size(url: &str) -> Result<Option<u64>, String> {
    let parse = |r: &ureq::http::Response<ureq::Body>| {
        r.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
    };
    if let Ok(resp) = agent().head(url).call()
        && resp.status().is_success()
        && let Some(size) = parse(&resp)
    {
        return Ok(Some(size));
    }
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| format!("Cannot reach {}: {e}", host(url)))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{} answered HTTP {}",
            host(url),
            resp.status().as_u16()
        ));
    }
    Ok(parse(&resp))
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.into())
}

// ---------- matching ----------

fn glob_match(pattern: &str, name: &str) -> bool {
    glob::Pattern::new(pattern).is_ok_and(|p| p.matches(name))
}

/// Several assets match: prefer the one for this machine's architecture.
fn pick<T>(items: Vec<&T>, name: impl Fn(&T) -> &str) -> Option<&T> {
    if items.len() <= 1 {
        return items.into_iter().next();
    }
    let arch = crate::elf::host_arch();
    let is_arm = |n: &str| {
        let n = n.to_lowercase();
        ["arm64", "aarch64", "armv7l", "armhf"]
            .iter()
            .any(|a| n.contains(a))
    };
    let is_x86 = |n: &str| {
        let n = n.to_lowercase();
        n.contains("x86_64") || n.contains("amd64") || n.contains("x86-64") || n.contains("x64")
    };
    items
        .iter()
        .find(|i| match arch {
            "x86_64" => is_x86(name(i)) || !is_arm(name(i)),
            "aarch64" => is_arm(name(i)),
            _ => false,
        })
        .copied()
        .or(items.first().copied())
}

/// A file's hash, cached beside its metadata while the file is unchanged.
fn local_hash(path: &Path, alg: &str) -> Result<String, String> {
    let cache = crate::paths::Paths::discover()
        .ok()
        .zip(crate::appimage::Fingerprint::of(path).ok())
        .map(|(p, f)| p.cache().join(f.key()).join(format!("{alg}.txt")));
    if let Some(c) = &cache
        && let Ok(v) = std::fs::read_to_string(c)
        && !v.trim().is_empty()
    {
        return Ok(v.trim().to_string());
    }
    let value = compute_hash(path, alg)?;
    if let Some(c) = &cache
        && c.parent()
            .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
    {
        let _ = std::fs::write(c, &value);
    }
    Ok(value)
}

fn compute_hash(path: &Path, alg: &str) -> Result<String, String> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0; 1 << 20];
    macro_rules! run {
        ($h:expr) => {{
            let mut h = $h;
            loop {
                let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            h.finalize().to_vec()
        }};
    }
    let bytes = match alg {
        "sha1" => run!(sha1::Sha1::new()),
        "sha256" => run!(sha2::Sha256::new()),
        "sha512" => run!(sha2::Sha512::new()),
        _ => return Err(format!("Unknown hash {alg}")),
    };
    Ok(crate::appimage::hex(&bytes))
}

/// Parse a .zsync header for the target file's SHA-1, URL and length.
pub fn zsync_header(text: &str) -> (Option<String>, Option<String>, Option<u64>) {
    let header = text.split("\n\n").next().unwrap_or_default();
    let field = |name: &str| {
        header
            .lines()
            .find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string()))
    };
    (
        field("SHA-1:").map(|s| s.to_lowercase()),
        field("URL:"),
        field("Length:").and_then(|l| l.parse().ok()),
    )
}

fn resolve(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.into();
    }
    url::Url::parse(base)
        .and_then(|b| b.join(relative))
        .map(String::from)
        .unwrap_or_else(|_| relative.into())
}

// ---------- the check ----------

/// Facts about the newest release file a source offers.
struct Found {
    download: Download,
    version: Option<String>,
    /// SHA-1 from a .zsync file, which takes priority.
    zsync_sha1: Option<String>,
}

fn compare(path: &Path, found: &Found, size: u64) -> Result<(bool, &'static str), String> {
    if let Some(sha1) = &found.zsync_sha1 {
        return Ok((&local_hash(path, "sha1")? != sha1, "sha1"));
    }
    if let Some(sha256) = &found.download.sha256 {
        return Ok((&local_hash(path, "sha256")? != sha256, "sha256"));
    }
    if let Some(sha512) = &found.download.sha512 {
        return Ok((&local_hash(path, "sha512")? != sha512, "sha512"));
    }
    match found.download.size {
        Some(remote) => Ok((remote != size, "size")),
        None => Err(
            "The server did not report the file's size or a checksum, so it cannot be compared"
                .into(),
        ),
    }
}

fn github_releases(repo: &str, prereleases: bool) -> Result<Vec<Value>, String> {
    let base = format!("https://api.github.com/repos/{repo}/releases");
    if prereleases {
        Ok(get_json(&format!("{base}?per_page=10"))?
            .as_array()
            .cloned()
            .unwrap_or_default())
    } else {
        Ok(vec![get_json(&format!("{base}/latest"))?])
    }
}

fn asset_download(asset: &Value, url_key: &str) -> Download {
    Download {
        url: asset[url_key].as_str().unwrap_or_default().into(),
        name: asset["name"].as_str().map(str::to_string),
        size: asset["size"].as_u64().filter(|s| *s > 0),
        sha1: None,
        sha256: asset["digest"]
            .as_str()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_lowercase),
        sha512: None,
    }
}

/// GitHub/Codeberg/Forgejo style: releases with an assets array.
fn from_release_list(
    releases: &[Value],
    pattern: &str,
    url_key: &str,
    zsync: bool,
) -> Result<Option<Found>, String> {
    for release in releases {
        if release["draft"] == true {
            continue;
        }
        let assets: Vec<&Value> = release["assets"]
            .as_array()
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let matching: Vec<&Value> = assets
            .iter()
            .copied()
            .filter(|a| glob_match(pattern, a["name"].as_str().unwrap_or_default()))
            .collect();
        let Some(asset) = pick(matching, |a| a["name"].as_str().unwrap_or_default()) else {
            continue;
        };
        let version = release["tag_name"]
            .as_str()
            .or(release["name"].as_str())
            .map(|v| v.to_string());
        let name = asset["name"].as_str().unwrap_or_default();
        if zsync && let Some(target) = name.strip_suffix(".zsync") {
            let Some(file) = assets.iter().find(|a| a["name"] == target) else {
                return Err(format!("The release has {name} but not {target}"));
            };
            let zsync_text = get_text(asset[url_key].as_str().unwrap_or_default())?;
            let (sha1, _, _) = zsync_header(&zsync_text);
            return Ok(Some(Found {
                download: asset_download(file, url_key),
                version,
                zsync_sha1: sha1,
            }));
        }
        return Ok(Some(Found {
            download: asset_download(asset, url_key),
            version,
            zsync_sha1: None,
        }));
    }
    Ok(None)
}

fn split_repo_url(url: &str) -> Result<(String, String), String> {
    let u = url::Url::parse(url).map_err(|e| e.to_string())?;
    let path = u
        .path()
        .trim_matches('/')
        .trim_end_matches(".git")
        .to_string();
    if path.split('/').count() < 2 {
        return Err("The project URL needs OWNER/REPO after the host".into());
    }
    Ok((
        format!("{}://{}", u.scheme(), u.host_str().unwrap_or_default()),
        path,
    ))
}

fn find_from_source(source: &Source) -> Result<Option<Found>, String> {
    match source {
        Source::StaticUrl { url } => Ok(Some(Found {
            download: Download {
                url: url.clone(),
                name: url.rsplit('/').next().map(str::to_string),
                size: remote_size(url)?,
                sha1: None,
                sha256: None,
                sha512: None,
            },
            version: None,
            zsync_sha1: None,
        })),
        Source::Github {
            repo,
            repo_filename,
            allow_prereleases,
        } => from_release_list(
            &github_releases(repo, *allow_prereleases)?,
            repo_filename,
            "browser_download_url",
            false,
        ),
        Source::Codeberg {
            repo,
            repo_filename,
            allow_prereleases,
        } => {
            let mut url =
                format!("https://codeberg.org/api/v1/repos/{repo}/releases?draft=exclude");
            if !allow_prereleases {
                url.push_str("&pre-release=exclude");
            }
            let list = get_json(&url)?.as_array().cloned().unwrap_or_default();
            from_release_list(
                &list[..list.len().min(1)],
                repo_filename,
                "browser_download_url",
                false,
            )
        }
        Source::Forgejo {
            repo_url,
            repo_filename,
            allow_prereleases,
        } => {
            let (base, repo) = split_repo_url(repo_url)?;
            let list = if *allow_prereleases {
                get_json(&format!(
                    "{base}/api/v1/repos/{repo}/releases?draft=exclude"
                ))?
                .as_array()
                .cloned()
                .unwrap_or_default()
            } else {
                vec![get_json(&format!(
                    "{base}/api/v1/repos/{repo}/releases/latest"
                ))?]
            };
            from_release_list(
                &list[..list.len().min(1)],
                repo_filename,
                "browser_download_url",
                false,
            )
        }
        Source::Gitlab {
            repo_url,
            repo_filename,
        } => {
            let (base, repo) = split_repo_url(repo_url)?;
            let id = url::form_urlencoded::byte_serialize(repo.as_bytes()).collect::<String>();
            let list = get_json(&format!("{base}/api/v4/projects/{id}/releases"))?;
            let Some(release) = list.as_array().and_then(|l| l.first()) else {
                return Ok(None);
            };
            let links: Vec<&Value> = release["assets"]["links"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|l| {
                            glob_match(repo_filename, l["name"].as_str().unwrap_or_default())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let Some(link) = pick(links, |l| l["name"].as_str().unwrap_or_default()) else {
                return Ok(None);
            };
            let url = link["direct_asset_url"]
                .as_str()
                .or(link["url"].as_str())
                .unwrap_or_default()
                .to_string();
            Ok(Some(Found {
                download: Download {
                    size: remote_size(&url)?,
                    url,
                    name: link["name"].as_str().map(str::to_string),
                    sha1: None,
                    sha256: None,
                    sha512: None,
                },
                version: release["tag_name"].as_str().map(str::to_string),
                zsync_sha1: None,
            }))
        }
        Source::Ftp { url, filename } => ftp_find(url, filename),
    }
}

fn ftp_connect(url: &str) -> Result<suppaftp::FtpStream, String> {
    let host = url.trim_start_matches("ftp://").trim_end_matches('/');
    let addr = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:21")
    };
    let mut ftp =
        suppaftp::FtpStream::connect(&addr).map_err(|e| format!("Cannot reach {host}: {e}"))?;
    ftp.login("anonymous", "")
        .map_err(|e| format!("{host} refused anonymous login: {e}"))?;
    Ok(ftp)
}

fn ftp_find(url: &str, pattern: &str) -> Result<Option<Found>, String> {
    let mut ftp = ftp_connect(url)?;
    let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
    let first_wild = parts.iter().position(|p| p.contains(['*', '?', '[']));
    let mut candidates = vec![String::new()];
    let start = first_wild.unwrap_or(parts.len().saturating_sub(1));
    let base = format!("/{}", parts[..start].join("/"));
    candidates[0] = base;
    for (i, part) in parts.iter().enumerate().skip(start) {
        let last = i == parts.len() - 1;
        let mut next = Vec::new();
        for dir in &candidates {
            let names = ftp.nlst(Some(dir)).unwrap_or_default();
            let mut names: Vec<String> = names
                .into_iter()
                .map(|n| n.rsplit('/').next().unwrap_or(&n).to_string())
                .filter(|n| glob_match(part, n))
                .collect();
            // Newest-looking names last in natural order; take the highest.
            names.sort_by(|a, b| natural_cmp(b, a));
            for n in names {
                let full = format!("{}/{n}", dir.trim_end_matches('/'));
                if last {
                    if let Ok(size) = ftp.size(&full) {
                        next.push(full);
                        let _ = size;
                        break;
                    }
                } else {
                    next.push(full);
                }
            }
        }
        candidates = next;
        if candidates.is_empty() {
            return Ok(None);
        }
    }
    let Some(file) = candidates.first() else {
        return Ok(None);
    };
    let size = ftp.size(file).ok().map(|s| s as u64);
    let _ = ftp.quit();
    Ok(Some(Found {
        download: Download {
            url: format!("{}{file}", url.trim_end_matches('/')),
            name: file.rsplit('/').next().map(str::to_string),
            size,
            sha1: None,
            sha256: None,
            sha512: None,
        },
        version: None,
        zsync_sha1: None,
    }))
}

/// Compare names so that "app-1.10" sorts after "app-1.9".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek(), b.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        s.push(*c);
                        it.next();
                    }
                    s.trim_start_matches('0').to_string()
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let o = x.len().cmp(&y.len()).then(x.cmp(&y));
                if o.is_ne() {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.cmp(y);
                if o.is_ne() {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Turn embedded `.upd_info` into a lookup, the way AppImageUpdate reads it.
fn find_embedded(info: &str) -> Result<Option<(Found, String)>, String> {
    let parts: Vec<&str> = info.split('|').collect();
    match parts.as_slice() {
        ["gh-releases-zsync", owner, repo, release, file] => {
            let pre = matches!(*release, "latest-pre" | "latest-all");
            let releases = if *release == "latest" || pre {
                github_releases(&format!("{owner}/{repo}"), pre)?
            } else {
                vec![get_json(&format!(
                    "https://api.github.com/repos/{owner}/{repo}/releases/tags/{release}"
                ))?]
            };
            Ok(
                from_release_list(&releases, file, "browser_download_url", true)?
                    .map(|f| (f, format!("GitHub {owner}/{repo}"))),
            )
        }
        ["zsync", url] => {
            let text = get_text(url)?;
            let (sha1, target, length) = zsync_header(&text);
            let target = target
                .map(|t| resolve(url, &t))
                .unwrap_or_else(|| url.trim_end_matches(".zsync").into());
            Ok(Some((
                Found {
                    download: Download {
                        name: target.rsplit('/').next().map(str::to_string),
                        url: target,
                        size: length,
                        sha1: sha1.clone(),
                        sha256: None,
                        sha512: None,
                    },
                    version: None,
                    zsync_sha1: sha1,
                },
                host(url),
            )))
        }
        _ => Err(format!("Unsupported embedded update information: {info}")),
    }
}

/// electron-updater's latest-linux.yml: version, and files with url/sha512/size.
/// (url, sha512, size) of one file in an electron-updater feed.
pub type FeedFile = (String, Option<String>, Option<u64>);

pub fn electron_feed(text: &str) -> (Option<String>, Vec<FeedFile>) {
    let mut version = None;
    let mut files: Vec<FeedFile> = Vec::new();
    let mut in_files = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if !line.starts_with(' ') && !line.starts_with('-') {
            in_files = trimmed.starts_with("files:");
            if let Some(v) = trimmed.strip_prefix("version:") {
                version = Some(v.trim().trim_matches(['"', '\'']).to_string());
            }
            continue;
        }
        if !in_files {
            continue;
        }
        let item = trimmed.trim_start_matches("- ");
        let Some((k, v)) = item.split_once(':') else {
            continue;
        };
        let v = v.trim().trim_matches(['"', '\'']).to_string();
        match k.trim() {
            "url" => files.push((v, None, None)),
            "sha512" => {
                if let Some(f) = files.last_mut() {
                    f.1 = Some(v);
                }
            }
            "size" => {
                if let Some(f) = files.last_mut() {
                    f.2 = v.parse().ok();
                }
            }
            _ => {}
        }
    }
    (version, files)
}

fn b64_to_hex(value: &str) -> Option<String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()
        .map(|b| crate::appimage::hex(&b))
}

fn self_updater_label(s: &SelfUpdater) -> String {
    match s {
        SelfUpdater::ElectronUpdater { provider, .. } => format!("electron-updater ({provider})"),
    }
}

fn check_self_updater(meta: &Metadata, s: &SelfUpdater) -> Check {
    let label = self_updater_label(s);
    let mut check = Check::new(
        Status::SelfUpdating,
        "This app updates itself. Open it to check for and install updates.",
    );
    check.origin = Some(Origin::SelfUpdater);
    check.source = Some("electron-updater".into());
    check.current_version = meta.version.clone();
    let SelfUpdater::ElectronUpdater {
        provider,
        owner,
        repo,
        url,
        channel,
    } = s;
    let channel = channel.clone().unwrap_or_else(|| "latest".into());
    let feed = match (provider.as_str(), owner, repo, url) {
        ("github", Some(o), Some(r), _) => {
            check.source_detail = Some(format!("GitHub {o}/{r}"));
            format!("https://github.com/{o}/{r}/releases/latest/download/{channel}-linux.yml")
        }
        ("generic", _, _, Some(u)) => {
            check.source_detail = Some(u.clone());
            format!("{}/{channel}-linux.yml", u.trim_end_matches('/'))
        }
        _ => {
            check.message = format!(
                "This app updates itself through {label}. Its update server cannot be read from here, so open the app to check for updates."
            );
            return check;
        }
    };
    let text = match get_text(&feed) {
        Ok(t) => t,
        Err(e) => {
            check.message = format!(
                "This app updates itself. Checking its update feed failed ({e}). Open the app to check for updates."
            );
            return check;
        }
    };
    let (version, files) = electron_feed(&text);
    check.available_version = version.clone();
    let Some((file, sha512, size)) = files
        .iter()
        .find(|f| f.0.to_lowercase().ends_with(".appimage"))
        .cloned()
    else {
        check.message =
            "This app updates itself. Its feed lists no AppImage, so open the app to update it."
                .into();
        return check;
    };
    let sha512 = sha512.as_deref().and_then(b64_to_hex);
    let newer = match &sha512 {
        Some(remote) => local_hash(&meta.path, "sha512").ok().map(|l| &l != remote),
        None => version
            .as_ref()
            .zip(meta.version.as_ref())
            .map(|(r, l)| r != l),
    };
    check.compared_by = Some(
        if sha512.is_some() {
            "sha512"
        } else {
            "version"
        }
        .into(),
    );
    check.update_known = newer;
    let download = Download {
        url: resolve(&feed, &file),
        name: Some(file.rsplit('/').next().unwrap_or(&file).to_string()),
        size,
        sha1: None,
        sha256: None,
        sha512,
    };
    match newer {
        Some(true) => {
            // The feed is the same file the app would install; it carries a
            // SHA-512, so it can be downloaded and verified here as well.
            check.status = Status::Available;
            check.can_install = download.sha512.is_some();
            check.message = if check.can_install {
                format!(
                    "Version {} is available from the app's own update feed. Install it here, or open the app and let it update itself.",
                    version.as_deref().unwrap_or("?")
                )
            } else {
                "A newer version is listed by the app's own updater. Open the app to install it."
                    .into()
            };
            if !check.can_install {
                check.status = Status::SelfUpdating;
            }
            check.download = Some(download);
        }
        Some(false) => {
            check.status = Status::UpToDate;
            check.message =
                "Up to date according to the app's own update feed. The app also updates itself."
                    .into();
        }
        None => {
            check.message = "This app updates itself. Its feed could not be compared with the installed file; open the app to check.".into();
        }
    }
    check
}

/// Decide what to check, in order: a custom source, Gear Lever's saved
/// source, embedded update information, then the app's own updater.
pub fn check(meta: &Metadata, source: Option<(&Source, Origin)>) -> Check {
    let self_note = meta.self_updater.as_ref().map(self_updater_label);
    let mut result = if let Some((source, origin)) = source {
        let mut c = run_check(
            meta,
            find_from_source(source),
            source.manager().label.into(),
            Some(source.summary()),
        );
        c.origin = Some(origin);
        c
    } else if let Some(info) = &meta.update_info {
        match find_embedded(info) {
            Ok(found) => {
                let detail = found.as_ref().map(|(_, d)| d.clone());
                let mut c = run_check(
                    meta,
                    Ok(found.map(|(f, _)| f)),
                    "Embedded update information".into(),
                    detail,
                );
                c.origin = Some(Origin::Embedded);
                c
            }
            Err(e) if e.starts_with("Unsupported") => match &meta.self_updater {
                Some(s) => return check_self_updater(meta, s),
                None => Check::new(
                    Status::NoSource,
                    format!("{e}. Add an update source to check for updates."),
                ),
            },
            Err(e) => {
                let mut c = Check::new(
                    Status::Failed,
                    format!("Checking the embedded update source failed: {e}"),
                );
                c.origin = Some(Origin::Embedded);
                c
            }
        }
    } else if let Some(s) = &meta.self_updater {
        return check_self_updater(meta, s);
    } else {
        Check::new(
            Status::NoSource,
            "No update source. This AppImage has no update information; add a source (for example its GitHub releases) to check for updates.",
        )
    };
    result.current_version = meta.version.clone();
    if self_note.is_some() && result.status != Status::SelfUpdating {
        result.self_updater = self_note;
    }
    result
}

fn run_check(
    meta: &Metadata,
    found: Result<Option<Found>, String>,
    label: String,
    detail: Option<String>,
) -> Check {
    let mut c = match found {
        Err(e) => Check::new(Status::Failed, format!("The update check failed: {e}")),
        Ok(None) => Check::new(
            Status::NotFound,
            "The source has no release file matching the configured name. Check the file name pattern.",
        ),
        Ok(Some(found)) => match compare(&meta.path, &found, meta.fingerprint.size) {
            Err(e) => Check::new(Status::Failed, e),
            Ok((newer, by)) => {
                let mut c = if newer {
                    let mut c = Check::new(
                        Status::Available,
                        match &found.version {
                            Some(v) => format!("{v} is available."),
                            None => "A different file is available.".into(),
                        },
                    );
                    c.can_install = true;
                    c
                } else {
                    Check::new(Status::UpToDate, "Up to date.")
                };
                c.compared_by = Some(by.into());
                c.available_version = found.version.clone();
                let mut d = found.download;
                if d.sha1.is_none() {
                    d.sha1 = found.zsync_sha1;
                }
                c.download = Some(d);
                c
            }
        },
    };
    c.source = Some(label);
    c.source_detail = detail;
    c
}

/// Download to `dest` (a new file), reporting progress 0..1, then verify the
/// size and any known checksum. Cancelling leaves no partial file.
pub fn download(
    d: &Download,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<Vec<String>, String> {
    let result = (|| -> Result<Vec<String>, String> {
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(dest)
            .map_err(|e| format!("Cannot create {}: {e}", dest.display()))?;
        let (mut reader, total): (Box<dyn Read>, Option<u64>) = if d.url.starts_with("ftp://") {
            let rest = d.url.trim_start_matches("ftp://");
            let (host, path) = rest.split_once('/').ok_or("Invalid FTP URL")?;
            let mut ftp = ftp_connect(&format!("ftp://{host}"))?;
            ftp.transfer_type(suppaftp::types::FileType::Binary)
                .map_err(|e| e.to_string())?;
            let mut data = Vec::new();
            ftp.retr(&format!("/{path}"), |r| {
                r.read_to_end(&mut data)
                    .map_err(suppaftp::FtpError::ConnectionError)?;
                Ok(())
            })
            .map_err(|e| format!("FTP download failed: {e}"))?;
            let _ = ftp.quit();
            let len = data.len() as u64;
            (Box::new(std::io::Cursor::new(data)), Some(len))
        } else {
            let resp = ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_connect(Some(Duration::from_secs(20)))
                    .http_status_as_error(false)
                    .user_agent(concat!("gearlever-cosmic/", env!("CARGO_PKG_VERSION")))
                    .build(),
            )
            .get(&d.url)
            .call()
            .map_err(|e| format!("Cannot download from {}: {e}", host(&d.url)))?;
            if !resp.status().is_success() {
                return Err(format!(
                    "{} answered HTTP {}",
                    host(&d.url),
                    resp.status().as_u16()
                ));
            }
            let total = resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .or(d.size);
            (Box::new(resp.into_body().into_reader()), total)
        };
        use sha2::Digest;
        let mut s1 = sha1::Sha1::new();
        let mut s256 = sha2::Sha256::new();
        let mut s512 = sha2::Sha512::new();
        let mut done = 0u64;
        let mut buf = vec![0; 256 << 10];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Download cancelled".into());
            }
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            s1.update(&buf[..n]);
            s256.update(&buf[..n]);
            s512.update(&buf[..n]);
            done += n as u64;
            if let Some(t) = total.filter(|t| *t > 0) {
                progress((done as f32 / t as f32).min(1.0));
            }
        }
        out.sync_all().map_err(|e| e.to_string())?;
        let mut verified = vec![format!("{done} bytes")];
        if let Some(t) = total
            && t != done
        {
            return Err(format!("Download incomplete: received {done} of {t} bytes"));
        }
        let hex = crate::appimage::hex;
        for (name, expected, actual) in [
            ("SHA-1", &d.sha1, hex(&s1.finalize())),
            ("SHA-256", &d.sha256, hex(&s256.finalize())),
            ("SHA-512", &d.sha512, hex(&s512.finalize())),
        ] {
            if let Some(expected) = expected {
                if expected != &actual {
                    return Err(format!(
                        "{name} checksum mismatch: expected {expected}, got {actual}"
                    ));
                }
                verified.push(format!("{name} verified"));
            }
        }
        if !crate::appimage::is_appimage(dest) {
            return Err("The downloaded file is not an AppImage; check that the source points at the AppImage file".into());
        }
        Ok(verified)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gear_lever_style_pairs_build_sources() {
        let s = Source::from_pairs(
            "GithubUpdater",
            &[
                ("repo".into(), "o/r".into()),
                ("repo_filename".into(), "A-*.AppImage".into()),
                ("allow_prereleases".into(), "False".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            s,
            Source::Github {
                repo: "o/r".into(),
                repo_filename: "A-*.AppImage".into(),
                allow_prereleases: false
            }
        );
        assert!(
            Source::from_pairs(
                "GithubUpdater",
                &[
                    ("repo".into(), "bad".into()),
                    ("repo_filename".into(), "x".into())
                ]
            )
            .is_err()
        );
        assert!(Source::from_pairs("StaticFileUpdater", &[("bogus".into(), "x".into())]).is_err());
        assert!(Source::from_pairs("Nope", &[]).is_err());
        assert_eq!(s.manager().label, "GitHub");
    }
    #[test]
    fn zsync_and_electron_feeds_parse() {
        let (sha, url, len) = zsync_header(
            "zsync: 0.6.2\nFilename: A.AppImage\nLength: 1234\nURL: A.AppImage\nSHA-1: ABCDEF\n\nbinary",
        );
        assert_eq!(
            (sha.as_deref(), url.as_deref(), len),
            (Some("abcdef"), Some("A.AppImage"), Some(1234))
        );
        let (v, files) = electron_feed(
            "version: 1.2.3\nfiles:\n  - url: App-1.2.3.AppImage\n    sha512: AAAA\n    size: 99\n  - url: app.deb\npath: App-1.2.3.AppImage\nsha512: AAAA\n",
        );
        assert_eq!(v.as_deref(), Some("1.2.3"));
        assert_eq!(
            files[0],
            ("App-1.2.3.AppImage".into(), Some("AAAA".into()), Some(99))
        );
        assert_eq!(files.len(), 2);
        assert_eq!(
            resolve("https://h/x/latest-linux.yml", "A.AppImage"),
            "https://h/x/A.AppImage"
        );
    }
    #[test]
    fn architecture_and_natural_order_pick_the_right_file() {
        let names = ["App-arm64.AppImage", "App-x86_64.AppImage"];
        let items: Vec<&&str> = names.iter().collect();
        if crate::elf::host_arch() == "x86_64" {
            assert_eq!(pick(items, |s| s).copied(), Some("App-x86_64.AppImage"));
        }
        assert!(natural_cmp("app-1.10", "app-1.9").is_gt());
    }
    #[test]
    fn an_app_without_any_source_is_never_called_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::appimage::tests::fixture(
            dir.path(),
            "a.AppImage",
            "",
            &[("/a.desktop", b"[Desktop Entry]\nName=A\nExec=a\n")],
            &[],
        );
        let meta = crate::appimage::inspect(
            &path,
            &crate::appimage::Options {
                cache_dir: None,
                allow_extract: false,
            },
        )
        .unwrap();
        let c = check(&meta, None);
        assert_eq!(c.status, Status::NoSource);
        assert!(c.message.contains("No update source"));
    }
    #[test]
    fn unreadable_self_updaters_say_to_open_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::appimage::tests::fixture(
            dir.path(),
            "s.AppImage",
            "",
            &[
                ("/s.desktop", b"[Desktop Entry]\nName=S\nExec=s\n"),
                ("/resources/app-update.yml", b"provider: s3\nbucket: x\n"),
            ],
            &[],
        );
        let meta = crate::appimage::inspect(
            &path,
            &crate::appimage::Options {
                cache_dir: None,
                allow_extract: false,
            },
        )
        .unwrap();
        let c = check(&meta, None);
        assert_eq!(c.status, Status::SelfUpdating);
        assert!(c.message.contains("open the app"));
        assert_eq!(c.update_known, None);
    }
}
