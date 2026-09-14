//! Package URL templates.
//!
//! Helpers to render package permalinks from fields and {placeholder} URL templates.
//! Some repositories have special cases to handle like nix.

/// Package fields a template can substitute.
#[derive(Debug, Default, Clone, Copy)]
pub struct Fields<'a> {
    pub slug: &'a str,
    pub trackname: Option<&'a str>,
    pub name: &'a str,
    pub pkg_base: Option<&'a str>,
    pub version: Option<&'a str>,
    pub subrepo: Option<&'a str>,
    pub arch: Option<&'a str>,
}

impl<'a> Fields<'a> {
    fn get(&self, name: &str) -> Option<&'a str> {
        let v = match name {
            "slug" => Some(self.slug),
            "trackname" => self.trackname,
            "name" => Some(self.name),
            "pkg_base" => self.pkg_base,
            "version" => self.version,
            "subrepo" => self.subrepo,
            "arch" => self.arch,
            _ => None,
        }?;

        (!v.is_empty()).then_some(v)
    }
}

/// Repology's field mapping.
const FIELDS: [(&str, &str); 5] = [
    ("srcname", "pkg_base"),
    ("binname", "name"),
    ("rawversion", "version"),
    ("subrepo", "subrepo"),
    ("arch", "arch"),
];

const FILTERS: [&str; 5] = [
    "quote",
    "basename",
    "first_letter",
    "lib_and_first_letter",
    "py_or_first_letter",
];

/// Rewrite a Repology `packagelinks` to custom template.
/// Returns None if it references a field or filter we can't fill.
pub fn parse_repology(tmpl: &str) -> Option<String> {
    rewrite(tmpl, |name, filter| {
        let (_, ours) = FIELDS.iter().find(|(their, _)| *their == name)?;
        if filter.is_some_and(|f| !FILTERS.contains(&f)) {
            return None;
        }

        Some(match filter {
            Some(f) => format!("{{{ours}|{f}}}"),
            None => format!("{{{ours}}}"),
        })
    })
}

/// Expand a URL template with package fields. Returns None if any placeholder is unset.
pub fn expand(tmpl: &str, f: &Fields) -> Option<String> {
    rewrite(tmpl, |name, filter| apply(filter, f.get(name)?))
}

/// Iterate `{...}` placeholders and replace with `sub(name, filter)`.
fn rewrite(
    tmpl: &str,
    mut sub: impl FnMut(&str, Option<&str>) -> Option<String>,
) -> Option<String> {
    let mut out = String::with_capacity(tmpl.len() + 16);
    let mut rest = tmpl;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);

        let tail = &rest[open + 1..];
        let close = tail.find('}')?;
        let (name, filter) = match tail[..close].split_once('|') {
            Some((n, f)) => (n, Some(f)),
            None => (&tail[..close], None),
        };

        out.push_str(&sub(name, filter)?);
        rest = &tail[close + 1..];
    }

    out.push_str(rest);
    Some(out)
}

/// Apply one of Repology's template filters to a substituted value.
fn apply(filter: Option<&str>, v: &str) -> Option<String> {
    Some(match filter {
        None => v.to_string(),
        Some("quote") => urlencode(v),

        // 'security/sudo' -> 'sudo'. Gentoo, macports etc. use category/name
        // srcnames, and the recipe file is named after the trailing part.
        Some("basename") => v.rsplit('/').next()?.to_string(),

        Some("first_letter") => v.chars().next()?.to_string(),

        // Debian: 'libsoup3' -> 'libs', 'zsh' -> 'z'.
        Some("lib_and_first_letter") => match v.strip_prefix("lib") {
            Some(rest) => format!("lib{}", rest.chars().next()?),
            None => v.chars().next()?.to_string(),
        },

        // Solus: 'python-tomli-w' -> 'py', 'libfprint' -> 'l'.
        Some("py_or_first_letter") => match v.starts_with("py") {
            true => "py".to_string(),
            false => v.chars().next()?.to_string(),
        },

        Some(_) => return None,
    })
}

/// Urlencode the given string.
pub fn urlencode(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for b in v.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Fields<'static> {
        Fields {
            slug: "firefox",
            trackname: Some("firefox"),
            name: "firefox",
            pkg_base: Some("firefox"),
            version: Some("154.0.1-1"),
            subrepo: Some("extra"),
            arch: Some("x86_64"),
        }
    }

    #[test]
    fn rewrites_repology_fields() {
        assert_eq!(
            parse_repology("https://archlinux.org/packages/{subrepo}/{arch}/{binname}/").as_deref(),
            Some("https://archlinux.org/packages/{subrepo}/{arch}/{name}/")
        );
        assert_eq!(
            parse_repology("https://x/{srcname}/{srcname|basename}-{rawversion}.ebuild").as_deref(),
            Some("https://x/{pkg_base}/{pkg_base|basename}-{version}.ebuild")
        );
    }

    #[test]
    fn drops_templates_we_cannot_fill() {
        // nixpkgs source position and centos branch suffix aren't in the repology dump.
        assert_eq!(parse_repology("https://x/{?posfile}#L{?posline}"), None);
        assert_eq!(
            parse_repology("https://x/{srcname}/-/tree/c7{centossuffix}"),
            None
        );
        assert_eq!(parse_repology("https://x/{srcname|unknown_filter}"), None);
        assert_eq!(parse_repology("https://x/{unclosed"), None);
    }

    #[test]
    fn expands_fields() {
        let f = fields();
        assert_eq!(
            expand(
                "https://archlinux.org/packages/{subrepo}/{arch}/{name}/",
                &f
            )
            .as_deref(),
            Some("https://archlinux.org/packages/extra/x86_64/firefox/")
        );
        assert_eq!(
            expand("https://sources.debian.org/src/{pkg_base}/{version}/", &f).as_deref(),
            Some("https://sources.debian.org/src/firefox/154.0.1-1/")
        );

        // Zero placeholders is a valid URL.
        assert_eq!(expand("https://x/y", &f).as_deref(), Some("https://x/y"));
    }

    #[test]
    fn drops_urls_with_missing_fields() {
        let f = Fields {
            pkg_base: None,
            ..fields()
        };
        assert_eq!(
            expand("https://packages.gentoo.org/packages/{pkg_base}", &f),
            None
        );

        let f = Fields {
            subrepo: Some(""),
            ..fields()
        };
        assert_eq!(expand("https://x/{subrepo}/{name}", &f), None);
    }

    #[test]
    fn applies_filters() {
        let f = Fields {
            pkg_base: Some("app-accessibility/accerciser"),
            ..fields()
        };
        assert_eq!(
            expand("{pkg_base|basename}", &f).as_deref(),
            Some("accerciser")
        );

        let f = Fields {
            pkg_base: Some("lolcat++"),
            ..fields()
        };
        assert_eq!(
            expand("?h={pkg_base|quote}", &f).as_deref(),
            Some("?h=lolcat%2B%2B")
        );

        let f = Fields {
            pkg_base: Some("libsoup3"),
            ..fields()
        };
        assert_eq!(
            expand("{pkg_base|lib_and_first_letter}", &f).as_deref(),
            Some("libs")
        );
        assert_eq!(expand("{pkg_base|first_letter}", &f).as_deref(), Some("l"));
        assert_eq!(
            expand("{pkg_base|py_or_first_letter}", &f).as_deref(),
            Some("l")
        );

        let f = Fields {
            pkg_base: Some("python-tomli-w"),
            ..fields()
        };
        assert_eq!(
            expand("{pkg_base|py_or_first_letter}", &f).as_deref(),
            Some("py")
        );
    }
}
