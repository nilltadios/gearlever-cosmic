// SPDX-License-Identifier: GPL-3.0-or-later
//! Where things live. Menu entries always go to the host user's
//! ~/.local/share/applications, also from inside the Flatpak.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    /// The host's XDG data home (normally ~/.local/share).
    pub share: PathBuf,
    /// This app's private data: reports, metadata cache, operation lock.
    pub data: PathBuf,
    /// This app's settings and per-app configuration.
    pub config: PathBuf,
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

impl Paths {
    pub fn discover() -> Result<Self, String> {
        let home = env_dir("HOME").ok_or("HOME is not set to an absolute path")?;
        // Inside a Flatpak XDG_DATA_HOME is the app's private folder; the
        // host's own value, if the user set one, is in HOST_XDG_DATA_HOME.
        let share = env_dir("HOST_XDG_DATA_HOME")
            .or_else(|| {
                (!crate::host::sandboxed())
                    .then(|| env_dir("XDG_DATA_HOME"))
                    .flatten()
            })
            .unwrap_or_else(|| home.join(".local/share"));
        let data = env_dir("XDG_DATA_HOME")
            .unwrap_or_else(|| home.join(".local/share"))
            .join("gearlever-cosmic");
        let config = env_dir("XDG_CONFIG_HOME")
            .unwrap_or_else(|| home.join(".config"))
            .join("gearlever-cosmic");
        Ok(Self {
            home,
            share,
            data,
            config,
        })
    }
    pub fn applications(&self) -> PathBuf {
        self.share.join("applications")
    }
    pub fn reports(&self) -> PathBuf {
        self.data.join("operations")
    }
    pub fn cache(&self) -> PathBuf {
        self.data.join("metadata-cache")
    }
    pub fn downloads(&self) -> PathBuf {
        self.data.join("downloads")
    }
    pub fn lock(&self) -> PathBuf {
        self.data.join("operation.lock")
    }
    /// Expand a leading `~` the way Gear Lever's folder setting does.
    pub fn expand(&self, value: &str) -> PathBuf {
        if value == "~" {
            self.home.clone()
        } else if let Some(rest) = value.strip_prefix("~/") {
            self.home.join(rest)
        } else {
            PathBuf::from(value)
        }
    }
    /// Gear Lever's configuration, read only, for importing update sources.
    pub fn gearlever_configs(&self) -> Vec<PathBuf> {
        vec![
            self.home
                .join(".var/app/it.mijorus.gearlever/config/gearlever.conf"),
            self.home.join(".config/gearlever.conf"),
        ]
    }
    /// Display a path with the home folder shortened to `~`.
    pub fn short(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

#[cfg(test)]
pub fn fixture(root: &Path) -> Paths {
    Paths {
        home: root.to_path_buf(),
        share: root.join(".local/share"),
        data: root.join(".local/share/gearlever-cosmic"),
        config: root.join(".config/gearlever-cosmic"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tilde_expands_only_at_the_start() {
        let p = fixture(Path::new("/home/u"));
        assert_eq!(p.expand("~/AppImages"), PathBuf::from("/home/u/AppImages"));
        assert_eq!(p.expand("/opt/~x"), PathBuf::from("/opt/~x"));
        assert_eq!(p.short(Path::new("/home/u/AppImages/a")), "~/AppImages/a");
    }
}
