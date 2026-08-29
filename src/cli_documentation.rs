use std::io;

use codespan_reporting::term::termcolor::{
    Ansi, Color, ColorChoice, ColorSpec, StandardStream, WriteColor,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use splitscript::tooling::{database::CompilerDatabase, highlight::SemanticTokenKind};
use supports_color::Stream;

const SEARCH_RESULT_LIMIT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorDepth {
    Ansi16,
    Ansi256,
}

impl ColorDepth {
    fn stdout() -> Self {
        if supports_color::on_cached(Stream::Stdout).is_some_and(|support| support.has_256) {
            Self::Ansi256
        } else {
            Self::Ansi16
        }
    }

    const fn color(self, fallback: Color, ansi256: u8) -> Color {
        match self {
            Self::Ansi16 => fallback,
            Self::Ansi256 => Color::Ansi256(ansi256),
        }
    }
}

#[derive(Debug)]
enum Block {
    Heading {
        level: HeadingLevel,
        text: StyledText,
    },
    Paragraph {
        quote_depth: usize,
        text: StyledText,
    },
    ListItem {
        quote_depth: usize,
        depth: usize,
        marker: String,
        text: StyledText,
    },
    Code(Vec<CodeFragment>),
    Table(Vec<Vec<String>>),
    Rule,
}

#[derive(Debug)]
struct CodeFragment {
    text: String,
    kind: Option<SemanticTokenKind>,
}

#[derive(Debug)]
struct CodeState {
    source: String,
    is_splitscript: bool,
}

#[derive(Debug)]
struct ListState {
    next: Option<u64>,
}

#[derive(Debug)]
struct ItemState {
    depth: usize,
    marker: String,
    text: StyledText,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    code: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct InlineState {
    strong: usize,
    emphasis: usize,
    strikethrough: usize,
}

impl InlineState {
    fn style(self) -> InlineStyle {
        InlineStyle {
            bold: self.strong != 0,
            italic: self.emphasis != 0,
            strikethrough: self.strikethrough != 0,
            code: false,
        }
    }
}

#[derive(Debug, Clone)]
struct StyledChar {
    value: char,
    style: InlineStyle,
}

#[derive(Debug, Clone, Default)]
struct StyledText {
    chars: Vec<StyledChar>,
}

impl StyledText {
    fn push_str(&mut self, text: &str, style: InlineStyle) {
        self.chars
            .extend(text.chars().map(|value| StyledChar { value, style }));
    }

    fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    fn clear(&mut self) {
        self.chars.clear();
    }

    fn plain(&self) -> String {
        self.chars.iter().map(|character| character.value).collect()
    }
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
    text: StyledText,
    heading: Option<HeadingLevel>,
    code: Option<CodeState>,
    lists: Vec<ListState>,
    items: Vec<ItemState>,
    table: Option<TableState>,
    html_block: Option<String>,
    quote_depth: usize,
    suppress_next_table_header: bool,
    inline: InlineState,
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
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => self.push_text(&text),
            Event::Code(text) => {
                let mut style = self.inline.style();
                style.code = true;
                self.push_styled_text(&text, style);
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => self.blocks.push(Block::Rule),
            Event::TaskListMarker(checked) => {
                self.push_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::Html(html) => {
                if let Some(block) = &mut self.html_block {
                    block.push_str(&html);
                } else {
                    self.html(&html);
                }
            }
            Event::InlineHtml(html) => self.html(&html),
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
            Tag::CodeBlock(kind) => {
                let is_splitscript = matches!(
                    kind,
                    CodeBlockKind::Fenced(info)
                        if info.split_whitespace().next() == Some("splitscript")
                );
                self.code = Some(CodeState {
                    source: String::new(),
                    is_splitscript,
                });
            }
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
                    text: StyledText::default(),
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
            Tag::HtmlBlock => self.html_block = Some(String::new()),
            Tag::Emphasis => self.inline.emphasis += 1,
            Tag::Strong => self.inline.strong += 1,
            Tag::Strikethrough => self.inline.strikethrough += 1,
            Tag::Superscript
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
                    let source = code.source.trim_end_matches('\n');
                    self.blocks.push(Block::Code(if code.is_splitscript {
                        splitscript_code_fragments(source)
                    } else {
                        vec![CodeFragment {
                            text: source.to_owned(),
                            kind: None,
                        }]
                    }));
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
                    table.row.push(take_normalized_string(&mut cell));
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
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html_block.take() {
                    self.html(&html);
                }
            }
            TagEnd::Emphasis => self.inline.emphasis = self.inline.emphasis.saturating_sub(1),
            TagEnd::Strong => self.inline.strong = self.inline.strong.saturating_sub(1),
            TagEnd::Strikethrough => {
                self.inline.strikethrough = self.inline.strikethrough.saturating_sub(1);
            }
            TagEnd::Superscript
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
        self.push_styled_text(text, self.inline.style());
    }

    fn push_styled_text(&mut self, text: &str, style: InlineStyle) {
        if let Some(code) = &mut self.code {
            code.source.push_str(text);
        } else if let Some(table) = &mut self.table
            && let Some(cell) = &mut table.cell
        {
            cell.push_str(text);
        } else if let Some(item) = self.items.last_mut() {
            item.text.push_str(text, style);
        } else {
            self.text.push_str(text, style);
        }
    }

    fn html(&mut self, html: &str) {
        if let Some(code) = semantic_code_fragments(html) {
            self.blocks.push(Block::Code(code));
        } else if html.contains("splitscript-reference-table") {
            self.suppress_next_table_header = true;
        }
    }
}

fn splitscript_code_fragments(source: &str) -> Vec<CodeFragment> {
    let mut database = CompilerDatabase::new(source);
    let Ok(highlights) = database.semantic_highlights() else {
        return vec![CodeFragment {
            text: source.to_owned(),
            kind: None,
        }];
    };
    let spans = highlights
        .highlights()
        .iter()
        .map(|highlight| (highlight.span.start, highlight.span.end, highlight.kind));
    fragments_from_spans(source, spans)
}

fn fragments_from_spans(
    source: &str,
    spans: impl IntoIterator<Item = (usize, usize, SemanticTokenKind)>,
) -> Vec<CodeFragment> {
    let mut fragments = Vec::new();
    let mut cursor = 0;
    for (start, end, kind) in spans {
        if start < cursor || start > end || end > source.len() {
            continue;
        }
        push_code_fragment(&mut fragments, &source[cursor..start], None);
        push_code_fragment(&mut fragments, &source[start..end], Some(kind));
        cursor = end;
    }
    push_code_fragment(&mut fragments, &source[cursor..], None);
    fragments
}

fn semantic_code_fragments(html: &str) -> Option<Vec<CodeFragment>> {
    const PREFIX: &str = "<pre class=\"hljs splitscript-code\"><code>";
    const SUFFIX: &str = "</code></pre>";
    let mut source = html.trim().strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let mut fragments = Vec::new();
    while !source.is_empty() {
        if let Some(rest) = source.strip_prefix("<a ") {
            source = rest.split_once('>')?.1;
            continue;
        }
        if let Some(rest) = source.strip_prefix("</a>") {
            source = rest;
            continue;
        }
        if let Some(rest) = source.strip_prefix("<span data-splitscript-token=\"") {
            let (kind, rest) = rest.split_once('"')?;
            let kind = SemanticTokenKind::from_name(kind)?;
            let rest = rest.strip_prefix(" class=\"")?;
            let (_, rest) = rest.split_once("\">")?;
            let (text, rest) = rest.split_once("</span>")?;
            push_code_fragment(&mut fragments, &decode_generated_html(text), Some(kind));
            source = rest;
            continue;
        }
        let end = source.find('<').unwrap_or(source.len());
        if end == 0 {
            return None;
        }
        push_code_fragment(&mut fragments, &decode_generated_html(&source[..end]), None);
        source = &source[end..];
    }
    Some(fragments)
}

fn decode_generated_html(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn push_code_fragment(
    fragments: &mut Vec<CodeFragment>,
    text: &str,
    kind: Option<SemanticTokenKind>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(previous) = fragments.last_mut()
        && previous.kind == kind
    {
        previous.text.push_str(text);
    } else {
        fragments.push(CodeFragment {
            text: text.to_owned(),
            kind,
        });
    }
}

fn take_normalized(text: &mut StyledText) -> StyledText {
    let mut normalized = StyledText::default();
    let mut pending_space = None;
    for character in std::mem::take(&mut text.chars) {
        if character.value.is_whitespace() {
            if !normalized.chars.is_empty() {
                pending_space = Some(character.style);
            }
        } else {
            if let Some(style) = pending_space.take() {
                normalized.chars.push(StyledChar { value: ' ', style });
            }
            normalized.chars.push(character);
        }
    }
    normalized
}

fn take_normalized_string(text: &mut String) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.clear();
    normalized
}

/// Renders compiler-owned Markdown as terminal-native text.
///
/// Links intentionally collapse to their labels because the documentation
/// graph uses editor-only virtual paths. `WriteColor` decides whether styling
/// becomes ANSI escapes, so redirected output remains stable plain text.
#[cfg(test)]
pub(crate) fn emit(writer: &mut dyn WriteColor, markdown: &str, width: usize) -> io::Result<()> {
    emit_with_color_depth(writer, markdown, width, ColorDepth::stdout())
}

pub(crate) fn emit_stdout(markdown: &str, width: usize) -> io::Result<()> {
    let color_depth = ColorDepth::stdout();
    if color_depth == ColorDepth::Ansi256 {
        // `termcolor`'s legacy Windows-console backend only supports 16 colors.
        // A terminal that advertises 256 colors understands ANSI escapes, so
        // use its ANSI writer directly instead of silently dropping the richer
        // colors on Windows Terminal and VS Code's integrated terminal.
        let stdout = io::stdout();
        let mut writer = Ansi::new(stdout.lock());
        emit_with_color_depth(&mut writer, markdown, width, color_depth)
    } else {
        let writer = StandardStream::stdout(ColorChoice::Auto);
        emit_with_color_depth(&mut writer.lock(), markdown, width, color_depth)
    }
}

fn emit_with_color_depth(
    writer: &mut dyn WriteColor,
    markdown: &str,
    width: usize,
    color_depth: ColorDepth,
) -> io::Result<()> {
    let width = width.max(20);
    let blocks = DocumentBuilder::parse(markdown);
    for (index, block) in blocks.iter().enumerate() {
        if index != 0 {
            writeln!(writer)?;
        }
        render_block(writer, block, width, color_depth)?;
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

fn render_block(
    writer: &mut dyn WriteColor,
    block: &Block,
    width: usize,
    color_depth: ColorDepth,
) -> io::Result<()> {
    match block {
        Block::Heading { level, text } => {
            let mut style = ColorSpec::new();
            style.set_bold(true).set_fg(Some(match level {
                HeadingLevel::H1 => Color::Cyan,
                _ => Color::Blue,
            }));
            render_styled_range(writer, text, 0, text.chars.len(), &style, color_depth)?;
            writer.reset()?;
            writeln!(writer)
        }
        Block::Paragraph { quote_depth, text } => {
            let prefix = "> ".repeat(*quote_depth);
            write_wrapped(writer, text, &prefix, &prefix, width, color_depth)
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
            write_wrapped(writer, text, &initial, &subsequent, width, color_depth)
        }
        Block::Code(fragments) => render_code(writer, fragments, color_depth),
        Block::Table(rows) => render_table(writer, rows, width),
        Block::Rule => writeln!(writer, "{}", "─".repeat(width.min(80))),
    }
}

fn render_code(
    writer: &mut dyn WriteColor,
    fragments: &[CodeFragment],
    color_depth: ColorDepth,
) -> io::Result<()> {
    let mut line_start = true;
    for fragment in fragments {
        writer.set_color(&code_style(fragment.kind, color_depth))?;
        for part in fragment.text.split_inclusive('\n') {
            if line_start {
                writer.reset()?;
                write!(writer, "  ")?;
                writer.set_color(&code_style(fragment.kind, color_depth))?;
            }
            write!(writer, "{part}")?;
            line_start = part.ends_with('\n');
        }
    }
    writer.reset()?;
    if !line_start {
        writeln!(writer)?;
    }
    Ok(())
}

fn code_style(kind: Option<SemanticTokenKind>, color_depth: ColorDepth) -> ColorSpec {
    let mut style = ColorSpec::new();
    match kind {
        Some(SemanticTokenKind::Keyword | SemanticTokenKind::Debug) => {
            style
                .set_fg(Some(color_depth.color(Color::Red, 197)))
                .set_intense(true);
        }
        Some(SemanticTokenKind::Capability) => {
            style
                .set_fg(Some(color_depth.color(Color::Yellow, 208)))
                .set_intense(true);
        }
        Some(SemanticTokenKind::Type | SemanticTokenKind::Struct) => {
            style.set_fg(Some(color_depth.color(Color::Cyan, 81)));
        }
        Some(SemanticTokenKind::Enum) => {
            style
                .set_fg(Some(color_depth.color(Color::Cyan, 81)))
                .set_italic(true);
        }
        Some(SemanticTokenKind::EnumMember) => {
            style.set_fg(Some(color_depth.color(Color::Magenta, 197)));
        }
        Some(
            SemanticTokenKind::Constant | SemanticTokenKind::Number | SemanticTokenKind::Version,
        ) => {
            style.set_fg(Some(color_depth.color(Color::Magenta, 141)));
        }
        Some(
            SemanticTokenKind::Function | SemanticTokenKind::Method | SemanticTokenKind::Lifecycle,
        ) => {
            style
                .set_fg(Some(color_depth.color(Color::Green, 148)))
                .set_intense(true);
        }
        Some(
            SemanticTokenKind::Property
            | SemanticTokenKind::Setting
            | SemanticTokenKind::StateField,
        ) => {
            style
                .set_fg(Some(color_depth.color(Color::White, 193)))
                .set_intense(true);
        }
        Some(SemanticTokenKind::SettingTitle | SemanticTokenKind::String) => {
            style.set_fg(Some(color_depth.color(Color::Yellow, 186)));
        }
        Some(SemanticTokenKind::Namespace) => {
            style
                .set_fg(Some(color_depth.color(Color::Cyan, 85)))
                .set_intense(true);
        }
        Some(SemanticTokenKind::TemplateString | SemanticTokenKind::Signature) => {
            style.set_fg(Some(color_depth.color(Color::Green, 77)));
        }
        Some(SemanticTokenKind::Operator) => {
            style.set_fg(Some(color_depth.color(Color::Red, 197)));
        }
        Some(SemanticTokenKind::Comment) => {
            style
                .set_fg(Some(color_depth.color(Color::White, 242)))
                .set_dimmed(true);
        }
        Some(SemanticTokenKind::Variable | SemanticTokenKind::Parameter) => {
            style.set_fg(Some(color_depth.color(Color::White, 231)));
        }
        None => {
            style.set_fg(Some(color_depth.color(Color::White, 231)));
        }
    }
    style
}

fn write_wrapped(
    writer: &mut dyn WriteColor,
    text: &StyledText,
    initial: &str,
    subsequent: &str,
    width: usize,
    color_depth: ColorDepth,
) -> io::Result<()> {
    let plain = text.plain();
    let options = textwrap::Options::new(width)
        .initial_indent(initial)
        .subsequent_indent(subsequent);
    let mut offset = 0;
    for (index, line) in textwrap::wrap(&plain, options).into_iter().enumerate() {
        let prefix = if index == 0 { initial } else { subsequent };
        let content = line.strip_prefix(prefix).unwrap_or(&line);
        writer.reset()?;
        write!(writer, "{prefix}")?;
        let count = content.chars().count();
        render_styled_range(writer, text, offset, count, &ColorSpec::new(), color_depth)?;
        offset += count;
        while text
            .chars
            .get(offset)
            .is_some_and(|character| character.value.is_whitespace())
        {
            offset += 1;
        }
        writer.reset()?;
        writeln!(writer)?;
    }
    Ok(())
}

fn render_styled_range(
    writer: &mut dyn WriteColor,
    text: &StyledText,
    start: usize,
    count: usize,
    base: &ColorSpec,
    color_depth: ColorDepth,
) -> io::Result<()> {
    let mut current_style = None;
    let mut run = String::new();
    for character in text.chars.iter().skip(start).take(count) {
        if current_style != Some(character.style) {
            if let Some(style) = current_style {
                writer.set_color(&inline_style(base, style, color_depth))?;
                write!(writer, "{run}")?;
                run.clear();
            }
            current_style = Some(character.style);
        }
        run.push(character.value);
    }
    if let Some(style) = current_style {
        writer.set_color(&inline_style(base, style, color_depth))?;
        write!(writer, "{run}")?;
    }
    Ok(())
}

fn inline_style(base: &ColorSpec, inline: InlineStyle, color_depth: ColorDepth) -> ColorSpec {
    let mut style = base.clone();
    style
        .set_bold(base.bold() || inline.bold)
        .set_italic(base.italic() || inline.italic)
        .set_strikethrough(base.strikethrough() || inline.strikethrough);
    if inline.code {
        style.set_fg(Some(color_depth.color(Color::Yellow, 186)));
    }
    style
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
    let minimum = (available / columns).clamp(4, 12);
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
    fn preserves_markdown_emphasis_strong_text_and_inline_code() {
        let markdown = "_Method_\n\n**Effects:** reads `address`.\n";

        let mut plain = Buffer::no_color();
        emit(&mut plain, markdown, 80).unwrap();
        assert_eq!(
            String::from_utf8(plain.into_inner()).unwrap(),
            "Method\n\nEffects: reads address.\n",
        );

        let mut ansi = Buffer::ansi();
        emit_with_color_depth(&mut ansi, markdown, 80, ColorDepth::Ansi16).unwrap();
        let ansi = String::from_utf8(ansi.into_inner()).unwrap();
        assert!(ansi.contains("\u{1b}[3m"), "{ansi:?}");
        assert!(ansi.contains("\u{1b}[1m"), "{ansi:?}");
        assert!(ansi.contains("\u{1b}[33m"), "{ansi:?}");
    }

    #[test]
    fn maps_semantic_tokens_to_the_configured_ansi256_palette() {
        let cases = [
            (SemanticTokenKind::Keyword, 197),
            (SemanticTokenKind::Operator, 197),
            (SemanticTokenKind::Capability, 208),
            (SemanticTokenKind::Type, 81),
            (SemanticTokenKind::Enum, 81),
            (SemanticTokenKind::EnumMember, 197),
            (SemanticTokenKind::Function, 148),
            (SemanticTokenKind::Method, 148),
            (SemanticTokenKind::Property, 193),
            (SemanticTokenKind::Namespace, 85),
            (SemanticTokenKind::String, 186),
            (SemanticTokenKind::TemplateString, 77),
            (SemanticTokenKind::Number, 141),
            (SemanticTokenKind::Constant, 141),
            (SemanticTokenKind::Version, 141),
            (SemanticTokenKind::Comment, 242),
            (SemanticTokenKind::Variable, 231),
        ];
        for (kind, color) in cases {
            assert_eq!(
                code_style(Some(kind), ColorDepth::Ansi256).fg(),
                Some(&Color::Ansi256(color)),
                "wrong color for {kind:?}",
            );
        }
        assert!(code_style(Some(SemanticTokenKind::Enum), ColorDepth::Ansi256).italic());
        assert!(code_style(Some(SemanticTokenKind::Comment), ColorDepth::Ansi256).dimmed());
    }

    #[test]
    fn keeps_a_readable_basic_color_fallback() {
        assert_eq!(
            code_style(Some(SemanticTokenKind::Keyword), ColorDepth::Ansi16).fg(),
            Some(&Color::Red),
        );
        assert_eq!(
            code_style(Some(SemanticTokenKind::String), ColorDepth::Ansi16).fg(),
            Some(&Color::Yellow),
        );
        assert_eq!(
            code_style(Some(SemanticTokenKind::Type), ColorDepth::Ansi16).fg(),
            Some(&Color::Cyan),
        );
        assert!(code_style(Some(SemanticTokenKind::Enum), ColorDepth::Ansi16).italic());
    }

    #[test]
    fn renders_compiler_owned_semantic_code_as_terminal_colors() {
        let reference = splitscript::DocumentationReference::default();
        let page = reference
            .page("/stdlib/types/Process/methods/read.md")
            .expect("Process.read has a documentation page");

        let mut plain = Buffer::no_color();
        emit(&mut plain, &page.markdown, 100).unwrap();
        let plain = String::from_utf8(plain.into_inner()).unwrap();
        assert!(plain.contains("Process.read<T>(address: address) -> T!"));
        assert!(plain.contains("let health = process.read<i32>"));
        assert!(!plain.contains("<pre"));
        assert!(!plain.contains("data-splitscript-token"));
        assert!(!plain.contains("\u{1b}["));

        let mut ansi = Buffer::ansi();
        emit(&mut ansi, &page.markdown, 100).unwrap();
        let ansi = String::from_utf8(ansi.into_inner()).unwrap();
        assert!(ansi.matches("\u{1b}[").count() > 8, "{ansi:?}");
        assert!(ansi.contains("Process"));
        assert!(ansi.contains("read"));
    }

    #[test]
    fn renders_module_scan_example_code() {
        let reference = splitscript::DocumentationReference::default();
        let page = reference
            .topic("Module.scan")
            .expect("Module.scan has a documentation page");
        let blocks = DocumentBuilder::parse(&page.markdown);
        assert_eq!(
            blocks
                .iter()
                .filter(|block| matches!(block, Block::Code(_)))
                .count(),
            2,
            "{blocks:#?}",
        );
        let mut plain = Buffer::no_color();
        emit(&mut plain, &page.markdown, 100).unwrap();
        let plain = String::from_utf8(plain.into_inner()).unwrap();
        assert!(
            plain.contains("let marker = await gameAssembly.scan"),
            "markdown:\n{}\nrendered:\n{plain}",
            page.markdown,
        );
    }

    #[test]
    fn renders_ranked_search_results_as_a_compact_borderless_table() {
        let reference = splitscript::DocumentationReference::default();
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
