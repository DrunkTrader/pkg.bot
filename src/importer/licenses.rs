//! Repology dump has a large number of ridiculous license strings. This module
//! applies a bunch of string heuristics to normalize them.

const NAMES: &[&str] = &[
    "MIT",
    "MIT-0",
    "ISC",
    "BSD",
    "0BSD",
    "Apache",
    "Apache-1.0",
    "Apache-1.1",
    "Apache-2.0",
    "MPL-1.0",
    "MPL-1.1",
    "MPL-2.0",
    "Artistic",
    "Artistic-1.0",
    "Artistic-2.0",
    "Artistic-1.0-Perl",
    "Zlib",
    "Unlicense",
    "PostgreSQL",
    "BSL-1.0",
    "Python-2.0",
    "OFL",
    "OFL-1.0",
    "OFL-1.1",
    "LPPL",
    "LPPL-1.3",
    "LPPL-1.3c",
    "NCSA",
    "X11",
    "CC0-1.0",
    "CC-BY-3.0",
    "CC-BY-4.0",
    "CC-BY-SA-3.0",
    "CC-BY-SA-4.0",
    "Unknown",
    "Custom",
    "Unfree",
    "Proprietary",
    "Freeware",
];

pub(super) fn normalize(values: Option<Vec<String>>) -> Vec<String> {
    let mut out: Vec<_> = values
        .unwrap_or_default()
        .iter()
        .flat_map(|s| extract(s))
        .filter(|s| !s.is_empty())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn extract(raw: &str) -> Vec<String> {
    let cleaned = raw.replace(
        ['(', ')', '[', ']', '\'', '"', '|', '&', ',', ';', '/'],
        " ",
    );
    let tokens: Vec<_> = cleaned.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        i += 1;
        if token.eq_ignore_ascii_case("AND") || token.eq_ignore_ascii_case("OR") {
            continue;
        }
        if token.eq_ignore_ascii_case("WITH") {
            if let Some(exception) = tokens.get(i).filter(|t| {
                !["AND", "OR", "WITH"]
                    .iter()
                    .any(|op| t.eq_ignore_ascii_case(op))
            }) {
                if let Some(license) = out.last_mut() {
                    license.push_str(" WITH ");
                    license.push_str(exception);
                    i += 1;
                }
            }
            continue;
        }

        // Keep recognized multiword names together before splitting whitespace
        // lists (including Gentoo's `|| ( MIT Apache-2.0 )` syntax).
        let mut value = None;
        for count in (2..=3).rev() {
            let start = i - 1;
            if start + count <= tokens.len() {
                let phrase = tokens[start..start + count].join(" ");
                let name = get_canonical(&phrase);
                if name != phrase && matches!(name.as_str(), "Apache-2.0" | "Public Domain") {
                    value = Some(name);
                    i = start + count;
                    break;
                }
                if phrase.eq_ignore_ascii_case("public domain") {
                    value = Some("Public Domain".into());
                    i = start + count;
                    break;
                }
            }
        }
        let name = value.unwrap_or_else(|| get_canonical(token));
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

fn get_canonical(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('(') || raw.ends_with(')') {
        let inner = raw.trim_start_matches('(').trim_end_matches(')');
        return format!(
            "{}{}{}",
            "(".repeat(raw.len() - raw.trim_start_matches('(').len()),
            get_canonical(inner),
            ")".repeat(raw.len() - raw.trim_end_matches(')').len())
        );
    }
    let s = if raw
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("spdx:"))
    {
        raw[5..].trim()
    } else if raw
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("custom:"))
    {
        raw[7..].trim()
    } else {
        raw
    };
    if s.get(..11)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("LicenseRef-"))
    {
        return s[11..].trim().into();
    }
    let key: String = s
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !matches!(c, '-' | '_'))
        .map(|c| c.to_ascii_lowercase())
        .collect();

    for family in ["AGPL", "LGPL", "GPL"] {
        if let Some(rest) = key.strip_prefix(&family.to_ascii_lowercase()) {
            let rest = rest.strip_prefix('v').unwrap_or(rest);
            let (version, suffix) = if let Some(v) = rest.strip_suffix("orlater") {
                (v, "-or-later")
            } else if let Some(v) = rest.strip_suffix('+') {
                (v, "-or-later")
            } else if let Some(v) = rest.strip_suffix("only") {
                (v, "-only")
            } else {
                (rest, "-only")
            };
            if rest.is_empty() {
                return family.into();
            }
            let version = match version {
                "1" | "1.0" => "1.0",
                "2" | "2.0" => "2.0",
                "2.1" => "2.1",
                "3" | "3.0" => "3.0",
                _ => continue,
            };
            let valid = match family {
                "AGPL" => matches!(version, "1.0" | "3.0"),
                "LGPL" => matches!(version, "2.0" | "2.1" | "3.0"),
                _ => matches!(version, "1.0" | "2.0" | "3.0"),
            };
            if valid {
                return format!("{family}-{version}{suffix}");
            }
        }
    }

    let alias = match key.as_str() {
        "asl2" | "asl2.0" | "apache2" | "apachelicense2.0" => "Apache-2.0",
        "bsd2" | "bsd2clause" => "BSD-2-Clause",
        "bsd3" | "bsd3clause" => "BSD-3-Clause",
        "bsd4" | "bsd4clause" => "BSD-4-Clause",
        "publicdomain" => "Public Domain",
        "artistic1" => "Artistic-1.0",
        "artistic2" => "Artistic-2.0",
        _ => "",
    };
    if !alias.is_empty() {
        return alias.into();
    }

    // Case and separator variants of known identifiers. Custom license identifiers, leave them untouched.
    for name in NAMES {
        if name.to_ascii_lowercase().replace('-', "") == key {
            return (*name).into();
        }
    }
    s.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_and_deduplication() {
        let values = [
            " agpl-3 ",
            "AGPLv3",
            "AGPL-3.0-only",
            "agpl",
            "agpl3+",
            "",
            "  ",
        ];
        assert_eq!(
            normalize(Some(values.map(String::from).to_vec())),
            ["AGPL", "AGPL-3.0-only", "AGPL-3.0-or-later"]
        );
        for (raw, expected) in [
            ("ASL 2.0", "Apache-2.0"),
            ("BSD3", "BSD-3-Clause"),
            ("spdx:mit", "MIT"),
            ("custom:mit", "MIT"),
            ("LicenseRef-My-License", "My-License"),
            (
                "licenseref-Slint-Royalty-free-2.0",
                "Slint-Royalty-free-2.0",
            ),
            ("spdx:LicenseRef-Example", "Example"),
            ("LicenseRef-", ""),
            ("CUSTOM: Example", "Example"),
            ("custom:Example", "Example"),
            ("custom:", ""),
            ("LGPLv2.1+", "LGPL-2.1-or-later"),
            ("public-domain", "Public Domain"),
            ("lppl1.3c", "LPPL-1.3c"),
        ] {
            assert_eq!(get_canonical(raw), expected);
        }
    }

    #[test]
    fn flatten_expressions() {
        let input = vec![
            "\"|| ( MIT Apache-2.0 )\"".into(),
            "(GPLv2+ or 'MIT') AND BSD-2-Clause (BSD-2-Clause OR MIT/Apache-2.0)".into(),
            "Apache-2.0 WITH LLVM-exception AND custom:Zlib".into(),
        ];
        assert_eq!(
            normalize(Some(input)),
            [
                "Apache-2.0",
                "Apache-2.0 WITH LLVM-exception",
                "BSD-2-Clause",
                "GPL-2.0-or-later",
                "MIT",
                "Zlib",
            ]
        );
        assert_eq!(
            extract("ASL 2.0 AND Public Domain AND Apache License 2.0"),
            ["Apache-2.0", "Public Domain", "Apache-2.0"]
        );
        assert_eq!(
            extract("LicenseRef-My-License AND GPL-4.0"),
            ["My-License", "GPL-4.0"]
        );
        assert!(normalize(None).is_empty());
        assert!(normalize(Some(vec!["|| () AND OR custom:".into()])).is_empty());
    }
}
