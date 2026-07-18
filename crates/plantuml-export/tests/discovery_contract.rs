use std::fs;
use std::path::PathBuf;

use plantuml_export::discovery::{discover_inputs, DiscoveryErrorKind, DiscoveryOptions};
use tempfile::TempDir;

fn write(path: impl Into<PathBuf>, contents: &str) {
    let path = path.into();
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
    fs::write(path, contents).expect("write fixture");
}

#[test]
fn explicit_inputs_accept_only_supported_files_inside_the_root() {
    let project = TempDir::new().expect("project");
    let outside = TempDir::new().expect("outside");
    write(
        project.path().join("diagrams/one.puml"),
        "@startuml\n@enduml\n",
    );
    write(
        project.path().join("diagrams/two.PU"),
        "@startuml\n@enduml\n",
    );
    write(project.path().join("notes.md"), "not a standalone source");
    write(outside.path().join("escape.puml"), "@startuml\n@enduml\n");

    let mut options = DiscoveryOptions::new(project.path());
    options.inputs = vec![
        PathBuf::from("diagrams/two.PU"),
        PathBuf::from("diagrams/one.puml"),
    ];
    let inputs = discover_inputs(&options).expect("supported explicit inputs");
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.relative_path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>(),
        ["diagrams/one.puml", "diagrams/two.PU"]
    );

    options.inputs = vec![PathBuf::from("notes.md")];
    let unsupported = discover_inputs(&options).expect_err("markdown must be rejected");
    assert_eq!(unsupported.kind, DiscoveryErrorKind::UnsupportedInput);

    options.inputs = vec![outside.path().join("escape.puml")];
    let escaped = discover_inputs(&options).expect_err("outside input must be rejected");
    assert_eq!(escaped.kind, DiscoveryErrorKind::UnsafePath);
}

#[test]
fn explicit_inputs_bypass_workspace_ignores_but_not_protected_directories() {
    let project = TempDir::new().expect("project");
    write(project.path().join(".gitignore"), "ignored/\n");
    write(project.path().join("ignored/explicit.puml"), "explicit");
    write(project.path().join("out/generated.puml"), "generated");
    write(project.path().join("cache/cached.puml"), "cached");
    write(project.path().join(".git/internal.puml"), "internal");

    let mut options = DiscoveryOptions::new(project.path());
    options.inputs = vec![PathBuf::from("ignored/explicit.puml")];
    let inputs = discover_inputs(&options).expect("explicit ignored input");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].relative_path,
        PathBuf::from("ignored/explicit.puml")
    );

    options.cache_dir = Some(PathBuf::from("cache"));
    for protected in [
        "out/generated.puml",
        "cache/cached.puml",
        ".git/internal.puml",
    ] {
        options.inputs = vec![PathBuf::from(protected)];
        let error = discover_inputs(&options).expect_err("protected input must fail");
        assert_eq!(error.kind, DiscoveryErrorKind::InvalidInput);
    }
}

#[test]
fn gitignore_is_honored_without_a_git_repository() {
    let project = TempDir::new().expect("project");
    write(project.path().join(".gitignore"), "ignored/\n");
    write(project.path().join("ignored/hidden.puml"), "ignored");
    write(project.path().join("visible.puml"), "visible");

    let mut options = DiscoveryOptions::new(project.path());
    options.workspace = true;
    let inputs = discover_inputs(&options).expect("plain-directory discovery");

    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].relative_path, PathBuf::from("visible.puml"));
}

#[test]
fn workspace_discovery_is_ignore_aware_pruned_filtered_and_sorted() {
    let project = TempDir::new().expect("project");
    fs::create_dir(project.path().join(".git")).expect("git dir");
    write(project.path().join(".gitignore"), "ignored/\n*.wsd\n");
    write(project.path().join(".ignore"), "private/\n");
    write(project.path().join("z-last.puml"), "z");
    write(project.path().join("a-first.plantuml"), "a");
    write(project.path().join("docs/keep.iuml"), "keep");
    write(project.path().join("docs/excluded.pu"), "exclude");
    write(project.path().join("ignored/by-gitignore.puml"), "ignored");
    write(project.path().join("private/by-ignore.puml"), "ignored");
    write(project.path().join("ignored-by-suffix.wsd"), "ignored");
    write(project.path().join("out/loop.puml"), "output");
    write(
        project.path().join(".plantuml-export-cache/cache.puml"),
        "cache",
    );
    write(project.path().join(".git/internal.puml"), "git internals");
    write(project.path().join("README.md"), "not PlantUML");

    let mut options = DiscoveryOptions::new(project.path());
    options.workspace = true;
    options.out_dir = PathBuf::from("out");
    options.cache_dir = Some(PathBuf::from(".plantuml-export-cache"));
    options.include = vec!["**/*".into()];
    options.exclude = vec!["docs/excluded.*".into()];

    let inputs = discover_inputs(&options).expect("workspace discovery");
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.relative_path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>(),
        ["a-first.plantuml", "docs/keep.iuml", "z-last.puml"]
    );
}

#[test]
fn include_globs_limit_workspace_discovery() {
    let project = TempDir::new().expect("project");
    write(project.path().join("architecture/system.puml"), "system");
    write(project.path().join("sequence/login.puml"), "login");

    let mut options = DiscoveryOptions::new(project.path());
    options.workspace = true;
    options.include = vec!["sequence/**".into()];

    let inputs = discover_inputs(&options).expect("filtered discovery");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].relative_path,
        PathBuf::from("sequence/login.puml")
    );
}

#[test]
fn explicit_and_workspace_inputs_are_deduplicated_by_canonical_identity() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");

    let mut options = DiscoveryOptions::new(project.path());
    options.workspace = true;
    options.inputs = vec![PathBuf::from("diagram.puml")];

    let inputs = discover_inputs(&options).expect("combined discovery");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].relative_path, PathBuf::from("diagram.puml"));
}

#[test]
fn invalid_globs_and_project_path_escapes_are_rejected_before_walking() {
    let project = TempDir::new().expect("project");
    let outside = TempDir::new().expect("outside");

    let mut invalid_glob = DiscoveryOptions::new(project.path());
    invalid_glob.workspace = true;
    invalid_glob.include = vec!["[unterminated".into()];
    let error = discover_inputs(&invalid_glob).expect_err("invalid glob must fail");
    assert_eq!(error.kind, DiscoveryErrorKind::InvalidPattern);

    let mut relative_escape = DiscoveryOptions::new(project.path());
    relative_escape.workspace = true;
    relative_escape.out_dir = PathBuf::from("../outside");
    let error = discover_inputs(&relative_escape).expect_err("relative output escape must fail");
    assert_eq!(error.kind, DiscoveryErrorKind::UnsafePath);

    let mut absolute_escape = DiscoveryOptions::new(project.path());
    absolute_escape.workspace = true;
    absolute_escape.out_dir = outside.path().join("output");
    let error = discover_inputs(&absolute_escape).expect_err("absolute output escape must fail");
    assert_eq!(error.kind, DiscoveryErrorKind::UnsafePath);

    let mut base_escape = DiscoveryOptions::new(project.path());
    base_escape.input_base = outside.path().to_path_buf();
    let error = discover_inputs(&base_escape).expect_err("input base escape must fail");
    assert_eq!(error.kind, DiscoveryErrorKind::UnsafePath);
}

#[test]
fn empty_discovery_is_a_noop_unless_input_is_required() {
    let project = TempDir::new().expect("project");
    let options = DiscoveryOptions::new(project.path());
    assert!(discover_inputs(&options)
        .expect("interactive no-op")
        .is_empty());

    let mut required = DiscoveryOptions::new(project.path());
    required.require_input = true;
    let error = discover_inputs(&required).expect_err("automation must fail empty discovery");
    assert_eq!(error.kind, DiscoveryErrorKind::NoInputs);

    write(project.path().join("would-be-output.puml"), "generated");
    let mut output_is_root = DiscoveryOptions::new(project.path());
    output_is_root.workspace = true;
    output_is_root.out_dir = PathBuf::from(".");
    assert!(discover_inputs(&output_is_root)
        .expect("root output prunes entire workspace")
        .is_empty());
}

#[cfg(unix)]
#[test]
fn workspace_discovery_does_not_follow_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let project = TempDir::new().expect("project");
    let outside = TempDir::new().expect("outside");
    write(outside.path().join("external.puml"), "external");
    symlink(outside.path(), project.path().join("linked")).expect("directory symlink");
    write(project.path().join("local.puml"), "local");

    let mut options = DiscoveryOptions::new(project.path());
    options.workspace = true;
    let inputs = discover_inputs(&options).expect("workspace discovery");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].relative_path, PathBuf::from("local.puml"));

    symlink(
        outside.path().join("external.puml"),
        project.path().join("explicit-link.puml"),
    )
    .expect("file symlink");
    let mut explicit = DiscoveryOptions::new(project.path());
    explicit.inputs = vec![PathBuf::from("explicit-link.puml")];
    let error = discover_inputs(&explicit).expect_err("explicit symlink escape must fail");
    assert_eq!(error.kind, DiscoveryErrorKind::UnsafePath);

    symlink(outside.path(), project.path().join("escaped-output")).expect("output symlink");
    let mut escaped_output = DiscoveryOptions::new(project.path());
    escaped_output.workspace = true;
    escaped_output.out_dir = PathBuf::from("escaped-output");
    let error = discover_inputs(&escaped_output).expect_err("output symlink escape must fail");
    assert_eq!(error.kind, DiscoveryErrorKind::UnsafePath);
}
