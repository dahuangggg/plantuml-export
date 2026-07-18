use lsp_types::{DiagnosticSeverity, Position, Range};
use plantuml_export::diagnostics::{parse_standard_report, structural_diagnostics};

#[test]
fn maps_standard_plantuml_reports_to_exact_lsp_lines() {
    let diagnostics = parse_standard_report(
        "Error line 3 in file: docs/bad.puml\nSome diagram description contains errors\n",
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].range,
        Range::new(Position::new(2, 0), Position::new(2, 1))
    );
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diagnostics[0].source.as_deref(), Some("plantuml"));
    assert_eq!(
        diagnostics[0].message,
        "Some diagram description contains errors"
    );
}

#[test]
fn prefers_the_structured_v1_label_over_the_generic_summary() {
    let diagnostics = parse_standard_report(
        "protocolVersion=1\nstatus=ERROR\nlineNumber=2\nlabel=Syntax Error? (Assumed diagram type: sequence)\nError line 2 in file: docs/bad.puml\nSome diagram description contains errors\n",
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].range.start, Position::new(1, 0));
    assert_eq!(
        diagnostics[0].message,
        "Syntax Error? (Assumed diagram type: sequence)"
    );
}

#[test]
fn ignores_successful_plantuml_reports() {
    assert!(parse_standard_report("File generation OK\n").is_empty());
}

#[test]
fn structural_diagnostics_match_the_previous_native_editor_contract() {
    let missing_end = structural_diagnostics("@startuml\nAlice -> Bob : hi\n");
    assert_eq!(missing_end.len(), 1);
    assert_eq!(missing_end[0].message, "Missing matching @end directive.");
    assert_eq!(
        missing_end[0].range,
        Range::new(Position::new(0, 0), Position::new(0, 9))
    );
    assert_eq!(missing_end[0].severity, Some(DiagnosticSeverity::ERROR));

    let no_start = structural_diagnostics("Alice -> Bob : hi\n");
    assert_eq!(no_start.len(), 1);
    assert_eq!(
        no_start[0].message,
        "PlantUML files should start with an @start directive."
    );
    assert_eq!(
        no_start[0].range,
        Range::new(Position::new(0, 0), Position::new(0, 17))
    );
    assert_eq!(no_start[0].severity, Some(DiagnosticSeverity::WARNING));

    let empty = structural_diagnostics("@startuml\n@enduml\n");
    assert_eq!(empty.len(), 1);
    assert_eq!(empty[0].message, "PlantUML diagram has no body.");
    assert_eq!(
        empty[0].range,
        Range::new(Position::new(0, 0), Position::new(1, 7))
    );
    assert_eq!(empty[0].severity, Some(DiagnosticSeverity::WARNING));
}

#[test]
fn structural_positions_use_utf16_code_units_like_lsp() {
    let diagnostics = structural_diagnostics("😀 @startuml\n");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].range.start, Position::new(0, 3));
}
