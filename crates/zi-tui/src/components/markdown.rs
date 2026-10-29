// Markdown rendered as plain wrapped lines.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::component::Component;
use crate::wrap::wrap_text;

/// Renders markdown source as plain text lines (no inline styling).
pub struct Markdown {
    source: String,
}

impl Markdown {
    /// Creates a markdown component from source text.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }

    /// Replaces the markdown source.
    pub fn set_source(&mut self, source: impl Into<String>) {
        self.source = source.into();
    }

    fn markdown_to_plain(source: &str) -> String {
        let mut out = String::new();
        let parser = Parser::new_ext(source, Options::all());
        for event in parser {
            match event {
                Event::Text(text) | Event::Code(text) => out.push_str(&text),
                Event::SoftBreak | Event::HardBreak => out.push('\n'),
                Event::Rule => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("---\n");
                }
                Event::Start(tag) => push_block_prefix(&mut out, &tag),
                Event::End(tag) => push_block_suffix(&mut out, &tag),
                Event::Html(html) | Event::InlineHtml(html) => out.push_str(&html),
                Event::FootnoteReference(name) => {
                    out.push('[');
                    out.push_str(&name);
                    out.push(']');
                }
                _ => {}
            }
        }
        out
    }
}

fn push_block_prefix(out: &mut String, tag: &Tag<'_>) {
    match tag {
        Tag::Heading { .. } => {}
        Tag::Paragraph => {}
        Tag::List(_) => {}
        Tag::Item => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("• ");
        }
        Tag::BlockQuote(_) => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("> ");
        }
        _ => {}
    }
}

fn push_block_suffix(out: &mut String, tag: &TagEnd) {
    match tag {
        TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_) => {
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        TagEnd::List(_) => {
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        _ => {}
    }
}

impl Component for Markdown {
    fn render(&self, width: usize) -> Vec<String> {
        if self.source.trim().is_empty() {
            return Vec::new();
        }
        let plain = Self::markdown_to_plain(&self.source);
        wrap_text(&plain, width.max(1))
    }
}
