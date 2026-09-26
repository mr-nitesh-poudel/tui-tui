//! Who you are, and who you have played, kept between runs.
//!
//! The profile lives in one directory (`TUI_TUI_HOME`, or `tui-tui` under
//! the platform's config directory):
//!
//! - `identity.key` — the endpoint's secret key, readable only by you. Keeping
//!   it is what lets friends find you again without a code.
//! - `identity.lock` — held while the game runs, so that two copies on one
//!   machine never answer to the same key.
//! - `profile.json` — your name and your contacts.
//!
//! A second copy that finds the lock taken runs as a guest: a throwaway key,
//! and nothing saved.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

pub use crate::session::name::clean_name;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: EndpointId,
    pub name: String,
    pub games: u32,
    /// Seconds since the Unix epoch.
    pub last_played: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    name: String,
    #[serde(default)]
    contacts: Vec<Contact>,
}

pub struct Profile {
    /// Where to save, or `None` for a guest.
    dir: Option<PathBuf>,
    pub secret: SecretKey,
    pub name: String,
    /// Most recently played first.
    pub contacts: Vec<Contact>,
    /// Held for as long as the profile is, which is what keeps the lock.
    _lock: Option<File>,
}

impl Profile {
    /// The profile in the usual place.
    ///
    /// # Errors
    ///
    /// If there is no config directory, or [`Profile::load_from`] fails.
    pub fn load() -> Result<Self> {
        let dir = match std::env::var_os("TUI_TUI_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => dirs::config_dir()
                .context("no config directory to keep your profile in")?
                .join("tui-tui"),
        };
        Self::load_from(&dir)
    }

    /// The profile kept in `dir`, making it if there is none.
    ///
    /// # Errors
    ///
    /// If `dir` cannot be made or locked, or the key or profile in it cannot
    /// be read or created. A profile already locked by another copy is not an
    /// error: that copy gets a guest profile instead.
    pub fn load_from(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        let lock = File::create(dir.join("identity.lock")).context("opening the profile lock")?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(Self::guest()),
            Err(TryLockError::Error(e)) => return Err(e).context("locking the profile"),
        }

        let secret = load_or_create_key(&dir.join("identity.key"))?;
        let stored: Stored = match fs::read(dir.join("profile.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("reading profile.json")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(e).context("reading profile.json"),
        };
        let name = clean_name(&stored.name).unwrap_or_else(default_name);
        let mut profile = Self {
            dir: Some(dir.to_path_buf()),
            secret,
            name,
            contacts: stored.contacts,
            _lock: Some(lock),
        };
        profile.sort();
        Ok(profile)
    }

    /// A throwaway identity that saves nothing.
    #[must_use]
    pub fn guest() -> Self {
        Self {
            dir: None,
            secret: SecretKey::generate(),
            name: default_name(),
            contacts: Vec::new(),
            _lock: None,
        }
    }

    #[must_use]
    pub fn is_guest(&self) -> bool {
        self.dir.is_none()
    }

    #[must_use]
    pub fn id(&self) -> EndpointId {
        self.secret.public()
    }

    #[must_use]
    pub fn contact(&self, id: EndpointId) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.id == id)
    }

    /// Go by `name` from now on.
    ///
    /// # Errors
    ///
    /// If `name` has no letter or digit in it, or the profile cannot be saved.
    pub fn set_name(&mut self, name: &str) -> Result<()> {
        match clean_name(name) {
            Some(name) => self.name = name,
            None => bail!("a name needs at least one letter or digit"),
        }
        self.save()
    }

    /// Note a game with `id`, adding them if they are new and taking the name
    /// they go by now.
    ///
    /// # Errors
    ///
    /// If the profile cannot be saved.
    pub fn played(&mut self, id: EndpointId, name: &str) -> Result<()> {
        let name = clean_name(name).unwrap_or_else(|| id.fmt_short().to_string());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        match self.contacts.iter_mut().find(|c| c.id == id) {
            Some(c) => {
                c.name = name;
                c.games += 1;
                c.last_played = now;
            }
            None => self.contacts.push(Contact {
                id,
                name,
                games: 1,
                last_played: now,
            }),
        }
        self.sort();
        self.save()
    }

    /// Drop `id` from the contacts.
    ///
    /// # Errors
    ///
    /// If the profile cannot be saved.
    pub fn forget(&mut self, id: EndpointId) -> Result<()> {
        self.contacts.retain(|c| c.id != id);
        self.save()
    }

    fn sort(&mut self) {
        self.contacts
            .sort_by_key(|c| std::cmp::Reverse(c.last_played));
    }

    fn save(&self) -> Result<()> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let stored = Stored {
            name: self.name.clone(),
            contacts: self.contacts.clone(),
        };
        let json = serde_json::to_vec_pretty(&stored)?;
        // Written aside and renamed over, so a crash never leaves half a file.
        let tmp = dir.join("profile.json.tmp");
        fs::write(&tmp, json).context("saving profile")?;
        fs::rename(&tmp, dir.join("profile.json")).context("saving profile")?;
        Ok(())
    }
}

fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let bytes = decode_hex32(text.trim()).context("identity.key is damaged")?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            let hex = key.to_bytes().iter().fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(path).context("creating identity.key")?;
            file.write_all(hex.as_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e).context("reading identity.key"),
    }
}

fn decode_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn default_name() -> String {
    ["USER", "USERNAME"]
        .iter()
        .find_map(|var| std::env::var(var).ok())
        .and_then(|n| clean_name(&n))
        .unwrap_or_else(|| "player".into())
}
