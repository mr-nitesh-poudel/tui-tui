//! Fetching Stockfish for a player who has no engine, when they ask for it.
//!
//! It comes from Stockfish's own releases, a build pinned here by version and
//! SHA-256, so what runs is exactly what was checked in: a file that has been
//! tampered with, or swapped for another on the server, is refused before it
//! is unpacked, let alone run. It is fetched with the system's `curl` and
//! unpacked with its `tar`, both on every machine this is built for, and kept
//! in tuitui's data directory with Stockfish's licence beside it. It is
//! downloaded to the player's machine, not bundled with this program, so its
//! GPL stays its own.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::games::Waker;

/// The Stockfish release fetched, and where it comes from.
pub const VERSION: &str = "19";
const RELEASES: &str = "https://github.com/official-stockfish/Stockfish/releases/download/sf_19";
/// What the unpacked engine is called once installed.
const BINARY: &str = "stockfish";
/// How often the download's progress is looked at.
const POLL: Duration = Duration::from_millis(200);

/// One of Stockfish's builds, as published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Build {
    pub asset: &'static str,
    /// In bytes, for the progress bar.
    pub size: u64,
    pub sha256: &'static str,
}

const MACOS: Build = Build {
    asset: "stockfish-macos-universal.tar.gz",
    size: 82_323_876,
    sha256: "a1f0e3bcc5a6927a11fe6fc8e54a779754645f3c2bae2cf13420fd1957adaa77",
};
const LINUX_X86_64: Build = Build {
    asset: "stockfish-linux-x86-64-universal.tar.gz",
    size: 81_388_977,
    sha256: "9defc0d4e55d49c65a6d042f3e571a39fcea499ade6dbe741b53b8c65e03611f",
};
const LINUX_ARM64: Build = Build {
    asset: "stockfish-linux-arm64-universal.tar.gz",
    size: 80_181_250,
    sha256: "fe26cfd1d9db4c8af3d21e24d9ff34cacb31c1f940085a7583da11796f2bac01",
};

/// The build for an operating system and processor, named as Rust names
/// them, if Stockfish publishes one this can install. Its builds pick the
/// best instructions the processor has as they start, so one does for each.
#[must_use]
pub fn build_for(os: &str, arch: &str) -> Option<Build> {
    match (os, arch) {
        ("macos", "aarch64" | "x86_64") => Some(MACOS),
        ("linux", "x86_64") => Some(LINUX_X86_64),
        ("linux", "aarch64") => Some(LINUX_ARM64),
        _ => None,
    }
}

/// The build for this machine.
#[must_use]
pub fn this_build() -> Option<Build> {
    build_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Where downloaded engines live: `TUI_TUI_HOME` if it is set, as the
/// profile does, else the platform's data directory.
#[must_use]
pub fn home() -> Option<PathBuf> {
    match std::env::var_os("TUI_TUI_HOME") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => dirs::data_dir().map(|d| d.join("tui-tui")),
    }
}

/// Where this version goes, under `home`.
#[must_use]
pub fn install_dir(home: &Path) -> PathBuf {
    home.join("engines").join(format!("stockfish-{VERSION}"))
}

/// The engine, if it has been downloaded before.
#[must_use]
pub fn installed() -> Option<PathBuf> {
    let path = install_dir(&home()?).join(BINARY);
    path.is_file().then_some(path)
}

/// How a download is going.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Fetching { got: u64, of: u64 },
    Checking,
    Unpacking,
    Done(PathBuf),
    Failed(String),
}

impl Progress {
    /// A line for the status bar while it is under way.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Progress::Fetching { got, of } => {
                let percent = got.saturating_mul(100) / (*of).max(1);
                format!(
                    "downloading Stockfish {VERSION}… {}%  ({} of {} MB)",
                    percent.min(100),
                    got / 1_000_000,
                    of / 1_000_000
                )
            }
            Progress::Checking => "checking the download…".into(),
            Progress::Unpacking => "unpacking Stockfish…".into(),
            Progress::Done(_) => "Stockfish is ready".into(),
            Progress::Failed(why) => why.clone(),
        }
    }
}

/// A download under way. Dropping it gives up, stopping `curl` and leaving
/// nothing half-installed behind.
pub struct Download {
    progress: Arc<Mutex<Progress>>,
    task: JoinHandle<()>,
}

impl Drop for Download {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Download {
    /// Starts fetching `build` into `home`, calling `wake` as it goes.
    ///
    /// # Errors
    ///
    /// If there is no async runtime to download on.
    pub fn start(build: Build, home: PathBuf, wake: Option<Waker>) -> Result<Self, String> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "downloading needs the async runtime".to_string())?;
        let progress = Arc::new(Mutex::new(Progress::Fetching {
            got: 0,
            of: build.size,
        }));
        let wake = wake.unwrap_or_else(|| Arc::new(|| {}));
        let task = runtime.spawn({
            let progress = progress.clone();
            async move {
                let url = format!("{RELEASES}/{}", build.asset);
                let result = install(&url, build, &home, &progress, &wake).await;
                *lock(&progress) = match result {
                    Ok(path) => Progress::Done(path),
                    Err(why) => Progress::Failed(why),
                };
                wake();
            }
        });
        Ok(Self { progress, task })
    }

    #[must_use]
    pub fn progress(&self) -> Progress {
        lock(&self.progress).clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Fetches, checks and unpacks `build` from `url`, and puts the engine in
/// place under `home`. Everything happens in a staging directory beside the
/// final one, which is only swapped in once the engine is known good.
///
/// # Errors
///
/// If the engine cannot be fetched, fails its check, or cannot be unpacked
/// and put in place; the message says which, for the player.
pub async fn install(
    url: &str,
    build: Build,
    home: &Path,
    progress: &Mutex<Progress>,
    wake: &Waker,
) -> Result<PathBuf, String> {
    let dir = install_dir(home);
    let staging = dir.with_extension("partial");
    // Whatever an earlier attempt left.
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| format!("could not make {}: {e}", staging.display()))?;
    let result = async {
        let archive = staging.join(build.asset);
        fetch(url, &archive, build.size, progress, wake).await?;

        *lock(progress) = Progress::Checking;
        wake();
        verify(&archive, build.sha256).await?;

        *lock(progress) = Progress::Unpacking;
        wake();
        unpack(&archive, &staging).await?;
        let engine =
            find_engine(&staging.join("stockfish")).ok_or("the download had no engine in it")?;

        // Swap the new engine in for any old one.
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("could not make {}: {e}", dir.display()))?;
        let installed = dir.join(BINARY);
        tokio::fs::rename(&engine, &installed)
            .await
            .map_err(|e| format!("could not install the engine: {e}"))?;
        // Its licence goes with it.
        let _ = tokio::fs::rename(
            staging.join("stockfish").join("Copying.txt"),
            dir.join("Copying.txt"),
        )
        .await;
        Ok(installed)
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    result
}

/// Downloads `url` to `to` with `curl`, saying how far it has got.
async fn fetch(
    url: &str,
    to: &Path,
    size: u64,
    progress: &Mutex<Progress>,
    wake: &Waker,
) -> Result<(), String> {
    let mut curl = Command::new("curl")
        // HTTPS only, even through redirects, and fail on an HTTP error
        // rather than saving the error page.
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"])
        .args(["--retry", "2", "--output"])
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "downloading needs curl, which is not installed".into(),
            _ => format!("could not start curl: {e}"),
        })?;
    let status = loop {
        tokio::select! {
            status = curl.wait() => break status.map_err(|e| format!("curl failed: {e}"))?,
            () = tokio::time::sleep(POLL) => {
                let got = tokio::fs::metadata(to).await.map_or(0, |m| m.len());
                *lock(progress) = Progress::Fetching { got, of: size };
                wake();
            }
        }
    };
    if !status.success() {
        let mut said = String::new();
        if let Some(mut err) = curl.stderr.take() {
            use tokio::io::AsyncReadExt;
            let _ = (&mut err).take(500).read_to_string(&mut said).await;
        }
        let said = said.trim();
        return Err(if said.is_empty() {
            "the download failed".into()
        } else {
            format!("the download failed: {said}")
        });
    }
    Ok(())
}

/// Refuses a file whose SHA-256 is not `expected`.
async fn verify(file: &Path, expected: &str) -> Result<(), String> {
    let file = file.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut hasher = Sha256::new();
        std::io::copy(&mut std::fs::File::open(file)?, &mut hasher)?;
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await
    .map_err(|e| format!("could not check the download: {e}"))?
    .map_err(|e| format!("could not check the download: {e}"))?;
    if actual == expected {
        Ok(())
    } else {
        Err("the download was not the file expected, so it was thrown away".into())
    }
}

/// Unpacks a `.tar.gz` into `into` with the system's `tar`.
async fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("could not run tar: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("could not unpack the download".into())
    }
}

/// The engine in an unpacked release: the one file named `stockfish…` that
/// is a program rather than a text file.
pub fn find_engine(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("stockfish") && path.is_file() && is_program(path)
        })
}

#[cfg(unix)]
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_program(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_machine_this_is_built_for_has_a_build() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ] {
            let build = build_for(os, arch).unwrap_or_else(|| panic!("{os} {arch}"));
            assert_eq!(build.sha256.len(), 64);
            assert!(build.asset.ends_with(".tar.gz"));
        }
        assert_eq!(build_for("windows", "x86_64"), None);
        assert_eq!(build_for("linux", "riscv64"), None);
    }

    /// A directory of its own for a test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuitui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_download_is_kept_only_if_it_is_the_file_expected() {
        let file = scratch("verify").join("abc");
        std::fs::write(&file, "abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(verify(&file, abc).await, Ok(()));
        std::fs::write(&file, "abd").unwrap();
        assert!(verify(&file, abc).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_engine_is_found_in_what_is_unpacked() {
        use std::os::unix::fs::PermissionsExt;
        let made = scratch("pack");
        let release = made.join("stockfish");
        std::fs::create_dir_all(&release).unwrap();
        let engine = release.join("stockfish-linux-x86-64-universal");
        std::fs::write(&engine, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(release.join("Copying.txt"), "GPL").unwrap();
        std::fs::write(release.join("stockfish-notes.txt"), "not a program").unwrap();
        let archive = made.join("release.tar.gz");
        let packed = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&made)
            .arg("stockfish")
            .status()
            .unwrap();
        assert!(packed.success());

        let into = scratch("unpack");
        unpack(&archive, &into).await.unwrap();
        let found = find_engine(&into.join("stockfish")).unwrap();
        assert_eq!(
            found.file_name().unwrap(),
            "stockfish-linux-x86-64-universal"
        );
        assert!(unpack(&made.join("missing.tar.gz"), &into).await.is_err());
    }

    #[test]
    fn progress_reads_as_a_percentage() {
        let p = Progress::Fetching {
            got: 41_000_000,
            of: 82_000_000,
        };
        assert!(p.describe().contains("50%"), "{}", p.describe());
        // Never past 100, and never a division by nothing.
        let over = Progress::Fetching { got: 9, of: 0 };
        assert!(over.describe().contains("100%"));
    }
}
