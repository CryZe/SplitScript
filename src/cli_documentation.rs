use std::io;

use codespan_reporting::term::termcolor::{Color, ColorSpec, WriteColor};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const SEARCH_RESULT_LIMIT: usize = 12;

#[derive(Debug)]
enum Block {
    Heading {
        level: HeadingLevel,
        text: String,
    },
    Paragraph {
        quote_depth: usize,
        text: String,
    },
    ListItem {
        quote_depth: usize,
        depth: usize,
        marker: String,
        text: String,
    },
    Code(String),
    Table(Vec<Vec<String>>),
    Rule,
}

#[derive(Debug)]
struct ListState {
    next: Option<u64>,
}

#[derive(Debug)]
struct ItemState {
    depth: usize,
    marker: String,
    text: String,
}

#[derive(Debug, Default)]
struct TableState {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: Option<String>,
    in_header: bool,
    suppress_header: bool,
}

#[derive(Debug, Default)]
struct DocumentBuilder {
    blocks: Vec<Block>,
    text: String,
    heading: Option<HeadingLevel>,
    code: Option<String>,
    lists: Vec<ListState>,
    items: Vec<ItemState>,
    table: Option<TableState>,
    quote_depth: usize,
    suppress_next_table_header: bool,
}

impl DocumentBuilder {
    fn parse(markdown: &str) -> Vec<Block> {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TASKLISTS);

        let mut builder = Self::default();
        for event in Parser::new_ext(markdown, options) {
            builder.event(event);
        }
        builder.blocks
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text)
            | Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => self.push_text(&text),
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => self.blocks.push(Block::Rule),
            Event::TaskListMarker(checked) => {
                self.push_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if html.contains("splitscript-reference-table") {
                    self.suppress_next_table_header = true;
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.items.is_empty() && self.table.is_none() {
                    self.text.clear();
                } else if self.items.last().is_some_and(|item| !item.text.is_empty()) {
                    self.push_text(" ");
                }
            }
            Tag::Heading { level, .. } => {
                self.text.clear();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::CodeBlock(_) => self.code = Some(String::new()),
            Tag::List(start) => self.lists.push(ListState { next: start }),
            Tag::Item => {
                let list = self
                    .lists
                    .last_mut()
                    .expect("Markdown list items are nested inside a list");
                let marker = match &mut list.next {
                    Some(next) => {
                        let marker = format!("{next}.");
                        *next += 1;
                        marker
                    }
                    None => "-".to_owned(),
                };
                self.items.push(ItemState {
                    depth: self.lists.len().saturating_sub(1),
                    marker,
                    text: String::new(),
                });
            }
            Tag::Table(_) => {
                self.table = Some(TableState {
                    suppress_header: std::mem::take(&mut self.suppress_next_table_header),
                    ..TableState::default()
                });
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_header = true;
                }
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(table) = &mut self.table {
                    table.cell = Some(String::new());
                }
            }
            Tag::HtmlBlock
            | Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
            | Tag::FootnoteDefinition(_)
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.items.is_empty() && self.table.is_none() && self.heading.is_none() {
                    let text = take_normalized(&mut self.text);
                    if !text.is_empty() {
                        self.blocks.push(Block::Paragraph {
                            quote_depth: self.quote_depth,
                            text,
                        });
                    }
                }
            }
            TagEnd::Heading(_) => {
                let level = self.heading.take().expect("heading start precedes its end");
                let text = take_normalized(&mut self.text);
                self.blocks.push(Block::Heading { level, text });
            }
            TagEnd::BlockQuote(_) => self.quote_depth = self.quote_depth.saturating_sub(1),
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    self.blocks
                        .push(Block::Code(code.trim_end_matches('\n').to_owned()));
                }
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => {
                if let Some(mut item) = self.items.pop() {
                    let text = take_normalized(&mut item.text);
                    self.blocks.push(Block::ListItem {
                        quote_depth: self.quote_depth,
                        depth: item.depth,
                        marker: item.marker,
                        text,
                    });
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table
                    && let Some(mut cell) = table.cell.take()
                {
                    table.row.push(take_normalized(&mut cell));
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table
                    && !(table.in_header && table.suppress_header)
                {
                    table.rows.push(std::mem::take(&mut table.row));
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_header = false;
                    table.row.clear();
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take()
                    && !table.rows.is_empty()
                {
                    self.blocks.push(Block::Table(table.rows));
                }
            }
            TagEnd::HtmlBlock
            | TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
        } else if let Some(table) = &mut self.table
            && let Some(cell) = &mut table.cell
        {
            cell.push_str(text);
        } else if let Some(item) = self.items.last_mut() {
            item.text.push_str(text);
        } else {
            self.text.push_str(text);
        }
    }
}

fn take_normalized(text: &mut String) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.clear();
    normalized
}

/// Renders compiler-owned Markdown as terminal-native text.
///
/// Links intentionally collapse to their labels because the documentation
/// graph uses editor-only virtual paths. `WriteColor` decides whether styling
/// becomes ANSI escapes, so redirected output remains stable plain text.
pub(crate) fn emit(writer: &mut dyn WriteColor, markdown: &str, width: usize) -> io::Result<()> {
    let width = width.max(20);
    let blocks = DocumentBuilder::parse(markdown);
    for (index, block) in blocks.iter().enumerate() {
        if index != 0 {
            writeln!(writer)?;
        }
        render_block(writer, block, width)?;
    }
    Ok(())
}

pub(crate) fn search_results_markdown(
    query: &str,
    results: &[splitscript::DocumentationIndexEntry],
) -> String {
    let mut markdown = format!(
        "# Documentation results for `{}`\n\n\
         Open a result with `splitc docs` followed by its exact topic.\n\n\
         <div class=\"splitscript-reference-table\"></div>\n\n\
         | Topic | Kind | Description |\n\
         | --- | --- | --- |",
        escape_table_cell(query),
    );
    for result in results.iter().take(SEARCH_RESULT_LIMIT) {
        markdown.push_str(&format!(
            "\n| {} | {} | {} |",
            escape_table_cell(&result.title),
            escape_table_cell(result.kind),
            escape_table_cell(&result.summary),
        ));
    }
    if results.len() > SEARCH_RESULT_LIMIT {
        markdown.push_str(&format!(
            "\n\nShowing the first {SEARCH_RESULT_LIMIT} of {} matches. Add more search terms to narrow the results.",
            results.len(),
        ));
    }
    markdown
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn render_block(writer: &mut dyn WriteColor, block: &Block, width: usize) -> io::Result<()> {
    match block {
        Block::Heading { level, text } => {
            let mut style = ColorSpec::new();
            style.set_bold(true).set_fg(Some(match level {
                HeadingLevel::H1 => Color::Cyan,
                _ => Color::Blue,
            }));
            writer.set_color(&style)?;
            writeln!(writer, "{text}")?;
            writer.reset()
        }
        Block::Paragraph { quote_depth, text } => {
            let prefix = "> ".repeat(*quote_depth);
            write_wrapped(writer, text, &prefix, &prefix, width)
        }
        Block::ListItem {
            quote_depth,
            depth,
            marker,
            text,
        } => {
            let quote = "> ".repeat(*quote_depth);
            let initial = format!("{quote}{}{marker} ", "  ".repeat(*depth));
            let subsequent = " ".repeat(initial.chars().count());
            write_wrapped(writer, text, &initial, &subsequent, width)
        }
        Block::Code(code) => {
            let mut style = ColorSpec::new();
            style.set_fg(Some(Color::Green));
            writer.set_color(&style)?;
            for line in code.lines() {
                writeln!(writer, "  {line}")?;
            }
            writer.reset()
        }
        Block::Table(rows) => render_table(writer, rows, width),
        Block::Rule => writeln!(writer, "{}", "─".repeat(width.min(80))),
    }
}

fn write_wrapped(
    writer: &mut dyn WriteColor,
    text: &str,
    initial: &str,
    subsequent: &str,
    width: usize,
) -> io::Result<()> {
    let options = textwrap::Options::new(width)
        .initial_indent(initial)
        .subsequent_indent(subsequent);
    for line in textwrap::wrap(text, options) {
        writeln!(writer, "{line}")?;
    }
    Ok(())
}

fn render_table(writer: &mut dyn WriteColor, rows: &[Vec<String>], width: usize) -> io::Result<()> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return Ok(());
    }
    let spacing = 2;
    let available = width.saturating_sub(spacing * columns.saturating_sub(1));
    let mut widths = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect::<Vec<_>>();
    let minimum = (available / columns).min(12).max(4);
    while widths.iter().sum::<usize>() > available {
        let Some((column, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > minimum)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[column] -= 1;
    }

    for row in rows {
        let wrapped = (0..columns)
            .map(|column| {
                let cell = row.get(column).map(String::as_str).unwrap_or("");
                textwrap::wrap(cell, widths[column].max(1))
                    .into_iter()
                    .map(|line| line.into_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line in 0..height {
            let last_column = (0..columns)
                .rfind(|column| {
                    wrapped[*column]
                        .get(line)
                        .is_some_and(|cell| !cell.is_empty())
                })
                .unwrap_or(0);
            for column in 0..=last_column {
                if column != 0 {
                    write!(writer, "{}", " ".repeat(spacing))?;
                }
                let cell = wrapped[column].get(line).map(String::as_str).unwrap_or("");
                if column == 0 && !cell.is_empty() {
                    let mut style = ColorSpec::new();
                    style.set_fg(Some(Color::Cyan));
                    writer.set_color(&style)?;
                    write!(writer, "{cell}")?;
                    writer.reset()?;
                } else {
                    write!(writer, "{cell}")?;
                }
                if column != last_column {
                    write!(
                        writer,
                        "{}",
                        " ".repeat(widths[column].saturating_sub(cell.chars().count()))
                    )?;
                }
            }
            writeln!(writer)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codespan_reporting::term::termcolor::Buffer;

    #[test]
    fn renders_markdown_without_links_html_fences_or_table_borders() {
        let markdown = concat!(
            "[SplitScript reference](../index.md) / Process\n\n",
            "# Process.read\n\n",
            "Reads a [`value`](virtual.md) from memory.\n\n",
            "<div class=\"splitscript-reference-table\"></div>\n\n",
            "| Member | Description |\n",
            "| --- | --- |\n",
            "| `read` | Reads memory without changing the process. |\n\n",
            "```splitscript\nlet value = process.read<u32>(0x1000)\n```\n",
        );
        let mut buffer = Buffer::no_color();
        emit(&mut buffer, markdown, 72).unwrap();
        let rendered = String::from_utf8(buffer.into_inner()).unwrap();

        assert!(rendered.contains("SplitScript reference / Process"));
        assert!(rendered.contains("Process.read"));
        assert!(rendered.contains("read  Reads memory"));
        assert!(rendered.contains("  let value = process.read<u32>(0x1000)"));
        for unwanted in ["virtual.md", "<div", "| ---", "```", "\u{1b}["] {
            assert!(!rendered.contains(unwanted), "unexpected `{unwanted}`");
        }
        assert!(rendered.lines().all(|line| !line.ends_with(' ')));
    }

    #[test]
    fn emits_styles_only_when_the_writer_supports_color() {
        let mut plain = Buffer::no_color();
        emit(&mut plain, "# Heading\n", 80).unwrap();
        assert!(!plain.into_inner().contains(&0x1b));

        let mut ansi = Buffer::ansi();
        emit(&mut ansi, "# Heading\n", 80).unwrap();
        assert!(ansi.into_inner().contains(&0x1b));
    }

    #[test]
    fn renders_ranked_search_results_as_a_compact_borderless_table() {
        let reference = splitscript::DocumentationReference::for_terminal();
        let results = reference.search("multiple processes");
        let markdown = search_results_markdown("multiple processes", &results);
        let mut buffer = Buffer::no_color();
        emit(&mut buffer, &markdown, 100).unwrap();
        let rendered = String::from_utf8(buffer.into_inner()).unwrap();

        assert!(rendered.contains("Documentation results for multiple processes"));
        assert!(rendered.contains("Attachment state declaration"));
        assert!(!rendered.contains("Topic | Kind"));
        assert!(!rendered.contains("splitscript-reference-table"));
    }
}
