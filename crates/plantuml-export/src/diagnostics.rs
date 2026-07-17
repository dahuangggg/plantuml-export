use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

const SOURCE: &str = "plantuml";

pub fn structural_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let starts = directives(text, "@start");
    let ends = directives(text, "@end");

    if starts.len() > ends.len() {
        let unmatched = starts.last().expect("starts is not empty");
        let start = offset_to_position(text, unmatched.offset);
        diagnostics.push(diagnostic(
            Range::new(
                start,
                Position::new(start.line, start.character + unmatched.length as u32),
            ),
            DiagnosticSeverity::ERROR,
            "Missing matching @end directive.",
        ));
    }

    if starts.is_empty() && !text.trim().is_empty() {
        let first_line = text.split_once('\n').map_or(text, |(line, _)| line);
        let length = first_line.encode_utf16().count().min(80) as u32;
        diagnostics.push(diagnostic(
            Range::new(Position::new(0, 0), Position::new(0, length)),
            DiagnosticSeverity::WARNING,
            "PlantUML files should start with an @start directive.",
        ));
    }

    if !starts.is_empty()
        && starts.len() == ends.len()
        && without_directives(text, &starts, &ends).trim().is_empty()
    {
        let first = starts.first().expect("starts is not empty");
        let last = ends.last().expect("balanced directives contain an end");
        diagnostics.push(diagnostic(
            Range::new(
                offset_to_position(text, first.offset),
                offset_to_position(text, last.offset + last.length),
            ),
            DiagnosticSeverity::WARNING,
            "PlantUML diagram has no body.",
        ));
    }

    diagnostics
}

pub fn parse_standard_report(output: &str) -> Vec<Diagnostic> {
    let lines: Vec<&str> = output.lines().collect();
    let mut diagnostics = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let Some(line_number) = report_line_number(line.trim()) else {
            continue;
        };
        let message = lines
            .iter()
            .skip(index + 1)
            .map(|line| line.trim())
            .take_while(|line| report_line_number(line).is_none())
            .find(|line| !line.is_empty())
            .unwrap_or("PlantUML syntax error");
        let line = line_number.saturating_sub(1) as u32;
        diagnostics.push(diagnostic(
            Range::new(Position::new(line, 0), Position::new(line, 1)),
            DiagnosticSeverity::ERROR,
            message,
        ));
    }

    diagnostics
}

fn report_line_number(line: &str) -> Option<usize> {
    let lower = line.to_ascii_lowercase();
    let rest = lower.strip_prefix("error line ")?;
    let (number, _) = rest.split_once(" in file:")?;
    number.trim().parse().ok()
}

fn diagnostic(range: Range, severity: DiagnosticSeverity, message: &str) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
        source: Some(SOURCE.to_string()),
        message: message.to_string(),
        ..Diagnostic::default()
    }
}

#[derive(Clone, Copy, Debug)]
struct Directive {
    offset: usize,
    length: usize,
}

fn directives(text: &str, prefix: &str) -> Vec<Directive> {
    text.match_indices(prefix)
        .map(|(offset, _)| {
            let suffix = &text[offset + prefix.len()..];
            let identifier_length = suffix
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count();
            Directive {
                offset,
                length: prefix.len() + identifier_length,
            }
        })
        .collect()
}

fn without_directives(text: &str, starts: &[Directive], ends: &[Directive]) -> String {
    let mut directives: Vec<Directive> = starts.iter().chain(ends).copied().collect();
    directives.sort_by_key(|directive| directive.offset);

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for directive in directives {
        result.push_str(&text[cursor..directive.offset]);
        cursor = directive.offset + directive.length;
    }
    result.push_str(&text[cursor..]);
    result
}

fn offset_to_position(text: &str, offset: usize) -> Position {
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let last_line = prefix.rsplit_once('\n').map_or(prefix, |(_, line)| line);
    let character = last_line.encode_utf16().count() as u32;
    Position::new(line, character)
}
