//! Formatter-facing `.editorconfig` resolution.
//!
//! The resolver is independent of filesystem access so the native CLI and the
//! WebAssembly language server share the exact same property and glob
//! semantics. Frontends only collect ancestor files and provide each target
//! path relative to the directory containing that file.

use crate::formatter::{FormatOptions, IndentStyle, LineEnding};

/// One ancestor `.editorconfig` and the formatted file's path relative to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfigLayer {
    pub relative_path: String,
    pub source: String,
}

#[derive(Debug, Default)]
struct ParsedEditorConfig<'a> {
    root: bool,
    sections: Vec<Section<'a>>,
}

#[derive(Debug)]
struct Section<'a> {
    pattern: &'a str,
    properties: Vec<(&'a str, &'a str)>,
}

/// Applies matching files from the outermost relevant ancestor to the nearest.
///
/// `layers` must be ordered from the target's directory toward the filesystem
/// root. A `root = true` preamble prevents more distant files from applying.
pub fn resolve(mut options: FormatOptions, layers: &[EditorConfigLayer]) -> FormatOptions {
    if layers.is_empty() {
        return options;
    }
    let parsed = layers
        .iter()
        .map(|layer| parse(&layer.source))
        .collect::<Vec<_>>();
    let outermost = parsed
        .iter()
        .position(|config| config.root)
        .unwrap_or_else(|| layers.len().saturating_sub(1));
    let defaults = options;

    for index in (0..=outermost).rev() {
        let path = layers[index].relative_path.replace('\\', "/");
        for section in &parsed[index].sections {
            if matches_pattern(section.pattern, &path) {
                apply_properties(&mut options, defaults, &section.properties);
            }
        }
    }
    options
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_for_path(path: &std::path::Path, base: FormatOptions) -> FormatOptions {
    use std::{fs, path::PathBuf};

    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut directory = target.parent().map(std::path::Path::to_path_buf);
    let mut layers = Vec::new();
    while let Some(current) = directory {
        let config_path = current.join(".editorconfig");
        if let Ok(source) = fs::read_to_string(&config_path) {
            let relative_path = target
                .strip_prefix(&current)
                .unwrap_or(target.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let is_root = parse(&source).root;
            layers.push(EditorConfigLayer {
                relative_path,
                source,
            });
            if is_root {
                break;
            }
        }
        directory = current.parent().map(PathBuf::from);
    }
    resolve(base, &layers)
}

fn parse(source: &str) -> ParsedEditorConfig<'_> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut parsed = ParsedEditorConfig::default();
    let mut current_section = None;
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(pattern) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            parsed.sections.push(Section {
                pattern: pattern.trim(),
                properties: Vec::new(),
            });
            current_section = Some(parsed.sections.len() - 1);
            continue;
        }
        let Some((key, value)) = line.split_once(['=', ':']) else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if let Some(index) = current_section {
            parsed.sections[index].properties.push((key, value));
        } else if key.eq_ignore_ascii_case("root") {
            parsed.root = value.eq_ignore_ascii_case("true");
        }
    }
    parsed
}

fn apply_properties(
    options: &mut FormatOptions,
    defaults: FormatOptions,
    properties: &[(&str, &str)],
) {
    let property = |name: &str| {
        properties
            .iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    };

    if let Some(value) = property("indent_style") {
        options.indent_style = if value.eq_ignore_ascii_case("unset") {
            defaults.indent_style
        } else if value.eq_ignore_ascii_case("tab") {
            IndentStyle::Tabs
        } else if value.eq_ignore_ascii_case("space") {
            IndentStyle::Spaces
        } else {
            options.indent_style
        };
    }
    if let Some(value) = property("indent_size") {
        options.indent_width = if value.eq_ignore_ascii_case("unset") {
            defaults.indent_width
        } else if value.eq_ignore_ascii_case("tab") {
            property("tab_width")
                .and_then(parse_positive)
                .unwrap_or(options.indent_width)
        } else {
            parse_positive(value).unwrap_or(options.indent_width)
        };
    } else if options.indent_style == IndentStyle::Tabs
        && let Some(width) = property("tab_width").and_then(parse_positive)
    {
        options.indent_width = width;
    }
    if let Some(value) = property("max_line_length") {
        options.max_line_width = if value.eq_ignore_ascii_case("unset") {
            defaults.max_line_width
        } else if value.eq_ignore_ascii_case("off") {
            usize::MAX
        } else {
            parse_positive(value).unwrap_or(options.max_line_width)
        };
    }
    if let Some(value) = property("end_of_line") {
        options.line_ending = if value.eq_ignore_ascii_case("unset") {
            defaults.line_ending
        } else if value.eq_ignore_ascii_case("crlf") {
            LineEnding::CrLf
        } else if value.eq_ignore_ascii_case("cr") {
            LineEnding::Cr
        } else if value.eq_ignore_ascii_case("lf") {
            LineEnding::Lf
        } else {
            options.line_ending
        };
    }
    if let Some(value) = property("insert_final_newline") {
        options.insert_final_newline = if value.eq_ignore_ascii_case("unset") {
            defaults.insert_final_newline
        } else if value.eq_ignore_ascii_case("true") {
            true
        } else if value.eq_ignore_ascii_case("false") {
            false
        } else {
            options.insert_final_newline
        };
    }
}

fn parse_positive(value: &str) -> Option<usize> {
    value.parse().ok().filter(|value| *value > 0)
}

fn matches_pattern(pattern: &str, relative_path: &str) -> bool {
    expand_braces(pattern).iter().any(|pattern| {
        let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
        let candidate = if pattern.contains('/') {
            relative_path
        } else {
            relative_path.rsplit('/').next().unwrap_or(relative_path)
        };
        glob_matches(pattern.as_bytes(), candidate.as_bytes())
    })
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let Some(relative_close) = pattern[open + 1..].find('}') else {
        return vec![pattern.to_owned()];
    };
    let close = open + 1 + relative_close;
    let choices = pattern[open + 1..close].split(',').collect::<Vec<_>>();
    if choices.len() == 1 {
        return vec![pattern.to_owned()];
    }
    choices
        .into_iter()
        .flat_map(|choice| {
            expand_braces(&format!(
                "{}{}{}",
                &pattern[..open],
                choice,
                &pattern[close + 1..]
            ))
        })
        .collect()
}

fn glob_matches(pattern: &[u8], candidate: &[u8]) -> bool {
    fn recurse(
        pattern: &[u8],
        candidate: &[u8],
        pattern_index: usize,
        candidate_index: usize,
        memo: &mut [Vec<Option<bool>>],
    ) -> bool {
        if let Some(result) = memo[pattern_index][candidate_index] {
            return result;
        }
        let result = if pattern_index == pattern.len() {
            candidate_index == candidate.len()
        } else if pattern[pattern_index..].starts_with(b"**") {
            let mut next = pattern_index + 2;
            if pattern.get(next) == Some(&b'/') {
                next += 1;
            }
            recurse(pattern, candidate, next, candidate_index, memo)
                || candidate_index < candidate.len()
                    && recurse(pattern, candidate, pattern_index, candidate_index + 1, memo)
        } else if pattern[pattern_index] == b'*' {
            recurse(pattern, candidate, pattern_index + 1, candidate_index, memo)
                || candidate
                    .get(candidate_index)
                    .is_some_and(|byte| *byte != b'/')
                    && recurse(pattern, candidate, pattern_index, candidate_index + 1, memo)
        } else if pattern[pattern_index] == b'?' {
            candidate
                .get(candidate_index)
                .is_some_and(|byte| *byte != b'/')
                && recurse(
                    pattern,
                    candidate,
                    pattern_index + 1,
                    candidate_index + 1,
                    memo,
                )
        } else {
            candidate.get(candidate_index) == Some(&pattern[pattern_index])
                && recurse(
                    pattern,
                    candidate,
                    pattern_index + 1,
                    candidate_index + 1,
                    memo,
                )
        };
        memo[pattern_index][candidate_index] = Some(result);
        result
    }

    let mut memo = vec![vec![None; candidate.len() + 1]; pattern.len() + 1];
    recurse(pattern, candidate, 0, 0, &mut memo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearer_matching_sections_override_ancestors_until_root() {
        let layers = [
            EditorConfigLayer {
                relative_path: "game.split".into(),
                source: "[*.split]\nindent_size = 2\nend_of_line = crlf".into(),
            },
            EditorConfigLayer {
                relative_path: "scripts/game.split".into(),
                source: "root = true\n[*]\nindent_style = tab\nmax_line_length = 80".into(),
            },
            EditorConfigLayer {
                relative_path: "project/scripts/game.split".into(),
                source: "[*]\nmax_line_length = 40".into(),
            },
        ];
        let resolved = resolve(FormatOptions::default(), &layers);
        assert_eq!(resolved.indent_style, IndentStyle::Tabs);
        assert_eq!(resolved.indent_width, 2);
        assert_eq!(resolved.max_line_width, 80);
        assert_eq!(resolved.line_ending, LineEnding::CrLf);
    }

    #[test]
    fn slashless_patterns_match_the_file_name_and_double_star_matches_directories() {
        assert!(matches_pattern("*.split", "nested/game.split"));
        assert!(matches_pattern(
            "scripts/**/game.{split,txt}",
            "scripts/a/b/game.split"
        ));
        assert!(!matches_pattern("scripts/*.split", "scripts/a/game.split"));
    }
}
