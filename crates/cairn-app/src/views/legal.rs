//! In-app reader for the Terms of Service and the Privacy Policy.
//!
//! The text is the repository's `TERMS.md`, `TERMS.en.md`, `PRIVACY.md` and `PRIVACY.en.md`, embedded at
//! build time, so the launcher shows exactly what the repository publishes. Only the small Markdown subset
//! those files use is understood: `#` to `###` headings, paragraphs, `-` and `1.` list items, `>` quotes
//! and pipe tables. Inline `**bold**`, backticks and `[text](url)` links are reduced to plain text.

use std::sync::OnceLock;

use cairn_i18n::{Language, current_language, tr};
use cairn_ui::{Align, El, Insets, Styled, column, divider, label, row};

use crate::gui::Gui;
use crate::style::{accent, fg};

const TERMS_ZH: &str = include_str!("../../../../TERMS.md");
const TERMS_EN: &str = include_str!("../../../../TERMS.en.md");
const PRIVACY_ZH: &str = include_str!("../../../../PRIVACY.md");
const PRIVACY_EN: &str = include_str!("../../../../PRIVACY.en.md");

/// One block of a document.
#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// `#`: the title of the document.
    Title(String),
    /// `##`: a chapter.
    Chapter(String),
    /// `###`: a section of a chapter.
    Section(String),
    Paragraph(String),
    /// A list item. `marker` is a bullet or a number such as `3.`; `lead` is the bold lead-in when the
    /// item starts with one.
    Item {
        marker: String,
        lead: Option<String>,
        text: String,
    },
    Quote(String),
    /// A table. The first column names the row; the other columns are shown as `header: value` lines.
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

struct Documents {
    terms: Vec<Block>,
    privacy: Vec<Block>,
}

/// The documents for the current interface language, and whether that language has its own translation.
/// Languages without one (Japanese) get the English text.
fn documents() -> (&'static Documents, bool) {
    static ZH: OnceLock<Documents> = OnceLock::new();
    static EN: OnceLock<Documents> = OnceLock::new();
    match current_language() {
        Language::ZhCn => (ZH.get_or_init(|| Documents { terms: parse(TERMS_ZH), privacy: parse(PRIVACY_ZH) }), true),
        language => (EN.get_or_init(|| Documents { terms: parse(TERMS_EN), privacy: parse(PRIVACY_EN) }), language == Language::EnUs),
    }
}

/// The full text of both documents as scroll content: the Terms of Service, then the Privacy Policy.
pub(super) fn document() -> Vec<El<Gui>> {
    let (docs, translated) = documents();
    let colon = if current_language() == Language::ZhCn { "：" } else { ": " };
    let mut els: Vec<El<Gui>> = Vec::new();
    if !translated {
        els.push(label(tr!("setup.legal_untranslated")).caption().color(fg(0.6)).into());
    }
    els.extend(docs.terms.iter().map(|b| block_view(b, colon)));
    els.push(spaced(18.0, 6.0, divider().fill_width()));
    els.extend(docs.privacy.iter().map(|b| block_view(b, colon)));
    els.push(spaced(18.0, 10.0, label(tr!("setup.legal_end")).caption().color(fg(0.5))));
    els
}

fn spaced(top: f32, bottom: f32, child: impl Into<El<Gui>>) -> El<Gui> {
    column().fill_width().padding(Insets::new(top, 0.0, bottom, 0.0)).child(child).into()
}

fn block_view(block: &Block, colon: &str) -> El<Gui> {
    match block {
        Block::Title(t) => spaced(6.0, 6.0, label(t.as_str()).title()),
        Block::Chapter(t) => spaced(22.0, 4.0, label(t.as_str()).font_size(18.0).weight(700).color(fg(0.96))),
        Block::Section(t) => spaced(10.0, 0.0, label(t.as_str()).font_size(15.0).weight(700).color(fg(0.92))),
        Block::Paragraph(t) => label(t.as_str()).color(fg(0.88)).into(),
        Block::Item { marker, lead, text } => item_view(marker, lead.as_deref(), text),
        Block::Quote(t) => column()
            .fill_width()
            .padding(Insets::symmetric(14.0, 10.0))
            .background(fg(0.06))
            .corner_radius(8.0)
            .child(label(t.as_str()).weight(700).color(accent()))
            .into(),
        Block::Table { header, rows } => table_view(header, rows, colon),
    }
}

fn item_view(marker: &str, lead: Option<&str>, text: &str) -> El<Gui> {
    let mut body = column().grow(1.0).min_width(0.0).gap(2.0);
    if let Some(lead) = lead {
        body = body.child(label(lead).weight(700).color(fg(0.95)));
    }
    if !text.is_empty() {
        body = body.child(label(text).color(fg(0.88)));
    }
    row().fill_width().gap(8.0).align(Align::Start).child(label(marker).color(accent()).width(24.0).shrink(0.0)).child(body).into()
}

/// Each row becomes a small card: the first cell as its title, the other cells as `header: value` lines.
fn table_view(header: &[String], rows: &[Vec<String>], colon: &str) -> El<Gui> {
    let mut list = column().fill_width().gap(6.0);
    for cells in rows {
        let mut card = column().fill_width().gap(3.0).padding(Insets::symmetric(12.0, 10.0)).background(fg(0.045)).corner_radius(8.0);
        if let Some(first) = cells.first() {
            card = card.child(label(first.as_str()).weight(700).color(fg(0.95)));
        }
        for (i, cell) in cells.iter().enumerate().skip(1) {
            let line = match header.get(i) {
                Some(name) if !name.is_empty() => format!("{name}{colon}{cell}"),
                _ => cell.clone(),
            };
            card = card.child(label(line).color(fg(0.84)));
        }
        list = list.child(card);
    }
    list.into()
}

/// Reads the Markdown subset described in the module documentation.
fn parse(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    let mut table: Vec<&str> = Vec::new();

    for line in md.lines() {
        let line = line.trim_end();
        // A table or a quote ends at the first line that does not continue it.
        if !line.starts_with('|') && !table.is_empty() {
            out.push(parse_table(&table));
            table.clear();
        }
        if !line.starts_with('>') && !quote.is_empty() {
            out.push(Block::Quote(clean(&quote.join(" "))));
            quote.clear();
        }
        if line.is_empty() || line == "---" {
            flush_paragraph(&mut out, &mut para);
        } else if let Some(t) = line.strip_prefix("### ") {
            flush_paragraph(&mut out, &mut para);
            out.push(Block::Section(clean(t)));
        } else if let Some(t) = line.strip_prefix("## ") {
            flush_paragraph(&mut out, &mut para);
            out.push(Block::Chapter(clean(t)));
        } else if let Some(t) = line.strip_prefix("# ") {
            flush_paragraph(&mut out, &mut para);
            out.push(Block::Title(clean(t)));
        } else if line.starts_with('|') {
            flush_paragraph(&mut out, &mut para);
            table.push(line);
        } else if let Some(t) = line.strip_prefix('>') {
            flush_paragraph(&mut out, &mut para);
            let t = t.trim();
            if t.is_empty() {
                // An empty quote line separates two quoted paragraphs.
                if !quote.is_empty() {
                    out.push(Block::Quote(clean(&quote.join(" "))));
                    quote.clear();
                }
            } else {
                quote.push(t.to_string());
            }
        } else if let Some(t) = line.strip_prefix("- ") {
            flush_paragraph(&mut out, &mut para);
            out.push(item("•", t));
        } else if let Some((number, t)) = ordered(line) {
            flush_paragraph(&mut out, &mut para);
            out.push(item(&number, t));
        } else {
            para.push(line);
        }
    }
    flush_paragraph(&mut out, &mut para);
    if !table.is_empty() {
        out.push(parse_table(&table));
    }
    if !quote.is_empty() {
        out.push(Block::Quote(clean(&quote.join(" "))));
    }
    out
}

fn flush_paragraph(out: &mut Vec<Block>, para: &mut Vec<&str>) {
    if para.is_empty() {
        return;
    }
    let raw = para.join(" ");
    para.clear();
    if is_language_link(&raw) {
        return;
    }
    out.push(Block::Paragraph(clean(&raw)));
}

/// A line that only points at the other language's file, such as `English version: [PRIVACY.en.md](PRIVACY.en.md)`.
/// It means nothing inside the launcher, where both languages' texts are right here.
fn is_language_link(raw: &str) -> bool {
    let Some(open) = raw.find('[') else { return false };
    let (head, tail) = raw.split_at(open);
    let head = head.trim_end();
    head.ends_with(['：', ':']) && head.chars().count() <= 20 && tail.matches('[').count() == 1 && tail.ends_with(".md)")
}

/// `3. text` gives `("3.", "text")`.
fn ordered(line: &str) -> Option<(String, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    line[digits..].strip_prefix(". ").map(|rest| (format!("{}.", &line[..digits]), rest))
}

fn item(marker: &str, text: &str) -> Block {
    let (lead, rest) = split_lead(text);
    Block::Item { marker: marker.to_string(), lead: lead.map(|l| clean(&l)), text: clean(rest) }
}

/// Splits a leading `**bold**` run off the text.
fn split_lead(text: &str) -> (Option<String>, &str) {
    if let Some(rest) = text.strip_prefix("**")
        && let Some(end) = rest.find("**")
    {
        return (Some(rest[..end].to_string()), rest[end + 2..].trim_start());
    }
    (None, text)
}

fn parse_table(lines: &[&str]) -> Block {
    let cells = |line: &str| -> Vec<String> { line.trim().trim_matches('|').split('|').map(|c| clean(c.trim())).collect() };
    let mut rows = lines.iter().map(|l| cells(l));
    let header = rows.next().unwrap_or_default();
    // The line after the header is the `| --- |` separator.
    let body: Vec<Vec<String>> = rows.skip(1).collect();
    Block::Table { header, rows: body }
}

/// Reduces inline Markdown to plain text: `**` and backticks are dropped, and `[text](url)` becomes the text,
/// followed by the address in brackets when it is a web address that the text does not already show.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        // A link is `[text](target)`; the text holds no brackets. Anything else keeps its bracket.
        let link = after.find(']').filter(|&mid| after[mid..].starts_with("](")).and_then(|mid| {
            let target = &after[mid + 2..];
            target.find(')').map(|end| (&after[..mid], &target[..end], mid + 2 + end + 1))
        });
        match link {
            Some((text, target, used)) => {
                out.push_str(&rest[..open]);
                out.push_str(text);
                let bare = target.trim_start_matches("https://").trim_start_matches("http://");
                if target.starts_with("http") && text != target && text != bare {
                    out.push_str(" (");
                    out.push_str(target);
                    out.push(')');
                }
                rest = &after[used..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> [(&'static str, Vec<Block>); 4] {
        [("TERMS.md", parse(TERMS_ZH)), ("TERMS.en.md", parse(TERMS_EN)), ("PRIVACY.md", parse(PRIVACY_ZH)), ("PRIVACY.en.md", parse(PRIVACY_EN))]
    }

    /// Every piece of text a block shows.
    fn texts(blocks: &[Block]) -> Vec<String> {
        let mut out = Vec::new();
        for b in blocks {
            match b {
                Block::Title(t) | Block::Chapter(t) | Block::Section(t) | Block::Paragraph(t) | Block::Quote(t) => out.push(t.clone()),
                Block::Item { marker, lead, text } => {
                    out.push(marker.clone());
                    out.extend(lead.clone());
                    out.push(text.clone());
                }
                Block::Table { header, rows } => {
                    out.extend(header.iter().cloned());
                    out.extend(rows.iter().flatten().cloned());
                }
            }
        }
        out
    }

    /// How many blocks of each kind a document has, plus the table cell count.
    fn shape(blocks: &[Block]) -> [usize; 8] {
        let mut s = [0; 8];
        for b in blocks {
            match b {
                Block::Title(_) => s[0] += 1,
                Block::Chapter(_) => s[1] += 1,
                Block::Section(_) => s[2] += 1,
                Block::Paragraph(_) => s[3] += 1,
                Block::Item { .. } => s[4] += 1,
                Block::Quote(_) => s[5] += 1,
                Block::Table { rows, .. } => {
                    s[6] += rows.len();
                    s[7] += rows.iter().map(Vec::len).sum::<usize>();
                }
            }
        }
        s
    }

    #[test]
    fn inline_markdown_becomes_plain_text() {
        assert_eq!(clean("**粗体** 和 `代码` 与 [链接](https://example.com/a)"), "粗体 和 代码 与 链接 (https://example.com/a)");
        assert_eq!(clean("[example.com/a](https://example.com/a)"), "example.com/a");
        assert_eq!(clean("见 [TERMS.md](TERMS.md)"), "见 TERMS.md");
        assert_eq!(clean("[x] and [y](https://e.com)"), "[x] and y (https://e.com)");
        assert_eq!(clean("`hs_err_pid*.log`"), "hs_err_pid*.log");
    }

    #[test]
    fn blocks_are_read_from_the_markdown_subset() {
        let md =
            "# T\n\ntext one\ncontinues\n\n## C\n\n### S\n\n- **Lead.** body\n- plain\n1. first\n\n> q1\n>\n> q2\n\n| A | B |\n| --- | --- |\n| x | `y` |\n";
        assert_eq!(
            parse(md),
            vec![
                Block::Title("T".into()),
                Block::Paragraph("text one continues".into()),
                Block::Chapter("C".into()),
                Block::Section("S".into()),
                Block::Item { marker: "•".into(), lead: Some("Lead.".into()), text: "body".into() },
                Block::Item { marker: "•".into(), lead: None, text: "plain".into() },
                Block::Item { marker: "1.".into(), lead: None, text: "first".into() },
                Block::Quote("q1".into()),
                Block::Quote("q2".into()),
                Block::Table { header: vec!["A".into(), "B".into()], rows: vec![vec!["x".into(), "y".into()]] },
            ]
        );
        // Windows line endings in a checked-out file change nothing.
        assert_eq!(parse("# T\r\n\r\n- a\r\n"), parse("# T\n\n- a\n"));
    }

    #[test]
    fn only_the_other_language_link_line_is_dropped() {
        assert!(parse("English version: [PRIVACY.en.md](PRIVACY.en.md)\n").is_empty());
        assert!(parse("中文版：[TERMS.md](TERMS.md)\n").is_empty());
        // A sentence that merely mentions the other document stays.
        assert_eq!(
            parse("隐私政策（[PRIVACY.md](PRIVACY.md)）是一份独立的文件。\n"),
            vec![Block::Paragraph("隐私政策（PRIVACY.md）是一份独立的文件。".into())]
        );
    }

    #[test]
    fn both_languages_have_the_same_shape() {
        let [terms_zh, terms_en, privacy_zh, privacy_en] = all().map(|(_, b)| shape(&b));
        assert_eq!(terms_zh, terms_en, "用户协议中英文的章节、列表、表格数量必须一致");
        assert_eq!(privacy_zh, privacy_en, "隐私政策中英文的章节、列表、表格数量必须一致");
        assert_eq!(terms_zh[1], 19, "用户协议有 18 章加上“重要提示”");
    }

    #[test]
    fn documents_are_complete_and_clean() {
        for (name, blocks) in all() {
            assert!(matches!(blocks.first(), Some(Block::Title(_))), "{name}: 第一块应是标题");
            for text in texts(&blocks) {
                assert!(!text.contains("**") && !text.contains('`') && !text.contains("]("), "{name}: 内联标记没有处理干净: {text}");
                for left_over in [
                    "[Developer",
                    "[Project",
                    "[Governing",
                    "[Effective",
                    "[Document",
                    "[LICENSE",
                    "[LGPL",
                    "[Asset",
                    "[Prevailing",
                    "[Dispute",
                    "[Legal",
                    "法律审查",
                    "@",
                ] {
                    assert!(!text.contains(left_over), "{name}: 残留了 {left_over}: {text}");
                }
            }
            for block in &blocks {
                if let Block::Table { header, rows } = block {
                    assert!(rows.iter().all(|r| r.len() == header.len()), "{name}: 表格每行的列数要和表头一致");
                }
            }
        }
    }

    #[test]
    fn terms_carry_the_official_non_affiliation_statement() {
        for (name, blocks) in all().into_iter().take(2) {
            let statement = "NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.";
            assert!(blocks.contains(&Block::Quote(statement.into())), "{name}: 缺少官方非关联声明");
        }
    }
}
