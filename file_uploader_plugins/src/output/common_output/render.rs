#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Markdown,
    Html,
    Link,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "markdown" => Ok(Self::Markdown),
            "html" => Ok(Self::Html),
            "link" => Ok(Self::Link),
            other => Err(format!("unknown output format: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Link => "link",
        }
    }
}

pub struct Values<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub size: usize,
    pub file_type: &'a str,
}

pub fn render(
    format: OutputFormat,
    values: &Values<'_>,
    template: Option<&str>,
) -> Result<String, String> {
    if let Some(tpl) = template {
        if !tpl.is_empty() {
            return Ok(tpl
                .replace("{url}", values.url)
                .replace("{name}", values.name)
                .replace("{size}", &values.size.to_string())
                .replace("{type}", values.file_type));
        }
    }
    Ok(match format {
        OutputFormat::Markdown => {
            let name = escape_md(values.name);
            let url = encode_md_url(values.url);
            if values.file_type.starts_with("image/") {
                format!("![{name}]({url})")
            } else {
                format!("[{name}]({url})")
            }
        }
        OutputFormat::Html => {
            let url = escape_html(values.url);
            let alt = escape_html(values.name);
            format!("<img src=\"{url}\" alt=\"{alt}\">")
        }
        OutputFormat::Link => values.url.to_string(),
    })
}

fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '[' | ']' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn encode_md_url(s: &str) -> String {
    s.replace(' ', "%20").replace('(', "%28").replace(')', "%29")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_uses_image_syntax_and_escapes_name() {
        let values = Values {
            name: "a[1].png",
            url: "https://x/a (1).png",
            size: 12,
            file_type: "image/png",
        };
        assert_eq!(
            render(OutputFormat::Markdown, &values, None).unwrap(),
            "![a\\[1\\].png](https://x/a%20%281%29.png)"
        );
    }

    #[test]
    fn markdown_uses_link_for_non_image() {
        let values = Values {
            name: "a.pdf",
            url: "https://x/a.pdf",
            size: 12,
            file_type: "application/pdf",
        };
        assert_eq!(
            render(OutputFormat::Markdown, &values, None).unwrap(),
            "[a.pdf](https://x/a.pdf)"
        );
    }

    #[test]
    fn html_escapes_name_and_url() {
        let values = Values {
            name: "a&\".png",
            url: "https://x/?a=1&b=2",
            size: 12,
            file_type: "image/png",
        };
        assert_eq!(
            render(OutputFormat::Html, &values, None).unwrap(),
            "<img src=\"https://x/?a=1&amp;b=2\" alt=\"a&amp;&quot;.png\">"
        );
    }

    #[test]
    fn link_returns_url() {
        let values = Values {
            name: "a",
            url: "https://x/a",
            size: 12,
            file_type: "text/plain",
        };
        assert_eq!(
            render(OutputFormat::Link, &values, None).unwrap(),
            "https://x/a"
        );
    }

    #[test]
    fn template_overrides_format_and_replaces_all_values() {
        let values = Values {
            name: "a.png",
            url: "https://x/a.png",
            size: 12,
            file_type: "image/png",
        };
        assert_eq!(
            render(
                OutputFormat::Html,
                &values,
                Some("{name}|{url}|{size}|{type}")
            )
            .unwrap(),
            "a.png|https://x/a.png|12|image/png"
        );
    }

    #[test]
    fn parse_recognizes_all_formats_and_rejects_unknown() {
        assert_eq!(OutputFormat::parse("markdown").unwrap(), OutputFormat::Markdown);
        assert_eq!(OutputFormat::parse("html").unwrap(), OutputFormat::Html);
        assert_eq!(OutputFormat::parse("link").unwrap(), OutputFormat::Link);
        assert!(OutputFormat::parse("pdf").is_err());
    }
}
