// Copyright 2012-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use errors::{Error, ErrorKind};
use rustc_serialize::json;
use std::str::FromStr;

// These structs are a subset of the ones found in
// `syntax::errors::json`.

#[derive(RustcEncodable, RustcDecodable)]
struct Diagnostic {
    message: String,
    code: Option<DiagnosticCode>,
    level: String,
    spans: Vec<DiagnosticSpan>,
    children: Vec<Diagnostic>,
    rendered: Option<String>,
}

#[derive(RustcEncodable, RustcDecodable, Clone)]
struct DiagnosticSpan {
    file_name: String,
    line_start: usize,
    line_end: usize,
    column_start: usize,
    column_end: usize,
    expansion: Option<Box<DiagnosticSpanMacroExpansion>>,
}

#[derive(RustcEncodable, RustcDecodable, Clone)]
struct DiagnosticSpanMacroExpansion {
    /// span where macro was applied to generate this code
    span: DiagnosticSpan,

    /// name of macro that was applied (e.g., "foo!" or "#[derive(Eq)]")
    macro_decl_name: String,
}

#[derive(RustcEncodable, RustcDecodable, Clone)]
struct DiagnosticCode {
    /// The code itself.
    code: String,
    /// An explanation for the code.
    explanation: Option<String>,
}

pub fn parse_output(file_name: &str, output: &str) -> Vec<Error> {
    output.lines()
          .flat_map(|line| parse_line(file_name, line))
          .collect()
}

fn parse_line(file_name: &str, line: &str) -> Vec<Error> {
    // The compiler sometimes intermingles non-JSON stuff into the
    // output.  This hack just skips over such lines. Yuck.
    if line.chars().next() == Some('{') {
        match json::decode::<Diagnostic>(line) {
            Ok(diagnostic) => {
                let mut expected_errors = vec![];
                push_expected_errors(&mut expected_errors, &diagnostic, file_name);
                expected_errors
            }
            Err(error) => {
                println!("failed to decode compiler output as json: `{}`", error);
                panic!("failed to decode compiler output as json");
            }
        }
    } else {
        vec![]
    }
}

fn push_expected_errors(expected_errors: &mut Vec<Error>,
                        diagnostic: &Diagnostic,
                        file_name: &str) {
    // We only consider messages pertaining to the current file.
    let matching_spans =
        || diagnostic.spans.iter().filter(|span| span.file_name == file_name);
    let with_code =
        |span: &DiagnosticSpan, text: &str| match diagnostic.code {
            Some(ref code) =>
                // FIXME(#33000) -- it'd be better to use a dedicated
                // UI harness than to include the line/col number like
                // this, but some current tests rely on it.
                //
                // Note: Do NOT include the filename. These can easily
                // cause false matches where the expected message
                // appears in the filename, and hence the message
                // changes but the test still passes.
                format!("{}:{}: {}:{}: {} [{}]",
                        span.line_start, span.column_start,
                        span.line_end, span.column_end,
                        text, code.code.clone()),
            None =>
                // FIXME(#33000) -- it'd be better to use a dedicated UI harness
                format!("{}:{}: {}:{}: {}",
                        span.line_start, span.column_start,
                        span.line_end, span.column_end,
                        text),
        };

    // Convert multi-line messages into multiple expected
    // errors. We expect to replace these with something
    // more structured shortly anyhow.
    let mut message_lines = diagnostic.message.lines();
    if let Some(first_line) = message_lines.next() {
        for span in matching_spans() {
            let msg = with_code(span, first_line);
            let kind = ErrorKind::from_str(&diagnostic.level).ok();
            expected_errors.push(
                Error {
                    line_num: span.line_start,
                    kind: kind,
                    msg: msg,
                }
            );
        }
    }
    for next_line in message_lines {
        for span in matching_spans() {
            expected_errors.push(
                Error {
                    line_num: span.line_start,
                    kind: None,
                    msg: with_code(span, next_line),
                }
            );
        }
    }

    // If the message has a suggestion, register that.
    if let Some(ref rendered) = diagnostic.rendered {
        let start_line = matching_spans().map(|s| s.line_start).min().expect("\
            every suggestion should have at least one span");
        for (index, line) in rendered.lines().enumerate() {
            expected_errors.push(
                Error {
                    line_num: start_line + index,
                    kind: Some(ErrorKind::Suggestion),
                    msg: line.to_string()
                }
            );
        }
    }

    // Add notes for the backtrace
    for span in matching_spans() {
        for frame in &span.expansion {
            push_backtrace(expected_errors,
                           frame,
                           file_name);
        }
    }

    // Flatten out the children.
    for child in &diagnostic.children {
        push_expected_errors(expected_errors, child, file_name);
    }
}

fn push_backtrace(expected_errors: &mut Vec<Error>,
                  expansion: &DiagnosticSpanMacroExpansion,
                  file_name: &str) {
    if expansion.span.file_name == file_name {
        expected_errors.push(
            Error {
                line_num: expansion.span.line_start,
                kind: Some(ErrorKind::Note),
                msg: format!("in this expansion of {}", expansion.macro_decl_name),
            }
        );
    }

    for previous_expansion in &expansion.span.expansion {
        push_backtrace(expected_errors, previous_expansion, file_name);
    }
}
