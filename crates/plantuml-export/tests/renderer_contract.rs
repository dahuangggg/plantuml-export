use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use plantuml_export::renderer::{
    self, build_render_command, build_syntax_command, check_health, ensure_managed_jar_with,
    ensure_managed_java_with, extract_managed_java_archive,
    extract_managed_java_archive_with_limits, managed_jar_path, managed_java_asset,
    managed_java_dir, managed_java_path, minimum_java_major, parse_java_major, validate_output,
    ChecksumVerifier, CommandExecutor, CommandSpec, DownloadError, Downloader, Format,
    HealthStatus, InstallPolicy, Layout, ManagedArchiveFormat, ManagedJavaAsset, ProcessFailure,
    ProcessOutput, RenderRequest, RenderSecurity, Renderer, RendererError, SecurityProfile,
    SyntaxRequest, MANAGED_JAVA_VERSION, MANAGED_PLANTUML_SHA256, MANAGED_PLANTUML_URL,
    MANAGED_PLANTUML_VERSION, MAX_MANAGED_JAR_BYTES,
};

#[test]
fn managed_artifact_is_fixed_to_the_accepted_plantuml_release() {
    assert_eq!(MANAGED_PLANTUML_VERSION, "1.2026.6");
    assert_eq!(
        MANAGED_PLANTUML_URL,
        "https://github.com/plantuml/plantuml/releases/download/v1.2026.6/plantuml.jar"
    );
    fn assert_production_downloader<T: Downloader>() {}
    assert_production_downloader::<renderer::UreqDownloader>();
    assert_eq!(
        MANAGED_PLANTUML_SHA256,
        "89948f14c93756c7a3fb7b69078ff37e8489fd79dd430c582b931e2f65358690"
    );
    assert_eq!(MAX_MANAGED_JAR_BYTES, 64 * 1024 * 1024);
}

#[test]
fn managed_java_is_pinned_for_all_six_release_targets() {
    let expected = [
        ("macos", "aarch64", ManagedArchiveFormat::TarGz, "4b7a8cd2"),
        ("macos", "x86_64", ManagedArchiveFormat::TarGz, "b341fb8e"),
        ("linux", "aarch64", ManagedArchiveFormat::TarGz, "fa23d9d9"),
        ("linux", "x86_64", ManagedArchiveFormat::TarGz, "e5038aae"),
        ("windows", "aarch64", ManagedArchiveFormat::Zip, "22e2c2b8"),
        ("windows", "x86_64", ManagedArchiveFormat::Zip, "be26677a"),
    ];

    assert_eq!(MANAGED_JAVA_VERSION, "21.0.11+10");
    for (os, architecture, archive_format, checksum_prefix) in expected {
        let asset = managed_java_asset(os, architecture).unwrap();
        assert_eq!(asset.archive_format, archive_format);
        assert!(asset.url.contains("jdk-21.0.11%2B10"));
        assert!(asset.sha256.starts_with(checksum_prefix));
        assert_eq!(asset.sha256.len(), 64);
        assert!(asset.java_relative_path.ends_with(if os == "windows" {
            "bin/java.exe"
        } else {
            "bin/java"
        }));
    }
    assert!(managed_java_asset("freebsd", "x86_64").is_none());
    assert!(managed_java_asset("macos", "riscv64").is_none());
}

#[test]
fn managed_command_uses_public_network_with_workspace_scoped_local_reads() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let input = root.join("docs/model.puml");
    let include = root.join("shared/includes");
    let cache = temp.path().join("cache");
    let request = request(
        &root,
        &input,
        Format::Svg,
        Layout::Graphviz,
        false,
        vec![include.clone()],
    );

    let command = build_render_command(
        &Renderer::Managed {
            java: PathBuf::from("java17"),
            cache_dir: cache.clone(),
        },
        &request,
    )
    .unwrap();

    assert_eq!(command.program, PathBuf::from("java17"));
    assert!(command.args.contains(&OsString::from("-jar")));
    assert!(command
        .args
        .contains(&managed_jar_path(&cache).into_os_string()));
    assert!(command.args.contains(&OsString::from("--format")));
    assert!(command.args.contains(&OsString::from("svg")));
    assert!(command.args.contains(&OsString::from("--output-dir")));
    assert!(command.args.contains(&OsString::from("--stop-on-error")));
    assert!(command.env.contains(&(
        OsString::from("GRAPHVIZ_DOT"),
        root.join("dot").into_os_string(),
    )));
    assert!(command
        .args
        .contains(&OsString::from("--ignore-startuml-filename")));
    assert!(command.args.contains(&OsString::from("--disable-metadata")));
    assert!(command
        .args
        .iter()
        .any(|arg| arg.to_string_lossy() == "-DPLANTUML_SECURITY_PROFILE=INTERNET"));
    let allowlist = command
        .args
        .iter()
        .find(|arg| {
            arg.to_string_lossy()
                .starts_with("-Dplantuml.allowlist.path=")
        })
        .unwrap()
        .to_string_lossy();
    assert!(allowlist.contains(&root.to_string_lossy().to_string()));
    assert!(allowlist.contains(&input.parent().unwrap().to_string_lossy().to_string()));
    assert!(allowlist.contains(&include.to_string_lossy().to_string()));
    assert!(command
        .args
        .contains(&OsString::from("-Dplantuml.allowlist.url=")));
    assert!(!command
        .env
        .iter()
        .any(|(key, _)| key.to_string_lossy().contains("ALLOWLIST_URL")));
    assert!(command
        .env_remove
        .contains(&OsString::from("plantuml.include.path")));
    assert!(command
        .env_remove
        .contains(&OsString::from("plantuml.allowlist.url")));
    for inherited_java_options in ["JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS"] {
        assert!(command
            .env_remove
            .contains(&OsString::from(inherited_java_options)));
    }
    assert!(command.env_remove.contains(&OsString::from("GRAPHVIZ_DOT")));
}

#[test]
fn renderer_mode_is_explicit_and_never_falls_back_to_another_program() {
    let temp = tempfile::tempdir().unwrap();
    let request = request(
        temp.path(),
        &temp.path().join("model.puml"),
        Format::Png,
        Layout::Graphviz,
        false,
        vec![],
    );
    let binary = build_render_command(
        &Renderer::Binary {
            executable: PathBuf::from("chosen-plantuml"),
        },
        &request,
    )
    .unwrap();
    let jar = build_render_command(
        &Renderer::Jar {
            java: PathBuf::from("chosen-java"),
            jar: PathBuf::from("chosen.jar"),
        },
        &request,
    )
    .unwrap();

    assert_eq!(binary.program, PathBuf::from("chosen-plantuml"));
    assert!(!binary.args.contains(&OsString::from("-jar")));
    assert!(binary.env.contains(&(
        OsString::from("PLANTUML_SECURITY_PROFILE"),
        OsString::from("INTERNET")
    )));
    assert!(binary
        .env
        .iter()
        .any(|(key, _)| key == "PLANTUML_ALLOWLIST_PATH"));
    assert!(binary
        .env
        .contains(&(OsString::from("PLANTUML_ALLOWLIST_URL"), OsString::new(),)));
    assert!(binary
        .env_remove
        .contains(&OsString::from("PLANTUML_SECURITY_PROFILE")));
    assert_eq!(jar.program, PathBuf::from("chosen-java"));
    assert!(jar.args.contains(&OsString::from("chosen.jar")));
}

#[test]
fn syntax_command_uses_an_isolated_diagnostic_render_and_the_same_security_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("model.puml");
    let command = build_syntax_command(
        &Renderer::Jar {
            java: PathBuf::from("chosen-java"),
            jar: PathBuf::from("chosen.jar"),
        },
        &SyntaxRequest {
            input: input.clone(),
            output_dir: temp.path().join("syntax-output"),
            worktree_root: temp.path().to_path_buf(),
            include_paths: vec![temp.path().join("includes")],
            security: RenderSecurity {
                profile: SecurityProfile::Internet,
                allowed_remote_urls: vec![],
            },
        },
    )
    .unwrap();

    assert_eq!(command.program, PathBuf::from("chosen-java"));
    assert!(command.args.contains(&OsString::from("chosen.jar")));
    assert!(!command.args.contains(&OsString::from("--check-syntax")));
    assert!(command.args.contains(&OsString::from("--stop-on-error")));
    assert!(command.args.contains(&OsString::from("--no-error-image")));
    assert!(command.args.contains(&OsString::from("-stdrpt:1")));
    assert!(command.args.contains(&input.into_os_string()));
    assert!(command.args.contains(&OsString::from("--format")));
    assert!(command.args.contains(&OsString::from("svg")));
    assert!(command.args.contains(&OsString::from("--output-dir")));
    assert!(command
        .args
        .contains(&temp.path().join("syntax-output").into_os_string()));
    assert!(command
        .args
        .contains(&OsString::from("--ignore-startuml-filename")));
    assert!(command.args.contains(&OsString::from("--disable-metadata")));
    assert!(command.args.contains(&OsString::from("-Playout=smetana")));
    assert!(command
        .args
        .iter()
        .any(|argument| argument.to_string_lossy() == "-DPLANTUML_SECURITY_PROFILE=INTERNET"));
    assert!(command
        .env_remove
        .contains(&OsString::from("plantuml.allowlist.url")));
}

#[test]
fn concurrent_syntax_output_directories_are_private_distinct_and_cleaned() {
    let first = plantuml_export::renderer::create_syntax_output_dir().unwrap();
    let second = plantuml_export::renderer::create_syntax_output_dir().unwrap();
    let first_path = first.path().to_path_buf();
    let second_path = second.path().to_path_buf();
    assert_ne!(first_path, second_path);
    assert!(first_path.is_dir());
    assert!(second_path.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(&first_path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    first.close().unwrap();
    second.close().unwrap();
    assert!(!first_path.exists());
    assert!(!second_path.exists());
}

#[test]
fn explicit_remote_roots_are_passed_only_with_the_selected_secure_profile() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut request = request(
        root,
        &root.join("model.puml"),
        Format::Svg,
        Layout::Smetana,
        false,
        vec![],
    );
    request.security = RenderSecurity {
        profile: SecurityProfile::Allowlist,
        allowed_remote_urls: vec![
            "http://plantuml.internal:8080/".to_string(),
            "https://example.com/".to_string(),
        ],
    };

    let command = build_render_command(
        &Renderer::Jar {
            java: PathBuf::from("java"),
            jar: PathBuf::from("plantuml.jar"),
        },
        &request,
    )
    .unwrap();

    assert!(command
        .args
        .contains(&OsString::from("-DPLANTUML_SECURITY_PROFILE=ALLOWLIST")));
    assert!(command.args.contains(&OsString::from(
        "-Dplantuml.allowlist.url=http://plantuml.internal:8080/;https://example.com/"
    )));

    let command = build_render_command(
        &Renderer::Binary {
            executable: PathBuf::from("plantuml"),
        },
        &request,
    )
    .unwrap();
    assert!(command.env.contains(&(
        OsString::from("PLANTUML_SECURITY_PROFILE"),
        OsString::from("ALLOWLIST"),
    )));
    assert!(command.env.contains(&(
        OsString::from("PLANTUML_ALLOWLIST_URL"),
        OsString::from("http://plantuml.internal:8080/;https://example.com/"),
    )));
}

#[test]
fn syntax_command_uses_the_same_explicit_remote_policy_as_export() {
    let temp = tempfile::tempdir().unwrap();
    let command = build_syntax_command(
        &Renderer::Jar {
            java: PathBuf::from("java"),
            jar: PathBuf::from("plantuml.jar"),
        },
        &SyntaxRequest {
            input: temp.path().join("model.puml"),
            output_dir: temp.path().join("syntax-output"),
            worktree_root: temp.path().to_path_buf(),
            include_paths: vec![],
            security: RenderSecurity {
                profile: SecurityProfile::Allowlist,
                allowed_remote_urls: vec!["https://example.com/".to_string()],
            },
        },
    )
    .unwrap();

    assert!(command
        .args
        .contains(&OsString::from("-DPLANTUML_SECURITY_PROFILE=ALLOWLIST")));
    assert!(command.args.contains(&OsString::from(
        "-Dplantuml.allowlist.url=https://example.com/"
    )));
}

#[test]
fn smetana_and_metadata_are_added_only_by_explicit_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("model.puml");
    let graphviz = build_render_command(
        &Renderer::Binary {
            executable: PathBuf::from("plantuml"),
        },
        &request(
            temp.path(),
            &input,
            Format::Svg,
            Layout::Graphviz,
            false,
            vec![],
        ),
    )
    .unwrap();
    let smetana = build_render_command(
        &Renderer::Binary {
            executable: PathBuf::from("plantuml"),
        },
        &request(
            temp.path(),
            &input,
            Format::Svg,
            Layout::Smetana,
            true,
            vec![],
        ),
    )
    .unwrap();

    assert!(!graphviz.args.contains(&OsString::from("-Playout=smetana")));
    assert!(graphviz.env.iter().any(|(key, _)| key == "GRAPHVIZ_DOT"));
    assert!(graphviz
        .args
        .contains(&OsString::from("--disable-metadata")));
    assert!(smetana.args.contains(&OsString::from("-Playout=smetana")));
    assert!(!smetana.env.iter().any(|(key, _)| key == "GRAPHVIZ_DOT"));
    assert!(!smetana.args.contains(&OsString::from("--disable-metadata")));
}

#[test]
fn java_version_policy_is_17_for_svg_png_and_21_for_pdf() {
    assert_eq!(minimum_java_major(Format::Svg), 17);
    assert_eq!(minimum_java_major(Format::Png), 17);
    assert_eq!(minimum_java_major(Format::Pdf), 21);
    assert_eq!(
        parse_java_major("openjdk version \"17.0.12\" 2024-07-16"),
        Some(17)
    );
    assert_eq!(parse_java_major("java version \"1.8.0_402\""), Some(8));
    assert_eq!(parse_java_major("openjdk 21-ea 2023-09-19"), Some(21));
    assert_eq!(parse_java_major("not a java version"), None);
}

#[test]
fn health_is_typed_and_requires_graphviz_only_for_graphviz_layout() {
    let temp = tempfile::tempdir().unwrap();
    let jar = temp.path().join("custom.jar");
    fs::write(&jar, b"custom jar").unwrap();
    let executor = FakeExecutor::new(vec![
        ok("openjdk version \"17.0.12\""),
        ok("dot - graphviz version 13.1.0"),
    ]);

    let report = check_health(
        &Renderer::Jar {
            java: PathBuf::from("java"),
            jar,
        },
        Format::Pdf,
        Layout::Graphviz,
        Path::new("dot"),
        &executor,
    );

    assert!(!report.ready);
    assert_eq!(report.renderer.status, HealthStatus::Ready);
    assert_eq!(report.java.status, HealthStatus::Incompatible);
    assert_eq!(report.graphviz.status, HealthStatus::Ready);
    assert_eq!(
        executor.programs(),
        vec![PathBuf::from("java"), PathBuf::from("dot")]
    );

    let smetana_executor = FakeExecutor::new(vec![ok("openjdk version \"17.0.12\"")]);
    let smetana = check_health(
        &Renderer::Jar {
            java: PathBuf::from("java"),
            jar: temp.path().join("custom.jar"),
        },
        Format::Svg,
        Layout::Smetana,
        Path::new("dot"),
        &smetana_executor,
    );
    assert!(smetana.ready);
    assert_eq!(smetana.graphviz.status, HealthStatus::NotRequired);
    assert_eq!(smetana_executor.programs(), vec![PathBuf::from("java")]);
}

#[test]
fn process_executor_classifies_nonzero_and_timeout() {
    let executor = renderer::StdCommandExecutor;
    let nonzero = executor
        .execute(&platform_command("exit 7"), Duration::from_secs(2))
        .unwrap_err();
    let timeout = executor
        .execute(&platform_command("sleep 2"), Duration::from_millis(20))
        .unwrap_err();

    assert!(matches!(nonzero, ProcessFailure::NonZero { .. }));
    assert!(matches!(timeout, ProcessFailure::Timeout { .. }));
}

#[test]
fn process_executor_removes_inherited_security_values_before_setting_explicit_ones() {
    let executor = renderer::StdCommandExecutor;
    let output = executor
        .execute(&platform_security_env_command(), Duration::from_secs(2))
        .unwrap();

    assert_eq!(output.stdout, b"INTERNET");
}

#[test]
fn validates_nonempty_svg_png_and_pdf_signatures() {
    let temp = tempfile::tempdir().unwrap();
    let svg = temp.path().join("diagram.svg");
    let png = temp.path().join("diagram.png");
    let pdf = temp.path().join("diagram.pdf");
    fs::write(&svg, b"<?xml version=\"1.0\"?><svg></svg>").unwrap();
    fs::write(&png, b"\x89PNG\r\n\x1a\nrest").unwrap();
    fs::write(&pdf, b"%PDF-1.7\nrest").unwrap();

    validate_output(&svg, Format::Svg).unwrap();
    validate_output(&png, Format::Png).unwrap();
    validate_output(&pdf, Format::Pdf).unwrap();

    fs::write(&pdf, b"").unwrap();
    assert!(matches!(
        validate_output(&pdf, Format::Pdf),
        Err(RendererError::EmptyOutput { .. })
    ));
    fs::write(&pdf, b"not a pdf").unwrap();
    assert!(matches!(
        validate_output(&pdf, Format::Pdf),
        Err(RendererError::OutputType { .. })
    ));
}

#[test]
fn managed_install_uses_fixed_url_temp_checksum_lock_and_atomic_rename() {
    let temp = tempfile::tempdir().unwrap();
    let downloader = RecordingDownloader::new(b"verified jar");
    let verifier = MarkerVerifier;

    let installed = ensure_managed_jar_with(
        temp.path(),
        false,
        &downloader,
        &verifier,
        InstallPolicy::for_tests(),
    )
    .unwrap();

    assert_eq!(installed, managed_jar_path(temp.path()));
    assert_eq!(fs::read(&installed).unwrap(), b"verified jar");
    assert_eq!(downloader.urls(), vec![MANAGED_PLANTUML_URL]);
    assert!(downloader
        .destinations()
        .iter()
        .all(|path| path.to_string_lossy().contains(".tmp-")));
    assert!(temp.path().join("plantuml-1.2026.6.jar.lock").is_file());
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        2,
        "only the installed JAR and persistent advisory-lock file may remain"
    );
}

#[test]
fn managed_install_honors_offline_checksum_and_lock_failures() {
    let offline = tempfile::tempdir().unwrap();
    let downloader = RecordingDownloader::new(b"verified jar");
    assert!(matches!(
        ensure_managed_jar_with(
            offline.path(),
            true,
            &downloader,
            &MarkerVerifier,
            InstallPolicy::for_tests(),
        ),
        Err(RendererError::Offline { .. })
    ));
    assert!(downloader.destinations().is_empty());

    let mismatch = tempfile::tempdir().unwrap();
    let mismatch_downloader = RecordingDownloader::new(b"corrupt");
    assert!(matches!(
        ensure_managed_jar_with(
            mismatch.path(),
            false,
            &mismatch_downloader,
            &MarkerVerifier,
            InstallPolicy::for_tests(),
        ),
        Err(RendererError::Checksum { .. })
    ));
    assert!(!managed_jar_path(mismatch.path()).exists());

    let stale = tempfile::tempdir().unwrap();
    fs::write(stale.path().join("plantuml-1.2026.6.jar.lock"), b"dead pid").unwrap();
    ensure_managed_jar_with(
        stale.path(),
        false,
        &RecordingDownloader::new(b"verified jar"),
        &MarkerVerifier,
        InstallPolicy::for_tests(),
    )
    .expect("an unlocked file left by a crashed process must not block installation");

    let locked = tempfile::tempdir().unwrap();
    let lock_path = locked.path().join("plantuml-1.2026.6.jar.lock");
    let held_lock = open_and_lock(&lock_path);
    assert!(matches!(
        ensure_managed_jar_with(
            locked.path(),
            false,
            &RecordingDownloader::new(b"verified jar"),
            &MarkerVerifier,
            InstallPolicy::for_tests(),
        ),
        Err(RendererError::LockTimeout { .. })
    ));
    drop(held_lock);
}

#[test]
fn managed_install_rejects_oversized_downloads_and_removes_the_temp_file() {
    let temp = tempfile::tempdir().unwrap();

    let result = ensure_managed_jar_with(
        temp.path(),
        false,
        &OversizeDownloader,
        &MarkerVerifier,
        InstallPolicy::for_tests(),
    );

    assert!(matches!(
        result,
        Err(RendererError::DownloadTooLarge {
            limit: MAX_MANAGED_JAR_BYTES,
            actual,
            ..
        }) if actual == MAX_MANAGED_JAR_BYTES + 1
    ));
    assert!(!managed_jar_path(temp.path()).exists());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    assert!(temp.path().join("plantuml-1.2026.6.jar.lock").is_file());
}

#[test]
fn managed_java_install_verifies_extracts_marks_and_reuses_the_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let archive = fake_tar_gz("fake-jre/bin/java", b"managed java");
    let asset = fake_java_asset(ManagedArchiveFormat::TarGz, "bin/java");
    let downloader = RecordingDownloader::new(&archive);
    let verifier = BodyVerifier::new(&archive, asset.sha256);

    let installed = ensure_managed_java_with(
        temp.path(),
        false,
        asset,
        &downloader,
        &verifier,
        InstallPolicy::for_tests(),
    )
    .unwrap();

    assert_eq!(installed, managed_java_path(temp.path(), asset));
    assert_eq!(fs::read(&installed).unwrap(), b"managed java");
    assert_eq!(downloader.raw_urls(), vec![asset.url.to_string()]);
    let marker =
        fs::read_to_string(managed_java_dir(temp.path()).join(".plantuml-export-managed-jre"))
            .unwrap();
    assert!(marker.contains("version=21.0.11+10"));
    assert!(marker.contains(asset.sha256));
    assert!(temp.path().join("temurin-jre-21.0.11+10.lock").is_file());

    let no_download = RecordingDownloader::new(b"must not be used");
    assert_eq!(
        ensure_managed_java_with(
            temp.path(),
            true,
            asset,
            &no_download,
            &verifier,
            InstallPolicy::for_tests(),
        )
        .unwrap(),
        installed
    );
    assert!(no_download.raw_urls().is_empty());
}

#[test]
fn managed_java_install_honors_offline_and_checksum_failures() {
    let archive = fake_tar_gz("fake-jre/bin/java", b"managed java");
    let asset = fake_java_asset(ManagedArchiveFormat::TarGz, "bin/java");
    let verifier = BodyVerifier::new(&archive, asset.sha256);

    let offline = tempfile::tempdir().unwrap();
    let downloader = RecordingDownloader::new(&archive);
    assert!(matches!(
        ensure_managed_java_with(
            offline.path(),
            true,
            asset,
            &downloader,
            &verifier,
            InstallPolicy::for_tests(),
        ),
        Err(RendererError::Offline { .. })
    ));
    assert!(downloader.raw_urls().is_empty());

    let mismatch = tempfile::tempdir().unwrap();
    let corrupt = RecordingDownloader::new(b"not the verified archive");
    assert!(matches!(
        ensure_managed_java_with(
            mismatch.path(),
            false,
            asset,
            &corrupt,
            &verifier,
            InstallPolicy::for_tests(),
        ),
        Err(RendererError::Checksum { .. })
    ));
    assert!(!managed_java_dir(mismatch.path()).exists());
}

#[test]
fn managed_java_extracts_zip_and_rejects_path_traversal() {
    let fixture = tempfile::tempdir().unwrap();
    let archive = fixture.path().join("runtime.zip");
    fs::write(
        &archive,
        fake_zip(&[("fake-jre/bin/java.exe", b"managed java")]),
    )
    .unwrap();
    let output = fixture.path().join("output");

    extract_managed_java_archive(&archive, &output, ManagedArchiveFormat::Zip).unwrap();
    assert_eq!(
        fs::read(output.join("fake-jre/bin/java.exe")).unwrap(),
        b"managed java"
    );

    let unsafe_archive = fixture.path().join("unsafe.zip");
    fs::write(&unsafe_archive, fake_zip(&[("../escape", b"escape")])).unwrap();
    let unsafe_output = fixture.path().join("unsafe-output");
    assert!(matches!(
        extract_managed_java_archive(&unsafe_archive, &unsafe_output, ManagedArchiveFormat::Zip,),
        Err(RendererError::Archive { .. })
    ));
    assert!(!fixture.path().join("escape").exists());
}

#[test]
fn managed_java_rejects_archive_links_and_expansion_limits() {
    let fixture = tempfile::tempdir().unwrap();
    let safe_archive = fixture.path().join("safe-link.tar.gz");
    fs::write(&safe_archive, fake_tar_with_safe_symlink()).unwrap();
    let safe_output = fixture.path().join("safe-link-output");
    extract_managed_java_archive(&safe_archive, &safe_output, ManagedArchiveFormat::TarGz).unwrap();
    let materialized = safe_output.join("fake-jre/legal/java.rmi/LICENSE");
    assert_eq!(fs::read(&materialized).unwrap(), b"license text");
    assert!(fs::symlink_metadata(&materialized)
        .unwrap()
        .file_type()
        .is_file());

    let linked_archive = fixture.path().join("linked.tar.gz");
    fs::write(
        &linked_archive,
        fake_tar_symlink("fake-jre/bin/java", "../../../outside"),
    )
    .unwrap();
    assert!(matches!(
        extract_managed_java_archive(
            &linked_archive,
            &fixture.path().join("linked-output"),
            ManagedArchiveFormat::TarGz,
        ),
        Err(RendererError::Archive { .. })
    ));

    let oversized_archive = fixture.path().join("oversized.tar.gz");
    fs::write(
        &oversized_archive,
        fake_tar_gz("fake-jre/bin/java", b"more than four bytes"),
    )
    .unwrap();
    assert!(matches!(
        extract_managed_java_archive_with_limits(
            &oversized_archive,
            &fixture.path().join("oversized-output"),
            ManagedArchiveFormat::TarGz,
            4,
            10,
        ),
        Err(RendererError::Archive { .. })
    ));
}

fn open_and_lock(path: &Path) -> File {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .unwrap();
    file.lock().unwrap();
    file
}

fn request(
    root: &Path,
    input: &Path,
    format: Format,
    layout: Layout,
    embed_source_metadata: bool,
    include_paths: Vec<PathBuf>,
) -> RenderRequest {
    RenderRequest {
        input: input.to_path_buf(),
        output_dir: root.join("out"),
        worktree_root: root.to_path_buf(),
        include_paths,
        security: RenderSecurity {
            profile: SecurityProfile::Internet,
            allowed_remote_urls: vec![],
        },
        format,
        layout,
        graphviz_path: root.join("dot"),
        embed_source_metadata,
    }
}

fn ok(output: &str) -> Result<ProcessOutput, ProcessFailure> {
    Ok(ProcessOutput {
        stdout: output.as_bytes().to_vec(),
        stderr: Vec::new(),
    })
}

struct FakeExecutor {
    results: Mutex<VecDeque<Result<ProcessOutput, ProcessFailure>>>,
    programs: Mutex<Vec<PathBuf>>,
}

impl FakeExecutor {
    fn new(results: Vec<Result<ProcessOutput, ProcessFailure>>) -> Self {
        Self {
            results: Mutex::new(results.into()),
            programs: Mutex::new(Vec::new()),
        }
    }

    fn programs(&self) -> Vec<PathBuf> {
        self.programs.lock().unwrap().clone()
    }
}

impl CommandExecutor for FakeExecutor {
    fn execute(
        &self,
        command: &CommandSpec,
        _timeout: Duration,
    ) -> Result<ProcessOutput, ProcessFailure> {
        self.programs.lock().unwrap().push(command.program.clone());
        self.results.lock().unwrap().pop_front().unwrap()
    }
}

struct RecordingDownloader {
    body: Vec<u8>,
    calls: Mutex<Vec<(String, PathBuf)>>,
}

impl RecordingDownloader {
    fn new(body: &[u8]) -> Self {
        Self {
            body: body.to_vec(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn urls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(url, _)| {
                if url == MANAGED_PLANTUML_URL {
                    MANAGED_PLANTUML_URL
                } else {
                    "unexpected"
                }
            })
            .collect()
    }

    fn destinations(&self) -> Vec<PathBuf> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, path)| path.clone())
            .collect()
    }

    fn raw_urls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }
}

impl Downloader for RecordingDownloader {
    fn download_to(&self, url: &str, destination: &Path) -> Result<(), DownloadError> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), destination.to_path_buf()));
        fs::write(destination, &self.body).map_err(|error| DownloadError(error.to_string()))
    }
}

struct OversizeDownloader;

impl Downloader for OversizeDownloader {
    fn download_to(&self, _url: &str, destination: &Path) -> Result<(), DownloadError> {
        fs::OpenOptions::new()
            .write(true)
            .open(destination)
            .and_then(|file| file.set_len(MAX_MANAGED_JAR_BYTES + 1))
            .map_err(|error| DownloadError(error.to_string()))
    }
}

struct MarkerVerifier;

impl ChecksumVerifier for MarkerVerifier {
    fn sha256(&self, path: &Path) -> Result<String, RendererError> {
        let bytes = fs::read(path).map_err(|error| RendererError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        Ok(if bytes == b"verified jar" {
            MANAGED_PLANTUML_SHA256.to_string()
        } else {
            "bad".to_string()
        })
    }
}

struct BodyVerifier {
    body: Vec<u8>,
    expected: &'static str,
}

impl BodyVerifier {
    fn new(body: &[u8], expected: &'static str) -> Self {
        Self {
            body: body.to_vec(),
            expected,
        }
    }
}

impl ChecksumVerifier for BodyVerifier {
    fn sha256(&self, path: &Path) -> Result<String, RendererError> {
        let bytes = fs::read(path).map_err(|error| RendererError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        Ok(if bytes == self.body {
            self.expected.to_string()
        } else {
            "bad".to_string()
        })
    }
}

fn fake_java_asset(
    archive_format: ManagedArchiveFormat,
    java_relative_path: &'static str,
) -> ManagedJavaAsset {
    ManagedJavaAsset {
        url: "https://example.test/temurin-jre.test",
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        archive_format,
        java_relative_path,
    }
}

fn fake_tar_gz(path: &str, body: &[u8]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    archive
        .append_data(&mut header, path, Cursor::new(body))
        .unwrap();
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn fake_tar_symlink(path: &str, target: &str) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o777);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_path(path).unwrap();
    header.set_link_name(target).unwrap();
    header.set_cksum();
    archive.append(&header, std::io::empty()).unwrap();
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn fake_tar_with_safe_symlink() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);

    let body = b"license text";
    let mut file = tar::Header::new_gnu();
    file.set_size(body.len() as u64);
    file.set_mode(0o644);
    file.set_cksum();
    archive
        .append_data(
            &mut file,
            "fake-jre/legal/java.base/LICENSE",
            Cursor::new(body),
        )
        .unwrap();

    let mut link = tar::Header::new_gnu();
    link.set_size(0);
    link.set_mode(0o777);
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_path("fake-jre/legal/java.rmi/LICENSE").unwrap();
    link.set_link_name("../java.base/LICENSE").unwrap();
    link.set_cksum();
    archive.append(&link, std::io::empty()).unwrap();

    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn fake_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let output = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    for (path, body) in entries {
        archive.start_file(*path, options).unwrap();
        archive.write_all(body).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[cfg(unix)]
fn platform_command(script: &str) -> CommandSpec {
    let script = if script == "sleep 2" {
        "while :; do :; done"
    } else {
        script
    };
    CommandSpec {
        program: PathBuf::from("/bin/sh"),
        args: vec![OsString::from("-c"), OsString::from(script)],
        env: vec![],
        env_remove: vec![],
    }
}

#[cfg(unix)]
fn platform_security_env_command() -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("/bin/sh"),
        args: vec![
            OsString::from("-c"),
            OsString::from("printf %s \"$PLANTUML_SECURITY_PROFILE\""),
        ],
        env: vec![(
            OsString::from("PLANTUML_SECURITY_PROFILE"),
            OsString::from("INTERNET"),
        )],
        env_remove: vec![OsString::from("PLANTUML_SECURITY_PROFILE")],
    }
}

#[cfg(windows)]
fn platform_command(script: &str) -> CommandSpec {
    let script = if script == "sleep 2" {
        "ping -n 3 127.0.0.1 >NUL"
    } else {
        script
    };
    CommandSpec {
        program: PathBuf::from("cmd.exe"),
        args: vec![OsString::from("/C"), OsString::from(script)],
        env: vec![],
        env_remove: vec![],
    }
}

#[cfg(windows)]
fn platform_security_env_command() -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("cmd.exe"),
        args: vec![
            OsString::from("/D"),
            OsString::from("/C"),
            OsString::from(
                "<NUL set /P _plantuml_export_probe=%PLANTUML_SECURITY_PROFILE% & exit /B 0",
            ),
        ],
        env: vec![(
            OsString::from("PLANTUML_SECURITY_PROFILE"),
            OsString::from("INTERNET"),
        )],
        env_remove: vec![OsString::from("PLANTUML_SECURITY_PROFILE")],
    }
}
