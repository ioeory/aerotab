//! CSS `font-family` stacks for glyphon/cosmic-text: pick the first installed face.

/// Split a CSS font-family list into candidates (quotes stripped, generics kept).
pub fn parse_css_font_families(stack: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in stack.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ',' => {
                push_family(&mut out, &cur);
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    push_family(&mut out, &cur);
    out
}

fn push_family(out: &mut Vec<String>, raw: &str) {
    let name = raw.trim().trim_matches(|c| c == '\'' || c == '"').trim();
    if !name.is_empty() {
        out.push(name.to_string());
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod glyphon_resolve {
    use super::parse_css_font_families;
    use glyphon::{Family, FontSystem};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ResolvedFamily {
        Monospace,
        Serif,
        SansSerif,
        Cursive,
        Fantasy,
        Named(String),
    }

    impl ResolvedFamily {
        pub fn as_glyphon(&self) -> Family<'_> {
            match self {
                Self::Monospace => Family::Monospace,
                Self::Serif => Family::Serif,
                Self::SansSerif => Family::SansSerif,
                Self::Cursive => Family::Cursive,
                Self::Fantasy => Family::Fantasy,
                Self::Named(name) => Family::Name(name.as_str()),
            }
        }
    }

    fn family_installed(fonts: &FontSystem, name: &str) -> bool {
        fonts.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(family, _)| family.eq_ignore_ascii_case(name))
        })
    }

    /// Resolve a CSS stack (or single name) to the first installed / generic family.
    pub fn resolve_font_family(fonts: &FontSystem, stack: &str) -> ResolvedFamily {
        for name in parse_css_font_families(stack) {
            match name.to_ascii_lowercase().as_str() {
                "monospace" => return ResolvedFamily::Monospace,
                "serif" => return ResolvedFamily::Serif,
                "sans-serif" => return ResolvedFamily::SansSerif,
                "cursive" => return ResolvedFamily::Cursive,
                "fantasy" => return ResolvedFamily::Fantasy,
                _ if family_installed(fonts, &name) => return ResolvedFamily::Named(name),
                _ => {}
            }
        }
        for fallback in [
            "Cascadia Mono",
            "Cascadia Code",
            "Consolas",
            "Courier New",
            "Menlo",
        ] {
            if family_installed(fonts, fallback) {
                return ResolvedFamily::Named(fallback.to_string());
            }
        }
        ResolvedFamily::Monospace
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use glyphon_resolve::resolve_font_family;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_css_stack() {
        assert_eq!(
            parse_css_font_families("JetBrains Mono, Menlo, monospace"),
            vec!["JetBrains Mono", "Menlo", "monospace"]
        );
        assert_eq!(
            parse_css_font_families("'Courier New', \"Consolas\", monospace"),
            vec!["Courier New", "Consolas", "monospace"]
        );
    }
}
