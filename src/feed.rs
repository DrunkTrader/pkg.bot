//! RSS 2.0 feed generation.

use std::io;

use quick_xml::{
    events::{BytesDecl, BytesText, Event},
    writer::Writer,
};

/// An RSS channel and its items.
#[derive(Default)]
pub struct Feed {
    pub title: String,
    pub link: String,
    pub self_link: String,
    pub description: String,
    pub items: Vec<Item>,
}

/// A single `<item>` in a feed.
#[derive(Default)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub guid: String,
    pub permalink: bool,

    pub description: String,
    pub categories: Vec<String>,
    pub pub_date: String,
}

impl Feed {
    /// Serialize the feed as an RSS doc.
    pub fn to_xml(&self) -> io::Result<String> {
        let mut w = Writer::new_with_indent(Vec::new(), b' ', 2);
        w.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;

        w.create_element("rss")
            .with_attributes([
                ("version", "2.0"),
                ("xmlns:atom", "http://www.w3.org/2005/Atom"),
            ])
            .write_inner_content(|w| {
                w.create_element("channel").write_inner_content(|w| {
                    text(w, "title", &self.title)?;
                    text(w, "link", &self.link)?;
                    text(w, "description", &self.description)?;
                    text(w, "language", "en")?;
                    text(w, "generator", "pkgbot")?;

                    w.create_element("atom:link")
                        .with_attributes([
                            ("href", self.self_link.as_str()),
                            ("rel", "self"),
                            ("type", "application/rss+xml"),
                        ])
                        .write_empty()?;

                    for item in &self.items {
                        item.write(w)?;
                    }

                    Ok(())
                })?;

                Ok(())
            })?;

        String::from_utf8(w.into_inner()).map_err(io::Error::other)
    }
}

impl Item {
    fn write(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
        w.create_element("item").write_inner_content(|w| {
            text(w, "title", &self.title)?;
            text(w, "link", &self.link)?;

            w.create_element("guid")
                .with_attribute(("isPermaLink", if self.permalink { "true" } else { "false" }))
                .write_text_content(BytesText::new(&self.guid))?;

            if !self.description.is_empty() {
                text(w, "description", &self.description)?;
            }

            for c in &self.categories {
                text(w, "category", c)?;
            }

            if let Some(d) = rfc822(&self.pub_date) {
                text(w, "pubDate", &d)?;
            }

            Ok(())
        })?;

        Ok(())
    }
}

/// Write a `<name>text</name>` element.
fn text(w: &mut Writer<Vec<u8>>, name: &str, val: &str) -> io::Result<()> {
    w.create_element(name)
        .write_text_content(BytesText::new(val))?;

    Ok(())
}

/// Convert an ISO-8601 timestamp to the RFC 822 form `<pubDate>` requires.
fn rfc822(ts: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|d| d.to_rfc2822())
}
