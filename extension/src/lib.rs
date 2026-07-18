use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use zed::settings::LspSettings;
use zed::{Architecture, DownloadedFileType, Os};
use zed_extension_api as zed;

const NATIVE_HELPER_NAME: &str = "plantuml-export";
const NATIVE_HELPER_REPOSITORY: &str = "dahuangggg/plantuml-export";
const MAX_NATIVE_HELPER_BYTES: u64 = 64 * 1024 * 1024;
static INSTALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const NATIVE_HELPER_METADATA: &str = include_str!("../../release/native-helper-release.json");
const EXPECTED_ARTIFACTS: [(&str, &str); 6] = [
    (
        "aarch64-apple-darwin",
        "plantuml-export-aarch64-apple-darwin",
    ),
    ("x86_64-apple-darwin", "plantuml-export-x86_64-apple-darwin"),
    (
        "aarch64-unknown-linux-gnu",
        "plantuml-export-aarch64-unknown-linux-gnu",
    ),
    (
        "x86_64-unknown-linux-gnu",
        "plantuml-export-x86_64-unknown-linux-gnu",
    ),
    (
        "aarch64-pc-windows-msvc",
        "plantuml-export-aarch64-pc-windows-msvc.exe",
    ),
    (
        "x86_64-pc-windows-msvc",
        "plantuml-export-x86_64-pc-windows-msvc.exe",
    ),
];

struct PlantUmlExtension;

impl zed::Extension for PlantUmlExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let metadata = ReleaseMetadata::parse(NATIVE_HELPER_METADATA)?;
        let resolution = helper_resolution(&metadata, || {
            // Worktree::which is used only as an unpublished source-checkout
            // bootstrap. Published extensions always use the pinned helper.
            // Source: https://github.com/zed-industries/zed/blob/058f01fa93503491a735bfded53e77bfaa276148/crates/extension_api/wit/since_v0.6.0/extension.wit#L66-L75
            worktree.which(NATIVE_HELPER_NAME)
        })?;
        let command = match resolution {
            HelperResolution::Managed => resolve_managed_helper(&metadata)?,
            HelperResolution::Path(path) => path,
        };

        Ok(zed::Command {
            command,
            args: vec![
                "--root".to_string(),
                worktree.root_path(),
                "lsp".to_string(),
            ],
            env: Vec::new(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        Ok(LspSettings::for_worktree("plantuml-lsp", worktree)?.initialization_options)
    }
}

#[derive(Debug, Eq, PartialEq)]
enum HelperResolution {
    Managed,
    Path(String),
}

fn helper_resolution(
    metadata: &ReleaseMetadata,
    path_helper: impl FnOnce() -> Option<String>,
) -> zed::Result<HelperResolution> {
    match metadata.status {
        ReleaseStatus::Published => Ok(HelperResolution::Managed),
        ReleaseStatus::Unpublished => path_helper().map(HelperResolution::Path).ok_or_else(|| {
            metadata
                .published_tag()
                .expect_err("unpublished metadata must not have a release tag")
        }),
    }
}

fn resolve_managed_helper(metadata: &ReleaseMetadata) -> zed::Result<String> {
    let tag = metadata.published_tag()?;
    let platform = platform_target(zed::current_platform())?;
    let artifact = metadata.artifact_for(platform.target)?;

    let version_directory = PathBuf::from(format!("plantuml-export-{tag}"));
    fs::create_dir_all(&version_directory).map_err(|error| {
        format!(
            "failed to create native helper directory {}: {error}",
            version_directory.display()
        )
    })?;
    if let Some(binary_path) =
        find_cached_helper(&version_directory, platform.executable, &artifact.sha256)?
    {
        make_executable(&binary_path)?;
        ensure_helper_matches(&binary_path, &artifact.sha256, "cached native helper")?;
        return Ok(path_string(&binary_path));
    }

    let staging = InstallStaging::create(&version_directory)?;
    let temporary_path = staging.directory.join(platform.executable);

    // download_file writes only inside the extension's private working directory.
    // Source: https://github.com/zed-industries/zed/blob/058f01fa93503491a735bfded53e77bfaa276148/crates/extension_api/wit/since_v0.6.0/extension.wit#L53-L63
    if let Err(error) = zed::download_file(
        &artifact.url,
        &path_string(&temporary_path),
        DownloadedFileType::Uncompressed,
    ) {
        return Err(format!(
            "failed to download native helper {}: {error}",
            artifact.asset
        ));
    }

    let downloaded = regular_file_metadata(&temporary_path, "downloaded native helper")?
        .ok_or_else(|| "native helper download did not produce a file".to_string())?;
    if downloaded.len() == 0 || downloaded.len() > MAX_NATIVE_HELPER_BYTES {
        return Err(format!(
            "downloaded native helper size {} is outside the 1..={MAX_NATIVE_HELPER_BYTES} byte limit",
            downloaded.len()
        ));
    }

    let actual_sha256 = sha256_file(&temporary_path)?;
    if actual_sha256 != artifact.sha256 {
        return Err(format!(
            "native helper checksum mismatch for {}: expected {}, got {}",
            artifact.asset, artifact.sha256, actual_sha256
        ));
    }

    let binary_path = install_verified_helper(
        &temporary_path,
        &version_directory,
        platform.executable,
        &artifact.sha256,
    )?;
    ensure_helper_matches(&binary_path, &artifact.sha256, "installed native helper")?;
    make_executable(&binary_path)?;
    ensure_helper_matches(&binary_path, &artifact.sha256, "executable native helper")?;
    Ok(path_string(&binary_path))
}

fn make_executable(path: &Path) -> zed::Result<()> {
    zed::make_file_executable(&path_string(path)).map_err(|error| {
        format!(
            "failed to mark native helper executable at {}: {error}",
            path.display()
        )
    })
}

fn helper_matches(path: &Path, expected_sha256: &str) -> zed::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed to inspect native helper {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.len() == 0 || metadata.len() > MAX_NATIVE_HELPER_BYTES {
        return Ok(false);
    }
    Ok(sha256_file(path)? == expected_sha256)
}

fn ensure_helper_matches(path: &Path, expected_sha256: &str, label: &str) -> zed::Result<()> {
    if helper_matches(path, expected_sha256)? {
        return Ok(());
    }
    Err(format!(
        "{label} failed its post-install size or SHA-256 check: {}",
        path.display()
    ))
}

fn find_cached_helper(
    version_directory: &Path,
    executable: &str,
    expected_sha256: &str,
) -> zed::Result<Option<PathBuf>> {
    let canonical_name = managed_helper_name(executable, expected_sha256, None);
    let candidate_prefix = managed_helper_name(executable, expected_sha256, Some(""));
    let windows_executable = executable.ends_with(".exe");
    let mut candidates = fs::read_dir(version_directory)
        .map_err(|error| {
            format!(
                "failed to inspect native helper directory {}: {error}",
                version_directory.display()
            )
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            (name == canonical_name
                || managed_helper_name_matches(&name, &candidate_prefix, windows_executable))
            .then_some(entry.path())
        })
        .collect::<Vec<_>>();
    candidates.sort();

    for candidate in candidates {
        if helper_matches(&candidate, expected_sha256)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn install_verified_helper(
    temporary: &Path,
    version_directory: &Path,
    executable: &str,
    expected_sha256: &str,
) -> zed::Result<PathBuf> {
    install_verified_helper_with(
        temporary,
        version_directory,
        executable,
        expected_sha256,
        |source, destination| fs::hard_link(source, destination),
    )
}

fn install_verified_helper_with(
    temporary: &Path,
    version_directory: &Path,
    executable: &str,
    expected_sha256: &str,
    create_canonical_link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> zed::Result<PathBuf> {
    ensure_helper_matches(temporary, expected_sha256, "downloaded native helper")?;
    let canonical = version_directory.join(managed_helper_name(executable, expected_sha256, None));

    match create_canonical_link(temporary, &canonical) {
        Ok(()) => return Ok(canonical),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        // Some WASI hosts or filesystems may not provide hard links. A unique
        // same-directory rename below remains atomic and never replaces a
        // shared path.
        Err(_) => {}
    }
    if helper_matches(&canonical, expected_sha256)? {
        return Ok(canonical);
    }

    install_unique_helper(temporary, version_directory, executable, expected_sha256)
}

fn install_unique_helper(
    temporary: &Path,
    version_directory: &Path,
    executable: &str,
    expected_sha256: &str,
) -> zed::Result<PathBuf> {
    for _ in 0..64 {
        let token = unique_install_token()?;
        let candidate = version_directory.join(managed_helper_name(
            executable,
            expected_sha256,
            Some(&token),
        ));
        let reservation = version_directory.join(format!(".reserve-{token}"));
        let reservation_file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&reservation)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve native helper install path {}: {error}",
                    reservation.display()
                ));
            }
        };

        let install_result = match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::rename(temporary, &candidate).map_err(|error| {
                    format!(
                        "failed to install verified native helper at {}: {error}",
                        candidate.display()
                    )
                })
            }
            Ok(_) => Err(format!(
                "reserved native helper path already exists: {}",
                candidate.display()
            )),
            Err(error) => Err(format!(
                "failed to inspect reserved native helper path {}: {error}",
                candidate.display()
            )),
        };
        drop(reservation_file);
        let reservation_cleanup = remove_file_if_present(&reservation);

        match (install_result, reservation_cleanup) {
            // The checksum-addressed candidate is already complete and usable.
            // A unique stale reservation is harmless and must not turn a
            // successful installation into a one-time startup failure.
            (Ok(()), _) => return Ok(candidate),
            (Err(install_error), Ok(())) => return Err(install_error),
            (Err(install_error), Err(cleanup_error)) => {
                return Err(format!(
                    "{install_error}; reservation cleanup also failed: {cleanup_error}"
                ));
            }
        }
    }
    Err(format!(
        "failed to allocate a unique native helper install path under {}",
        version_directory.display()
    ))
}

fn managed_helper_name(executable: &str, sha256: &str, suffix: Option<&str>) -> String {
    let (stem, extension) = executable
        .strip_suffix(".exe")
        .map_or((executable, ""), |stem| (stem, ".exe"));
    match suffix {
        None => format!("{stem}-{sha256}{extension}"),
        Some("") => format!("{stem}-{sha256}-"),
        Some(suffix) => format!("{stem}-{sha256}-{suffix}{extension}"),
    }
}

fn managed_helper_name_matches(name: &str, prefix: &str, windows_executable: bool) -> bool {
    if !name.starts_with(prefix) || name.len() == prefix.len() {
        return false;
    }
    if windows_executable {
        name[prefix.len()..]
            .strip_suffix(".exe")
            .is_some_and(|token| !token.is_empty())
    } else {
        !name[prefix.len()..].contains('.')
    }
}

struct InstallStaging {
    directory: PathBuf,
}

impl InstallStaging {
    fn create(version_directory: &Path) -> zed::Result<Self> {
        for _ in 0..64 {
            let directory = version_directory.join(format!(".install-{}", unique_install_token()?));
            match fs::create_dir(&directory) {
                Ok(()) => return Ok(Self { directory }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "failed to create native helper staging directory {}: {error}",
                        directory.display()
                    ));
                }
            }
        }
        Err(format!(
            "failed to allocate a unique native helper staging directory under {}",
            version_directory.display()
        ))
    }
}

impl Drop for InstallStaging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn unique_install_token() -> zed::Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock cannot name native helper staging: {error}"))?
        .as_nanos();
    let sequence = INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{timestamp:x}-{sequence:x}"))
}

fn regular_file_metadata(path: &Path, label: &str) -> zed::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => Err(format!(
            "{label} path is not a regular file: {}",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn remove_file_if_present(path: &Path) -> zed::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove {}: {error}", path.display())),
    }
}

fn sha256_file(path: &Path) -> zed::Result<String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    sha256_reader(file).map_err(|error| format!("failed to hash {}: {error}", path.display()))
}

fn sha256_reader(mut reader: impl Read) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

struct PlatformTarget {
    target: &'static str,
    executable: &'static str,
}

fn platform_target((os, architecture): (Os, Architecture)) -> zed::Result<PlatformTarget> {
    let (target, executable) = match (os, architecture) {
        (Os::Mac, Architecture::Aarch64) => ("aarch64-apple-darwin", "plantuml-export"),
        (Os::Mac, Architecture::X8664) => ("x86_64-apple-darwin", "plantuml-export"),
        (Os::Linux, Architecture::Aarch64) => ("aarch64-unknown-linux-gnu", "plantuml-export"),
        (Os::Linux, Architecture::X8664) => ("x86_64-unknown-linux-gnu", "plantuml-export"),
        (Os::Windows, Architecture::Aarch64) => ("aarch64-pc-windows-msvc", "plantuml-export.exe"),
        (Os::Windows, Architecture::X8664) => ("x86_64-pc-windows-msvc", "plantuml-export.exe"),
        _ => {
            return Err(
                "PlantUML Export supports only macOS, Linux, and Windows on aarch64 or x86_64"
                    .to_string(),
            );
        }
    };
    Ok(PlatformTarget { target, executable })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseMetadata {
    schema_version: u32,
    repository: String,
    status: ReleaseStatus,
    release_tag: Option<String>,
    artifacts: Vec<ReleaseArtifact>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ReleaseStatus {
    Unpublished,
    Published,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseArtifact {
    target: String,
    asset: String,
    url: String,
    sha256: String,
}

impl ReleaseMetadata {
    fn parse(json: &str) -> zed::Result<Self> {
        let metadata: Self = zed::serde_json::from_str(json)
            .map_err(|error| format!("invalid native helper release metadata: {error}"))?;
        metadata.validate()?;
        Ok(metadata)
    }

    fn validate(&self) -> zed::Result<()> {
        if self.schema_version != 1 {
            return Err("native helper metadata schemaVersion must be 1".to_string());
        }
        if self.repository != NATIVE_HELPER_REPOSITORY {
            return Err(format!(
                "native helper metadata repository must be {NATIVE_HELPER_REPOSITORY}"
            ));
        }

        match self.status {
            ReleaseStatus::Unpublished => {
                if self.release_tag.is_some() || !self.artifacts.is_empty() {
                    return Err(
                        "unpublished native helper metadata must not contain a tag or artifacts"
                            .to_string(),
                    );
                }
            }
            ReleaseStatus::Published => {
                let tag = self.release_tag.as_deref().ok_or_else(|| {
                    "published native helper metadata requires a releaseTag".to_string()
                })?;
                if !valid_release_tag(tag) {
                    return Err(
                        "native helper releaseTag must be v0.1.0 or v0.1.0-rc.N".to_string()
                    );
                }
                if self.artifacts.len() != EXPECTED_ARTIFACTS.len() {
                    return Err(
                        "published native helper metadata must contain exactly six artifacts"
                            .to_string(),
                    );
                }

                for (target, expected_asset) in EXPECTED_ARTIFACTS {
                    let matches = self
                        .artifacts
                        .iter()
                        .filter(|artifact| artifact.target == target)
                        .collect::<Vec<_>>();
                    if matches.len() != 1 || matches[0].asset != expected_asset {
                        return Err(format!(
                            "native helper metadata must contain exactly one {expected_asset}"
                        ));
                    }
                    let artifact = matches[0];
                    let expected_url = format!(
                        "https://github.com/{NATIVE_HELPER_REPOSITORY}/releases/download/{tag}/{expected_asset}"
                    );
                    if artifact.url != expected_url {
                        return Err(format!(
                            "native helper URL must be pinned to {expected_url}"
                        ));
                    }
                    if !valid_sha256(&artifact.sha256) {
                        return Err(format!(
                            "native helper {} has an invalid SHA-256",
                            artifact.asset
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn published_tag(&self) -> zed::Result<&str> {
        match self.status {
            ReleaseStatus::Published => self
                .release_tag
                .as_deref()
                .ok_or_else(|| "published native helper metadata has no releaseTag".to_string()),
            ReleaseStatus::Unpublished => Err(
                "native helpers are not published yet; this source checkout needs a locally built `plantuml-export` development helper. Released extensions download and verify the helper automatically"
                    .to_string(),
            ),
        }
    }

    fn artifact_for(&self, target: &str) -> zed::Result<&ReleaseArtifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.target == target)
            .ok_or_else(|| format!("native helper release has no artifact for {target}"))
    }
}

fn valid_release_tag(tag: &str) -> bool {
    if tag == "v0.1.0" {
        return true;
    }
    let Some(number) = tag.strip_prefix("v0.1.0-rc.") else {
        return false;
    };
    number
        .parse::<u64>()
        .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == number)
}

fn valid_sha256(value: &str) -> bool {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return false;
    }
    let first = value.as_bytes()[0];
    value.as_bytes().iter().any(|byte| *byte != first)
}

zed::register_extension!(PlantUmlExtension);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn published_metadata_never_uses_a_path_helper() {
        let metadata = published_metadata();

        assert_eq!(
            helper_resolution(&metadata, || {
                panic!("published metadata must not inspect PATH")
            })
            .unwrap(),
            HelperResolution::Managed
        );
    }

    #[test]
    fn unpublished_metadata_requires_a_development_path_helper() {
        let metadata = unpublished_metadata();

        assert_eq!(
            helper_resolution(&metadata, || Some("/tmp/dev-helper".to_string())).unwrap(),
            HelperResolution::Path("/tmp/dev-helper".to_string())
        );
        assert!(helper_resolution(&metadata, || None)
            .unwrap_err()
            .contains("not published yet"));
    }

    #[test]
    fn checked_in_metadata_matches_its_declared_release_state() {
        let metadata = ReleaseMetadata::parse(NATIVE_HELPER_METADATA).unwrap();

        match metadata.status {
            ReleaseStatus::Unpublished => {
                assert!(metadata
                    .published_tag()
                    .unwrap_err()
                    .contains("not published yet"));
                assert!(metadata.artifacts.is_empty());
            }
            ReleaseStatus::Published => {
                assert_eq!(
                    metadata.published_tag().unwrap(),
                    metadata.release_tag.as_deref().unwrap()
                );
                assert_eq!(metadata.artifacts.len(), EXPECTED_ARTIFACTS.len());
            }
        }
    }

    #[test]
    fn published_metadata_requires_all_six_exact_assets() {
        let mut metadata = published_metadata();
        metadata.artifacts.pop();

        assert!(metadata.validate().unwrap_err().contains("exactly six"));
    }

    #[test]
    fn published_metadata_maps_target_to_pinned_asset() {
        let metadata = published_metadata();
        metadata.validate().unwrap();

        let artifact = metadata.artifact_for("aarch64-pc-windows-msvc").unwrap();
        assert_eq!(
            artifact.asset,
            "plantuml-export-aarch64-pc-windows-msvc.exe"
        );
        assert!(artifact.url.ends_with(&artifact.asset));
    }

    #[test]
    fn checksum_validation_rejects_placeholders_and_uppercase() {
        assert!(!valid_sha256(&"0".repeat(64)));
        assert!(!valid_sha256(&format!("{}1", "A".repeat(63))));
        assert!(valid_sha256(&format!("{:064x}", 42)));
    }

    #[test]
    fn sha256_reader_matches_the_standard_empty_digest() {
        assert_eq!(
            sha256_reader(&b""[..]).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn install_staging_paths_are_unique_and_cleaned_on_drop() {
        let fixture = TestDirectory::new("unique-staging");
        let first = InstallStaging::create(&fixture.path).unwrap();
        let second = InstallStaging::create(&fixture.path).unwrap();
        let first_path = first.directory.clone();
        let second_path = second.directory.clone();

        assert_ne!(first_path, second_path);
        assert!(first_path.is_dir());
        assert!(second_path.is_dir());

        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[test]
    fn concurrent_verified_installs_never_replace_a_shared_path() {
        let fixture = TestDirectory::new("concurrent-install");
        let first_staging = InstallStaging::create(&fixture.path).unwrap();
        let second_staging = InstallStaging::create(&fixture.path).unwrap();
        let first_download = first_staging.directory.join("plantuml-export");
        let second_download = second_staging.directory.join("plantuml-export");
        fs::write(&first_download, b"verified helper").unwrap();
        fs::write(&second_download, b"verified helper").unwrap();
        let expected = sha256_reader(&b"verified helper"[..]).unwrap();
        let barrier = Arc::new(Barrier::new(2));

        let install = |download: PathBuf, barrier: Arc<Barrier>| {
            let version_directory = fixture.path.clone();
            let expected = expected.clone();
            std::thread::spawn(move || {
                barrier.wait();
                install_verified_helper(&download, &version_directory, "plantuml-export", &expected)
                    .unwrap()
            })
        };
        let first = install(first_download, Arc::clone(&barrier));
        let second = install(second_download, barrier);
        let first_path = first.join().unwrap();
        let second_path = second.join().unwrap();

        ensure_helper_matches(&first_path, &expected, "first concurrent helper").unwrap();
        ensure_helper_matches(&second_path, &expected, "second concurrent helper").unwrap();
        let cached = find_cached_helper(&fixture.path, "plantuml-export", &expected)
            .unwrap()
            .unwrap();
        ensure_helper_matches(&cached, &expected, "cached concurrent helper").unwrap();
    }

    #[test]
    fn unsupported_hard_links_use_concurrent_unique_atomic_installs() {
        let fixture = TestDirectory::new("unsupported-hard-link");
        let first_staging = InstallStaging::create(&fixture.path).unwrap();
        let second_staging = InstallStaging::create(&fixture.path).unwrap();
        let first_download = first_staging.directory.join("plantuml-export");
        let second_download = second_staging.directory.join("plantuml-export");
        fs::write(&first_download, b"verified helper").unwrap();
        fs::write(&second_download, b"verified helper").unwrap();
        let expected = sha256_reader(&b"verified helper"[..]).unwrap();
        let barrier = Arc::new(Barrier::new(2));

        let install = |download: PathBuf, barrier: Arc<Barrier>| {
            let version_directory = fixture.path.clone();
            let expected = expected.clone();
            std::thread::spawn(move || {
                barrier.wait();
                install_verified_helper_with(
                    &download,
                    &version_directory,
                    "plantuml-export",
                    &expected,
                    |_, _| {
                        Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "hard links unavailable in test host",
                        ))
                    },
                )
                .unwrap()
            })
        };
        let first = install(first_download, Arc::clone(&barrier));
        let second = install(second_download, barrier);
        let first_path = first.join().unwrap();
        let second_path = second.join().unwrap();

        assert_ne!(first_path, second_path);
        ensure_helper_matches(&first_path, &expected, "first fallback helper").unwrap();
        ensure_helper_matches(&second_path, &expected, "second fallback helper").unwrap();
        assert!(!fixture
            .path
            .join(managed_helper_name("plantuml-export", &expected, None))
            .exists());
        let cached = find_cached_helper(&fixture.path, "plantuml-export", &expected)
            .unwrap()
            .unwrap();
        ensure_helper_matches(&cached, &expected, "cached fallback helper").unwrap();
    }

    #[test]
    fn corrupt_shared_candidate_is_preserved_while_a_unique_helper_is_installed() {
        let fixture = TestDirectory::new("corrupt-candidate");
        let staging = InstallStaging::create(&fixture.path).unwrap();
        let temporary = staging.directory.join("downloaded-helper");
        fs::write(&temporary, b"verified helper").unwrap();
        let expected = sha256_reader(&b"verified helper"[..]).unwrap();
        let canonical = fixture
            .path
            .join(managed_helper_name("plantuml-export", &expected, None));
        fs::write(&canonical, b"old corrupt helper").unwrap();

        let installed =
            install_verified_helper(&temporary, &fixture.path, "plantuml-export", &expected)
                .unwrap();
        ensure_helper_matches(&installed, &expected, "test helper").unwrap();

        assert_ne!(installed, canonical);
        assert_eq!(fs::read(canonical).unwrap(), b"old corrupt helper");
    }

    #[test]
    fn non_regular_checksum_candidate_is_skipped_without_execution() {
        let fixture = TestDirectory::new("non-regular-candidate");
        let staging = InstallStaging::create(&fixture.path).unwrap();
        let temporary = staging.directory.join("downloaded-helper");
        fs::write(&temporary, b"verified helper").unwrap();
        let expected = sha256_reader(&b"verified helper"[..]).unwrap();
        let canonical = fixture
            .path
            .join(managed_helper_name("plantuml-export", &expected, None));
        fs::create_dir(&canonical).unwrap();

        let installed =
            install_verified_helper(&temporary, &fixture.path, "plantuml-export", &expected)
                .unwrap();

        assert_ne!(installed, canonical);
        assert!(canonical.is_dir());
        ensure_helper_matches(&installed, &expected, "fallback helper").unwrap();
    }

    #[test]
    fn managed_helper_names_preserve_the_windows_executable_suffix() {
        let digest = "a".repeat(64);
        let canonical = managed_helper_name("plantuml-export.exe", &digest, None);
        let unique = managed_helper_name("plantuml-export.exe", &digest, Some("token"));

        assert_eq!(canonical, format!("plantuml-export-{digest}.exe"));
        assert_eq!(unique, format!("plantuml-export-{digest}-token.exe"));
        assert!(managed_helper_name_matches(
            &unique,
            &format!("plantuml-export-{digest}-"),
            true
        ));
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "plantuml-export-extension-{label}-{}",
                unique_install_token().unwrap()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn published_metadata() -> ReleaseMetadata {
        let tag = "v0.1.0-rc.1";
        ReleaseMetadata {
            schema_version: 1,
            repository: NATIVE_HELPER_REPOSITORY.to_string(),
            status: ReleaseStatus::Published,
            release_tag: Some(tag.to_string()),
            artifacts: EXPECTED_ARTIFACTS
                .iter()
                .enumerate()
                .map(|(index, (target, asset))| ReleaseArtifact {
                    target: (*target).to_string(),
                    asset: (*asset).to_string(),
                    url: format!(
                        "https://github.com/{NATIVE_HELPER_REPOSITORY}/releases/download/{tag}/{asset}"
                    ),
                    sha256: format!("{:064x}", index + 1),
                })
                .collect(),
        }
    }

    fn unpublished_metadata() -> ReleaseMetadata {
        ReleaseMetadata {
            schema_version: 1,
            repository: NATIVE_HELPER_REPOSITORY.to_string(),
            status: ReleaseStatus::Unpublished,
            release_tag: None,
            artifacts: Vec::new(),
        }
    }
}
