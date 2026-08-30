use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Component, Path, PathBuf},
};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use serde_json::json;
use splitscript::DocumentationReference;

const OUTPUT_MARKER: &str = ".splitscript-documentation-site";

struct SitePage {
    source_uri: String,
    html: String,
    anchors: BTreeSet<String>,
}

pub(super) fn generate(destination: Option<&Path>) -> Result<(), String> {
    let reference = DocumentationReference::default();
    let errors = reference.validate();
    if !errors.is_empty() {
        return Err(format!(
            "documentation graph is invalid:\n{}",
            errors.join("\n")
        ));
    }

    let index = reference.index();
    let search_index = search_index(&index)?;
    let mut uris = index
        .iter()
        .map(|entry| entry.uri.clone())
        .collect::<Vec<_>>();
    uris.push("/index.md".to_owned());
    uris.sort();
    uris.dedup();

    let mut pages = BTreeMap::new();
    for uri in uris {
        let page = reference
            .page(&uri)
            .ok_or_else(|| format!("missing rendered page `{uri}`"))?;
        let output_path = output_path(&uri)?;
        let (body, mut anchors) = markdown_to_html(&page.markdown);
        anchors.insert("content".to_owned());
        let body = rewrite_document_links(&body);
        let html = page_shell(&page.title, &uri, &output_path, &body);
        if let Some(previous) = pages.insert(
            output_path.clone(),
            SitePage {
                source_uri: uri.clone(),
                html,
                anchors,
            },
        ) {
            return Err(format!(
                "documentation URIs `{}` and `{uri}` map to the same output path `{}`",
                previous.source_uri,
                output_path.display()
            ));
        }
    }

    validate_site(&pages)?;
    if let Some(destination) = destination {
        write_site(destination, &pages, &search_index)?;
    } else {
        println!("validated {} generated documentation pages", pages.len());
    }
    Ok(())
}

fn search_index(entries: &[splitscript::DocumentationIndexEntry]) -> Result<String, String> {
    let mut values = Vec::with_capacity(entries.len() + 1);
    values.push(json!({
        "url": "/index.html",
        "title": "SplitScript reference",
        "kind": "reference",
        "summary": "Language, standard library, migration, and guide documentation.",
        "signature": null,
    }));
    values.extend(entries.iter().map(|entry| {
        json!({
            "url": document_url(&entry.uri),
            "title": entry.title,
            "kind": entry.kind,
            "summary": entry.summary,
            "signature": entry.signature,
        })
    }));
    serde_json::to_string(&values).map_err(|error| error.to_string())
}

fn output_path(uri: &str) -> Result<PathBuf, String> {
    let relative = uri
        .strip_prefix('/')
        .ok_or_else(|| format!("documentation URI is not absolute: `{uri}`"))?;
    let mut path = PathBuf::from(relative);
    if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
        return Err(format!("documentation URI is not Markdown: `{uri}`"));
    }
    path.set_extension("html");
    Ok(path)
}

fn document_url(uri: &str) -> String {
    format!(
        "/{}",
        output_path(uri)
            .expect("catalog URI should be valid")
            .display()
    )
    .replace('\\', "/")
}

fn markdown_to_html(markdown: &str) -> (String, BTreeSet<String>) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let events = Parser::new_ext(markdown, options).collect::<Vec<_>>();
    let mut output = String::new();
    let mut anchors = BTreeSet::new();
    let mut anchor_counts = HashMap::<String, usize>::new();
    let mut chunk_start = 0;
    let mut cursor = 0;

    while cursor < events.len() {
        let Event::Start(Tag::Heading { level, .. }) = &events[cursor] else {
            cursor += 1;
            continue;
        };
        let Some(end) = (cursor + 1..events.len()).find(
            |index| matches!(events[*index], Event::End(TagEnd::Heading(found)) if found == *level),
        ) else {
            cursor += 1;
            continue;
        };

        html::push_html(&mut output, events[chunk_start..cursor].iter().cloned());
        let heading_text = events[cursor + 1..end]
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) | Event::Code(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect::<String>();
        let base = heading_slug(&heading_text);
        let count = anchor_counts.entry(base.clone()).or_default();
        let anchor = if *count == 0 {
            base
        } else {
            format!("{base}-{count}")
        };
        *count += 1;
        anchors.insert(anchor.clone());
        output.push('<');
        output.push_str(heading_tag(*level));
        output.push_str(" id=\"");
        escape_html_attribute(&mut output, &anchor);
        output.push_str("\">");
        html::push_html(&mut output, events[cursor + 1..end].iter().cloned());
        output.push_str("</");
        output.push_str(heading_tag(*level));
        output.push('>');
        cursor = end + 1;
        chunk_start = cursor;
    }
    html::push_html(&mut output, events[chunk_start..].iter().cloned());
    (output, anchors)
}

fn heading_tag(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "h1",
        HeadingLevel::H2 => "h2",
        HeadingLevel::H3 => "h3",
        HeadingLevel::H4 => "h4",
        HeadingLevel::H5 => "h5",
        HeadingLevel::H6 => "h6",
    }
}

fn heading_slug(text: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() || character == '_' {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push(character);
        } else if character.is_whitespace() || character == '-' {
            separator = true;
        }
    }
    if slug.is_empty() {
        "section".to_owned()
    } else {
        slug
    }
}

fn rewrite_document_links(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("href=\"") {
        let value_start = start + "href=\"".len();
        output.push_str(&rest[..value_start]);
        rest = &rest[value_start..];
        let Some(end) = rest.find('"') else {
            output.push_str(rest);
            return output;
        };
        let target = &rest[..end];
        let route_end = target.find(['#', '?']).unwrap_or(target.len());
        let (route, suffix) = target.split_at(route_end);
        if !route.contains("://") && route.ends_with(".md") {
            output.push_str(&route[..route.len() - 3]);
            output.push_str(".html");
            output.push_str(suffix);
        } else {
            output.push_str(target);
        }
        rest = &rest[end..];
    }
    output.push_str(rest);
    output
}

fn page_shell(title: &str, uri: &str, path: &Path, body: &str) -> String {
    let depth = path
        .parent()
        .map_or(0, |parent| parent.components().count());
    let root = "../".repeat(depth);
    let current_url = document_url(uri);
    let mut output = String::from("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    output.push_str("  <meta charset=\"utf-8\">\n");
    output.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    output.push_str("  <title>");
    escape_html(&mut output, title);
    output.push_str(" · SplitScript</title>\n  <meta name=\"description\" content=\"Compiler-owned SplitScript language and standard-library reference.\">\n");
    output.push_str("  <link rel=\"stylesheet\" href=\"");
    output.push_str(&root);
    output.push_str("assets/site.css\">\n  <script defer src=\"");
    output.push_str(&root);
    output.push_str("assets/search-index.js\"></script>\n  <script defer src=\"");
    output.push_str(&root);
    output.push_str("assets/site.js\"></script>\n</head>\n<body data-document-url=\"");
    escape_html_attribute(&mut output, &current_url);
    output.push_str("\" data-site-root=\"");
    escape_html_attribute(&mut output, &root);
    output.push_str("\">\n<a class=\"skip-link\" href=\"#content\">Skip to documentation</a>\n<div class=\"site-shell\">\n<aside class=\"sidebar\">\n<a class=\"brand\" href=\"");
    output.push_str(&root);
    output.push_str("index.html\">SplitScript</a>\n<label class=\"search-label\" for=\"documentation-search\">Search documentation</label>\n<input id=\"documentation-search\" class=\"search\" type=\"search\" placeholder=\"Search symbols and guides\" autocomplete=\"off\">\n<nav id=\"documentation-navigation\" aria-label=\"Documentation\">\n<ul class=\"navigation-fallback\">\n");
    for (label, target) in [
        ("Language", "language/index.html"),
        ("Standard library", "index.html#namespaces"),
        ("Guides", "guides/getting-started.html"),
        ("Migration", "migration/index.html"),
    ] {
        output.push_str("<li><a href=\"");
        output.push_str(&root);
        output.push_str(target);
        output.push_str("\">");
        output.push_str(label);
        output.push_str("</a></li>\n");
    }
    output.push_str("</ul>\n</nav>\n</aside>\n<main id=\"content\" class=\"content\"><article>\n");
    output.push_str(body);
    output.push_str("\n</article><footer>Generated from the SplitScript compiler's documentation catalogs.</footer></main>\n</div>\n</body>\n</html>\n");
    output
}

fn write_site(
    destination: &Path,
    pages: &BTreeMap<PathBuf, SitePage>,
    search_index: &str,
) -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    validate_destination(destination)?;
    let destination = root.join(destination);
    if destination.exists() {
        let marker = destination.join(OUTPUT_MARKER);
        if !marker.is_file() {
            return Err(format!(
                "refusing to replace unmarked directory {}; choose an empty output path",
                destination.display()
            ));
        }
        fs::remove_dir_all(&destination)
            .map_err(|error| format!("could not clear {}: {error}", destination.display()))?;
    }
    fs::create_dir_all(destination.join("assets"))
        .map_err(|error| format!("could not create {}: {error}", destination.display()))?;
    fs::write(
        destination.join(OUTPUT_MARKER),
        "generated by cargo xtask docs\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(destination.join("assets/site.css"), SITE_CSS).map_err(|error| error.to_string())?;
    fs::write(destination.join("assets/site.js"), SITE_JS).map_err(|error| error.to_string())?;
    fs::write(
        destination.join("assets/search-index.js"),
        format!("window.SPLITSCRIPT_DOCUMENTATION = {search_index};\n"),
    )
    .map_err(|error| error.to_string())?;

    for (relative, page) in pages {
        let output = destination.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        fs::write(&output, &page.html)
            .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    }
    println!(
        "wrote {} documentation pages to {}",
        pages.len(),
        destination.display()
    );
    Ok(())
}

fn validate_destination(destination: &Path) -> Result<(), String> {
    if destination.as_os_str().is_empty()
        || destination.is_absolute()
        || destination
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "documentation output must be a non-empty repository-relative directory without `..`: {}",
            destination.display()
        ));
    }
    Ok(())
}

fn validate_site(pages: &BTreeMap<PathBuf, SitePage>) -> Result<(), String> {
    let known = pages
        .keys()
        .map(|path| format!("/{}", path.display()).replace('\\', "/"))
        .collect::<BTreeSet<_>>();
    let anchors = pages
        .iter()
        .map(|(path, page)| {
            (
                format!("/{}", path.display()).replace('\\', "/"),
                &page.anchors,
            )
        })
        .collect::<BTreeMap<_, _>>();

    for (path, page) in pages {
        let current = format!("/{}", path.display()).replace('\\', "/");
        for target in html_attributes(&page.html, "href=\"") {
            if target.starts_with("http:")
                || target.starts_with("https:")
                || target.starts_with("mailto:")
                || target.starts_with("command:")
            {
                continue;
            }
            if target.contains(".md") {
                return Err(format!(
                    "generated page `{current}` retains Markdown link `{target}`"
                ));
            }
            let (route, fragment) = target.split_once('#').unwrap_or((target, ""));
            let resolved = if route.is_empty() {
                current.clone()
            } else {
                resolve_url(&current, route)?
            };
            if resolved.contains("/assets/") {
                continue;
            }
            if !known.contains(&resolved) {
                return Err(format!(
                    "generated page `{current}` links to missing page `{resolved}` via `{target}`"
                ));
            }
            if !fragment.is_empty()
                && !anchors
                    .get(&resolved)
                    .is_some_and(|values| values.contains(fragment))
            {
                return Err(format!(
                    "generated page `{current}` links to missing anchor `{fragment}` on `{resolved}`"
                ));
            }
        }
    }
    Ok(())
}

fn html_attributes<'a>(html: &'a str, prefix: &str) -> Vec<&'a str> {
    let mut values = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find(prefix) {
        rest = &rest[start + prefix.len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        values.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    values
}

fn resolve_url(current: &str, target: &str) -> Result<String, String> {
    let target = target.split('?').next().unwrap_or(target);
    let mut segments = if target.starts_with('/') {
        Vec::new()
    } else {
        let mut base = current
            .trim_start_matches('/')
            .split('/')
            .collect::<Vec<_>>();
        base.pop();
        base
    };
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(format!("link escapes documentation root: `{target}`"));
                }
            }
            value => segments.push(value),
        }
    }
    if target.ends_with('/') {
        segments.push("index.html");
    }
    Ok(format!("/{}", segments.join("/")))
}

fn escape_html(output: &mut String, source: &str) {
    for character in source.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

fn escape_html_attribute(output: &mut String, source: &str) {
    escape_html(output, source);
}

const SITE_CSS: &str = r#":root {
  color-scheme: dark;
  --background: #181818;
  --surface: #222222;
  --surface-raised: #292929;
  --border: #3d3d3d;
  --text: #eeeeee;
  --muted: #aaaaaa;
  --link: #58a6ff;
  --focus: #5fd7ff;
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body { margin: 0; background: var(--background); color: var(--text); line-height: 1.6; }
a { color: var(--link); text-decoration: none; }
a:hover, a:focus-visible { text-decoration: underline; }
.skip-link { position: fixed; top: -4rem; left: 1rem; z-index: 10; padding: .6rem .9rem; background: var(--surface-raised); }
.skip-link:focus { top: 1rem; }
.site-shell { display: grid; grid-template-columns: minmax(16rem, 21rem) minmax(0, 1fr); min-height: 100vh; }
.sidebar { position: sticky; top: 0; height: 100vh; overflow: auto; padding: 1.4rem 1.2rem 2rem; border-right: 1px solid var(--border); background: var(--surface); }
.brand { display: block; margin: 0 0 1.2rem; color: var(--text); font-size: 1.35rem; font-weight: 700; text-decoration: none; }
.search-label { display: block; margin-bottom: .35rem; color: var(--muted); font-size: .78rem; font-weight: 650; text-transform: uppercase; letter-spacing: .06em; }
.search { width: 100%; margin-bottom: 1rem; padding: .65rem .75rem; border: 1px solid var(--border); border-radius: .4rem; background: var(--background); color: var(--text); font: inherit; }
.search:focus { outline: 2px solid var(--focus); outline-offset: 1px; }
#documentation-navigation ul { margin: 0; padding-left: 1rem; list-style: none; }
#documentation-navigation > ul { padding-left: 0; }
#documentation-navigation li { margin: .15rem 0; }
#documentation-navigation a { display: block; padding: .28rem .45rem; border-radius: .3rem; color: var(--muted); text-decoration: none; line-height: 1.35; }
#documentation-navigation a:hover, #documentation-navigation a:focus-visible { color: var(--text); background: var(--surface-raised); }
#documentation-navigation a[aria-current="page"] { color: var(--text); background: #343434; font-weight: 650; }
.navigation-section { margin-top: .4rem !important; }
#documentation-navigation .navigation-section-title { color: var(--text); font-weight: 650; }
.navigation-section > ul { margin: .2rem 0 .55rem; padding-left: .7rem !important; border-left: 1px solid var(--border); }
.navigation-kind { margin-left: .4rem; color: #777; font-size: .72rem; }
.search-summary { display: block; margin-top: .12rem; color: #888; font-size: .76rem; }
.content { width: min(100%, 68rem); padding: 2.5rem clamp(1.25rem, 4vw, 4rem) 4rem; }
article > :first-child { margin-top: 0; }
h1, h2, h3, h4 { line-height: 1.25; scroll-margin-top: 1rem; }
h1 { margin: 1rem 0 1.4rem; font-size: clamp(2rem, 5vw, 3rem); }
h2 { margin-top: 2.4rem; padding-bottom: .35rem; border-bottom: 1px solid var(--border); }
h3 { margin-top: 1.8rem; }
code { padding: .1rem .28rem; border-radius: .25rem; background: var(--surface-raised); font-family: "Cascadia Code", "SFMono-Regular", Consolas, monospace; font-size: .92em; }
pre { max-width: 100%; overflow: auto; padding: 1rem; border: 1px solid #2f2f2f; border-radius: .45rem; background: #111; line-height: 1.55; }
pre code { padding: 0; background: transparent; font-size: .9rem; }
blockquote { margin-left: 0; padding-left: 1rem; border-left: .25rem solid var(--border); color: var(--muted); }
table { width: 100%; border-collapse: collapse; }
th, td { padding: .45rem .8rem .45rem 0; text-align: left; vertical-align: top; border-bottom: 1px solid var(--border); }
.splitscript-reference-table { display: none; }
.splitscript-reference-table + table thead { display: none; }
.splitscript-reference-table + table th, .splitscript-reference-table + table td { border: 0; }
.splitscript-code a, .splitscript-code a:visited { color: inherit; text-decoration: none; }
.splitscript-code a:hover, .splitscript-code a:focus-visible { text-decoration: underline; }
.splitscript-code [data-splitscript-token="keyword"], .splitscript-code [data-splitscript-token="debug"], .splitscript-code [data-splitscript-token="operator"] { color: #ff005f; }
.splitscript-code [data-splitscript-token="interface"] { color: #ff8700; font-weight: 650; }
.splitscript-code [data-splitscript-token="type"], .splitscript-code [data-splitscript-token="struct"] { color: #5fd7ff; }
.splitscript-code [data-splitscript-token="enum"] { color: #5fd7ff; font-style: italic; }
.splitscript-code [data-splitscript-token="enumMember"] { color: #F397FF; }
.splitscript-code [data-splitscript-token="constant"], .splitscript-code [data-splitscript-token="number"], .splitscript-code [data-splitscript-token="version"] { color: #af87ff; }
.splitscript-code [data-splitscript-token="function"], .splitscript-code [data-splitscript-token="method"], .splitscript-code [data-splitscript-token="lifecycle"] { color: #afd75f; font-weight: 650; }
.splitscript-code [data-splitscript-token="property"], .splitscript-code [data-splitscript-token="setting"], .splitscript-code [data-splitscript-token="settingTitle"], .splitscript-code [data-splitscript-token="stateField"] { color: #d7ffaf; }
.splitscript-code [data-splitscript-token="string"] { color: #d7d75f; }
.splitscript-code [data-splitscript-token="namespace"] { color: #5fffaf; font-weight: 650; }
.splitscript-code [data-splitscript-token="templateString"], .splitscript-code [data-splitscript-token="signature"] { color: #5fd75f; }
.splitscript-code [data-splitscript-token="comment"] { color: #6c6c6c; }
.splitscript-code [data-splitscript-token="variable"], .splitscript-code [data-splitscript-token="parameter"] { color: #ffffff; }
footer { margin-top: 4rem; padding-top: 1rem; border-top: 1px solid var(--border); color: var(--muted); font-size: .85rem; }
@media (max-width: 760px) {
  .site-shell { display: block; }
  .sidebar { position: static; width: auto; height: auto; max-height: 42vh; border-right: 0; border-bottom: 1px solid var(--border); }
  .content { padding-top: 1.6rem; }
}
"#;

const SITE_JS: &str = r##"(() => {
  const entries = Array.isArray(window.SPLITSCRIPT_DOCUMENTATION)
    ? window.SPLITSCRIPT_DOCUMENTATION
    : [];
  const navigation = document.getElementById("documentation-navigation");
  const search = document.getElementById("documentation-search");
  const current = document.body.dataset.documentUrl || "/index.html";
  const siteRoot = document.body.dataset.siteRoot || "";
  const byUrl = new Map(entries.map((entry) => [entry.url, entry]));

  const localUrl = (url) => siteRoot + url.replace(/^\//, "");
  const link = (entry, includeSummary = false) => {
    const anchor = document.createElement("a");
    anchor.href = localUrl(entry.url);
    anchor.textContent = entry.title;
    if (entry.url === current) anchor.setAttribute("aria-current", "page");
    if (includeSummary && entry.summary) {
      const summary = document.createElement("span");
      summary.className = "search-summary";
      summary.textContent = entry.summary;
      anchor.append(summary);
    }
    return anchor;
  };
  const sorted = (values) => [...values].sort((left, right) => left.title.localeCompare(right.title));
  const entryList = (values) => {
    const list = document.createElement("ul");
    for (const entry of sorted(values)) {
      const item = document.createElement("li");
      item.append(link(entry));
      list.append(item);
    }
    return list;
  };
  const section = (title, target, values = []) => {
    const item = document.createElement("li");
    item.className = "navigation-section";
    const heading = document.createElement("a");
    heading.className = "navigation-section-title";
    heading.href = localUrl(target);
    heading.textContent = title;
    if (target.split("#")[0] === current) heading.setAttribute("aria-current", "page");
    item.append(heading);
    if (values.length) item.append(entryList(values));
    return item;
  };
  const inSection = (prefix) => current.startsWith(prefix);
  const standardLibraryEntries = (prefix) => {
    if (!inSection(prefix)) return [];
    if (prefix === "/stdlib/functions/" || prefix === "/stdlib/state-providers/") {
      return entries.filter((entry) => entry.url.startsWith(prefix));
    }
    const remainder = current.slice(prefix.length);
    const owner = remainder.split("/")[0];
    if (!owner) return [];
    const ownerPrefix = prefix + owner + "/";
    return entries.filter((entry) => entry.url === ownerPrefix + "index.html" || entry.url.startsWith(ownerPrefix));
  };
  const renderTree = () => {
    const list = document.createElement("ul");
    const root = byUrl.get("/index.html");
    if (root) {
      const item = document.createElement("li");
      item.append(link(root));
      list.append(item);
    }
    list.append(section(
      "Language",
      "/language/index.html",
      inSection("/language/")
        ? entries.filter((entry) => entry.url.startsWith("/language/") && entry.url !== "/language/index.html")
        : [],
    ));
    list.append(section(
      "Guides",
      "/index.html#guides",
      inSection("/guides/") ? entries.filter((entry) => entry.url.startsWith("/guides/")) : [],
    ));
    const standardLibrary = section("Standard library", "/index.html#namespaces");
    const categories = document.createElement("ul");
    for (const [title, prefix, anchor] of [
      ["Namespaces", "/stdlib/namespaces/", "namespaces"],
      ["Types", "/stdlib/types/", "types"],
      ["Type forms", "/stdlib/type-forms/", "types"],
      ["Capabilities", "/stdlib/capabilities/", "capabilities"],
      ["State providers", "/stdlib/state-providers/", "state-providers"],
      ["Functions", "/stdlib/functions/", "functions"],
    ]) {
      categories.append(section(title, `/index.html#${anchor}`, standardLibraryEntries(prefix)));
    }
    standardLibrary.append(categories);
    list.append(standardLibrary);
    const migrationValues = inSection("/migration/") && current !== "/migration/index.html"
      ? entries.filter((entry) => entry.url === current)
      : [];
    list.append(section("Migration", "/migration/index.html", migrationValues));
    navigation.replaceChildren(list);
  };
  const renderSearch = (query) => {
    const terms = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
    if (!terms.length) return renderTree();
    const matches = entries
      .map((entry) => {
        const title = entry.title.toLocaleLowerCase();
        const haystack = [entry.title, entry.kind, entry.summary, entry.signature || ""].join(" ").toLocaleLowerCase();
        if (!terms.every((term) => haystack.includes(term))) return null;
        const score = terms.reduce((value, term) => value + (title === term ? 20 : title.startsWith(term) ? 10 : title.includes(term) ? 5 : 1), 0);
        return { entry, score };
      })
      .filter(Boolean)
      .sort((left, right) => right.score - left.score || left.entry.title.localeCompare(right.entry.title))
      .slice(0, 50);
    const list = document.createElement("ul");
    for (const match of matches) {
      const item = document.createElement("li");
      item.append(link(match.entry, true));
      list.append(item);
    }
    if (!matches.length) {
      const item = document.createElement("li");
      item.textContent = "No documentation found.";
      list.append(item);
    }
    navigation.replaceChildren(list);
  };
  search?.addEventListener("input", () => renderSearch(search.value));
  document.addEventListener("keydown", (event) => {
    if (event.key === "/" && document.activeElement !== search) {
      event.preventDefault();
      search?.focus();
    } else if (event.key === "Escape" && document.activeElement === search) {
      search.value = "";
      renderTree();
      search.blur();
    }
  });
  renderTree();
})();
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_links_are_rewritten_without_touching_external_links() {
        let html =
            r#"<a href="../foo.md#bar">foo</a><a href="https://example.com/a.md">external</a>"#;
        assert_eq!(
            rewrite_document_links(html),
            r#"<a href="../foo.html#bar">foo</a><a href="https://example.com/a.md">external</a>"#
        );
    }

    #[test]
    fn headings_receive_stable_unique_anchors() {
        let (html, anchors) =
            markdown_to_html("# Hello, world!\n\n## Hello world\n\n## Hello world");
        assert!(html.contains("<h1 id=\"hello-world\">"));
        assert!(html.contains("<h2 id=\"hello-world-1\">"));
        assert!(html.contains("<h2 id=\"hello-world-2\">"));
        assert_eq!(anchors.len(), 3);
    }

    #[test]
    fn site_presentation_keeps_links_modern_prose_wide_and_variants_distinct() {
        assert!(SITE_CSS.contains("a { color: var(--link); text-decoration: none; }"));
        assert!(SITE_CSS.contains("a:hover, a:focus-visible { text-decoration: underline; }"));
        assert!(!SITE_CSS.contains("p, li { max-width:"));
        assert!(SITE_CSS.contains("[data-splitscript-token=\"enumMember\"] { color: #F397FF; }"));
    }
}
