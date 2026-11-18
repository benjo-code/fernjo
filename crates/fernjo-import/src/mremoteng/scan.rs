//! A linear byte scan that runs before `roxmltree` sees the text.
//!
//! `roxmltree` has no limit on attributes per element or on namespace declarations, and both are
//! superlinear: it compares every new attribute with the earlier ones of the same tag (one tag with
//! 100 000 attributes takes 21 s) and every prefixed name is resolved against all declarations in
//! scope (5 000 nested `xmlns:pN` elements take 55 s). The scan finds the two numbers the caps
//! need, the most attributes of any start tag and the number of `xmlns` names, in one pass without
//! allocating.
//!
//! It follows the tokens `roxmltree` follows, so a file that parses cannot hide anything from it:
//! comments end at the first `-->` after `<!--`, CDATA at the first `]]>`, processing instructions
//! at the first `?>`, and inside a start tag a quoted value ends at the first quote of the same kind
//! (an attribute value may not contain `<`, so there is no other way to end it). Every attribute
//! has a quoted value, so counting the opening quotes counts the attributes. Where the scan and the
//! parser could disagree (malformed input) the parser refuses the file anyway, and the scan only
//! ever errs towards counting more, never fewer.

use crate::error::ImportError;

/// What the scan found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Measure {
    /// Most quoted attribute values in one start tag.
    pub max_attributes: usize,
    /// Occurrences of `xmlns` as (part of) a name inside start tags, quoted values excluded.
    pub xmlns: usize,
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Measures `bytes` (a UTF-8 XML document or fragment) in time linear in its length.
pub(super) fn measure(bytes: &[u8]) -> Measure {
    let mut m = Measure::default();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b != b'<' {
            i += 1;
            continue;
        }
        let rest = &bytes[i..];
        // each of these skips to the end of its construct; an unterminated one ends the scan
        // (the parser refuses it)
        let end_of_construct = if rest.starts_with(b"<!--") {
            Some(find(bytes, i + 4, b"-->").map(|p| p + 3))
        } else if rest.starts_with(b"<![CDATA[") {
            Some(find(bytes, i + 9, b"]]>").map(|p| p + 3))
        } else if rest.starts_with(b"<?") {
            Some(find(bytes, i + 2, b"?>").map(|p| p + 2))
        } else if rest.starts_with(b"<!") || rest.starts_with(b"</") {
            // a DTD is refused by the parser; an end tag has no attributes
            Some(find(bytes, i + 2, b">").map(|p| p + 1))
        } else {
            None
        };
        match end_of_construct {
            Some(Some(next)) => {
                i = next;
                continue;
            }
            Some(None) => return m,
            None => {}
        }
        // a start tag: `<` name attributes `>` or `/>`
        i += 1;
        let mut attributes = 0usize;
        loop {
            match bytes.get(i) {
                None => {
                    m.max_attributes = m.max_attributes.max(attributes);
                    return m;
                }
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(q @ (b'"' | b'\'')) => {
                    attributes += 1;
                    match find(bytes, i + 1, std::slice::from_ref(q)) {
                        Some(end) => i = end + 1,
                        None => {
                            m.max_attributes = m.max_attributes.max(attributes);
                            return m;
                        }
                    }
                }
                Some(b'x') => {
                    if bytes[i..].starts_with(b"xmlns") {
                        m.xmlns += 1;
                    }
                    i += 1;
                }
                Some(_) => i += 1,
            }
        }
        m.max_attributes = m.max_attributes.max(attributes);
    }
    m
}

/// Refuses `text` when a start tag carries more than `max_attributes` attributes or the document
/// declares more than `max_xmlns` namespaces. Call it before parsing.
pub(super) fn check(
    text: &str,
    max_attributes: usize,
    max_xmlns: usize,
) -> Result<(), ImportError> {
    let m = measure(text.as_bytes());
    if m.max_attributes > max_attributes {
        return Err(ImportError::TooComplex(format!(
            "an XML element has more than {max_attributes} attributes"
        )));
    }
    if m.xmlns > max_xmlns {
        return Err(ImportError::TooComplex(format!(
            "the XML declares more than {max_xmlns} namespaces"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(text: &str) -> Measure {
        measure(text.as_bytes())
    }

    #[test]
    fn counts_attributes_and_namespace_names_per_tag() {
        let r = m(r#"<a x="1" y='2'><b p="1" q="2" r="3"/><c/></a>"#);
        assert_eq!(r.max_attributes, 3);
        assert_eq!(r.xmlns, 0);
        let r = m(r#"<a xmlns:p="u" xmlns="v" p:x="1"><b xmlns:q="w"/></a>"#);
        assert_eq!((r.max_attributes, r.xmlns), (3, 3));
    }

    #[test]
    fn quoted_values_hide_nothing_and_add_nothing() {
        // `>`, the other quote, `<!--`, `]]>` and `xmlns` inside values
        let r = m(r#"<a t=">" u='"' v="<!--" w=']]>' x="xmlns:a xmlns:b"/>"#);
        assert_eq!((r.max_attributes, r.xmlns), (5, 0));
    }

    #[test]
    fn comments_cdata_and_instructions_are_skipped_not_counted() {
        let r = m(
            r#"<?pi <a x="1" xmlns:q="u"?><!-- <a x="1" xmlns:q="u"> --><a><![CDATA[ <a x="1" xmlns:q="u"> ]]></a>"#,
        );
        assert_eq!((r.max_attributes, r.xmlns), (0, 0));
        // and a real tag after them is seen
        let r = m(r#"<!-- " --><![CDATA[ ' ]]><?p "?><a x="1" y="2"/>"#);
        assert_eq!(r.max_attributes, 2);
    }

    #[test]
    fn comment_edge_cases_follow_the_parser() {
        // `<!-->` does not end itself: the comment runs to the first `-->` after `<!--`
        let r = m(r#"<!--> <a x="1" y="2" z="3"/> --><b p="1"/>"#);
        assert_eq!(r.max_attributes, 1);
        assert_eq!(
            m(r#"<!---><a x="1"/>--><b p="1" q="2"/>"#).max_attributes,
            2
        );
    }

    #[test]
    fn unterminated_constructs_end_the_scan_without_panicking() {
        for t in [
            "<!-- never closed <a x=\"1\"/>",
            "<![CDATA[ never closed",
            "<? never closed",
            "<a x=\"never closed",
            "<a x=\"1\" y",
            "<",
            "<!",
            "</",
            "",
        ] {
            let _ = m(t);
        }
    }

    #[test]
    fn multibyte_text_is_fine() {
        let r = m("<a n=\"日本語 ünï\" m='😀'/>日本語<b/>");
        assert_eq!(r.max_attributes, 2);
    }

    // ---- differential check against roxmltree ----

    /// xorshift, so the check is deterministic without a dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    const VALUES: &[&str] = &[
        "",
        "plain",
        ">",
        "-->",
        "]]>",
        "?>",
        "xmlns",
        "xmlns:z=",
        "&amp;&lt;",
        "'",
        "\"",
        "日本",
        "/>",
        " x=1 ",
    ];

    /// Builds a random well-formed element tree and returns (text, most attributes on one
    /// element, number of namespace declarations).
    fn element(
        rng: &mut Rng,
        depth: usize,
        ids: &mut usize,
        max_attrs: &mut usize,
        ns: &mut usize,
    ) -> String {
        let mut out = String::new();
        *ids += 1;
        let name = format!("e{}", *ids);
        out.push_str(&format!("<{name}"));
        let attrs = rng.below(6) as usize;
        let declared = rng.below(3) as usize;
        let mut total = attrs;
        for _ in 0..attrs {
            *ids += 1;
            let v = VALUES[rng.below(VALUES.len() as u64) as usize];
            // a value holding `"` must use single quotes and the other way round
            let quote = if v.contains('"') { '\'' } else { '"' };
            let v = if quote == '\'' && v.contains('\'') {
                "x"
            } else {
                v
            };
            out.push_str(&format!(" a{}={quote}{v}{quote}", *ids));
        }
        for _ in 0..declared {
            *ids += 1;
            out.push_str(&format!(" xmlns:n{}=\"urn:{}\"", *ids, *ids));
            *ns += 1;
            total += 1;
        }
        *max_attrs = (*max_attrs).max(total);
        if depth == 0 || rng.below(3) == 0 {
            out.push_str("/>");
            return out;
        }
        out.push('>');
        for _ in 0..rng.below(4) {
            match rng.below(6) {
                0 => out.push_str(&format!(
                    "<!-- {} -->",
                    ["<a x=\"", "'", ">", "xmlns:c=\"u\""][rng.below(4) as usize]
                )),
                1 => out.push_str(&format!(
                    "<![CDATA[ {} ]]>",
                    ["<a x=\"", "'", ">", "xmlns:c=\"u\" -->"][rng.below(4) as usize]
                )),
                2 => out.push_str("<?pi data \" ' > xmlns?>"),
                3 => out.push_str("text &amp; > ' \" more"),
                _ => out.push_str(&element(rng, depth - 1, ids, max_attrs, ns)),
            }
        }
        out.push_str(&format!("</{name}>"));
        out
    }

    #[test]
    fn the_scan_never_counts_fewer_than_the_parser_sees() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut checked = 0;
        for _ in 0..3000 {
            let (mut ids, mut max_attrs, mut ns) = (0, 0, 0);
            let text = element(&mut rng, 4, &mut ids, &mut max_attrs, &mut ns);
            let opts = roxmltree::ParsingOptions {
                allow_dtd: false,
                ..Default::default()
            };
            let doc = roxmltree::Document::parse_with_options(&text, opts)
                .unwrap_or_else(|e| panic!("generator bug: {e}\n{text}"));
            // ground truth from the parser itself
            let dom_attrs = doc
                .descendants()
                .filter(|n| n.is_element())
                .map(|n| n.attributes().count())
                .max()
                .unwrap_or(0);
            let r = measure(text.as_bytes());
            assert!(r.max_attributes >= dom_attrs, "{text}");
            assert_eq!(r.max_attributes, max_attrs, "{text}");
            assert_eq!(r.xmlns, ns, "{text}");
            checked += 1;
        }
        assert_eq!(checked, 3000);
    }
}
