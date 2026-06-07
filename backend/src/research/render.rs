//! Pure HTML renderer for research reports. Consumes the structured output of
//! the synthesize stage and produces one self-contained document: inline CSS
//! (light/dark via prefers-color-scheme), per-claim citation anchors, styled
//! comparison tables, inline-SVG bar charts (no JavaScript anywhere), embedded
//! data-URI images, and a numbered sources list.
//!
//! Security invariant: every model-provided string passes through [`escape`]
//! before entering the document. Structure (tags, anchors, SVG) is built only
//! from code — model text can never inject live HTML.

use serde::Deserialize;

/// The synthesize stage's JSON, mirrored leniently: any missing field
/// defaults so partial model output still renders.
#[derive(Debug, Default, Deserialize)]
pub struct ReportDoc {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub intro: String,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub tables: Vec<Table>,
    #[serde(default)]
    pub charts: Vec<Chart>,
    #[serde(default)]
    pub images: Vec<ImagePick>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Section {
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Paragraph {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub cites: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Table {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Chart {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub values: Vec<f64>,
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ImagePick {
    /// Candidate id (I1, I2…) — the primary reference.
    #[serde(default)]
    pub id: String,
    /// Legacy exact-URL reference, still honoured when given.
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub caption: String,
}

/// A research source: web page or internal material.
#[derive(Debug, Clone)]
pub struct Source {
    /// The id the model cites (S1.., doc:…, email:…, memory:…, chat:…).
    pub id: String,
    /// Human label (page title, filename, subject…).
    pub label: String,
    /// Link for web sources; None for internal material.
    pub url: Option<String>,
}

/// An image the orchestrator fetched and encoded.
pub struct EmbeddedImage {
    pub caption: String,
    /// Complete `data:image/…;base64,…` URI.
    pub data_uri: String,
}

/// HTML-escape model-provided text. `&` first, then the rest.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Superscript citation links for a paragraph: [1][3] → anchors into Sources.
fn citation_links(cites: &[String], sources: &[Source]) -> String {
    let mut out = String::new();
    for cite in cites {
        if let Some(n) = sources.iter().position(|s| &s.id == cite) {
            out.push_str(&format!("<sup><a href=\"#src-{}\">[{}]</a></sup>", n + 1, n + 1));
        }
    }
    out
}

/// Horizontal bar chart as inline SVG. Pure geometry, no scripts.
fn bar_chart_svg(chart: &Chart) -> String {
    const W: f64 = 640.0;
    const ROW_H: f64 = 30.0;
    const GUTTER: f64 = 170.0; // label column
    const VALUE_PAD: f64 = 64.0; // room for value text at bar end

    let n = chart.labels.len().min(chart.values.len());
    if n == 0 {
        return String::new();
    }
    let max = chart.values.iter().take(n).cloned().fold(0.0_f64, f64::max);
    let scale = if max > 0.0 { (W - GUTTER - VALUE_PAD) / max } else { 0.0 };
    let height = ROW_H * n as f64 + 6.0;

    let mut svg = format!(
        "<svg class=\"chart-svg\" viewBox=\"0 0 {W} {height}\" width=\"100%\" \
         role=\"img\" aria-label=\"{}\" xmlns=\"http://www.w3.org/2000/svg\">",
        escape(&chart.title)
    );
    for i in 0..n {
        let v = chart.values[i].max(0.0);
        let bar_w = v * scale;
        let y = i as f64 * ROW_H + 4.0;
        let unit = chart.unit.as_deref().unwrap_or("");
        let value_text = if v.fract() == 0.0 {
            format!("{}{}", v as i64, escape(unit))
        } else {
            format!("{v:.2}{}", escape(unit))
        };
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"bar-label\" text-anchor=\"end\">{}</text>\
             <rect x=\"{GUTTER}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"3\" class=\"bar\"/>\
             <text x=\"{:.1}\" y=\"{:.1}\" class=\"bar-value\">{}</text>",
            GUTTER - 8.0,
            y + ROW_H * 0.62,
            escape(&chart.labels[i]),
            y,
            bar_w,
            ROW_H - 10.0,
            GUTTER + bar_w + 6.0,
            y + ROW_H * 0.62,
            value_text,
        ));
    }
    svg.push_str("</svg>");
    svg
}

const CSS: &str = r#"
:root{--bg:#fdfdfb;--fg:#1d1d1f;--muted:#6b6b70;--rule:#e4e4e2;--surface:#f3f3f0;
--accent:#2456a6;--bar:#5b8def;color-scheme:light dark}
@media (prefers-color-scheme:dark){:root{--bg:#121214;--fg:#dededf;--muted:#8d8d93;
--rule:#2c2c30;--surface:#1c1c1f;--accent:#7ab0ff;--bar:#3a6adf}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);
font:16px/1.6 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif}
main{max-width:760px;margin:0 auto;padding:2.5rem 1.25rem 4rem}
header{border-bottom:1px solid var(--rule);padding-bottom:1.25rem;margin-bottom:1.5rem}
h1{font-size:1.7rem;line-height:1.25;margin:0 0 .4rem}
.meta{color:var(--muted);font-size:.82rem}
.intro{font-size:1.05rem;color:var(--fg)}
h2{font-size:1.18rem;margin:2rem 0 .6rem;padding-top:.4rem}
p{margin:.6rem 0}
sup a{color:var(--accent);text-decoration:none;font-size:.72em;padding:0 .08em}
figure{margin:1.4rem 0}
figcaption{color:var(--muted);font-size:.82rem;margin-top:.35rem;text-align:center}
figure img{max-width:100%;border-radius:8px;display:block;margin:0 auto}
table{border-collapse:collapse;width:100%;margin:1.2rem 0;font-size:.9rem}
caption{caption-side:top;text-align:left;font-weight:600;padding-bottom:.45rem}
th,td{border:1px solid var(--rule);padding:.45rem .6rem;text-align:left;vertical-align:top}
th{background:var(--surface)}
.chart{margin:1.4rem 0;background:var(--surface);border:1px solid var(--rule);
border-radius:8px;padding:1rem}
.chart h3{margin:0 0 .6rem;font-size:.95rem}
.bar{fill:var(--bar)}
.bar-label{font-size:12px;fill:var(--fg)}
.bar-value{font-size:12px;fill:var(--muted)}
.sources{border-top:1px solid var(--rule);margin-top:2.5rem;padding-top:1rem}
.sources h2{margin-top:0}
.sources ol{padding-left:1.4rem;font-size:.88rem}
.sources li{margin:.3rem 0}
.sources a{color:var(--accent)}
.src-kind{color:var(--muted)}
footer{color:var(--muted);font-size:.75rem;margin-top:3rem}
"#;

/// Plain-markdown rendering of a report for the documents-RAG corpus: the
/// substance (title, intro, sections, tables, chart values) without the HTML
/// boilerplate, data-URI images, or numbered source anchors that would
/// pollute chunk embeddings.
pub fn render_markdown(doc: &ReportDoc, sources: &[Source]) -> String {
    let mut out = String::new();
    let title = if doc.title.trim().is_empty() { "Research report" } else { doc.title.trim() };
    out.push_str(&format!("# {title}\n"));
    if !doc.intro.trim().is_empty() {
        out.push_str(&format!("\n{}\n", doc.intro.trim()));
    }
    for section in &doc.sections {
        out.push_str(&format!("\n## {}\n", section.heading.trim()));
        for p in &section.paragraphs {
            out.push_str(&format!("\n{}\n", p.text.trim()));
        }
    }
    for table in &doc.tables {
        out.push_str(&format!("\n### {}\n\n", table.title.trim()));
        out.push_str(&format!("| {} |\n", table.columns.join(" | ")));
        out.push_str(&format!("|{}\n", "---|".repeat(table.columns.len().max(1))));
        for row in &table.rows {
            out.push_str(&format!("| {} |\n", row.join(" | ")));
        }
    }
    for chart in &doc.charts {
        let unit = chart.unit.as_deref().unwrap_or("");
        out.push_str(&format!("\n### {}\n\n", chart.title.trim()));
        for (label, value) in chart.labels.iter().zip(&chart.values) {
            out.push_str(&format!("- {label}: {value}{unit}\n"));
        }
    }
    if !sources.is_empty() {
        out.push_str("\n## Sources\n\n");
        for s in sources {
            match &s.url {
                Some(url) => out.push_str(&format!("- {} ({url})\n", s.label)),
                None => out.push_str(&format!("- {}\n", s.label)),
            }
        }
    }
    out
}

/// Render the complete self-contained report document.
pub fn render_report(
    doc: &ReportDoc,
    sources: &[Source],
    images: &[EmbeddedImage],
    generated: &str,
) -> String {
    let title = if doc.title.trim().is_empty() { "Research report" } else { doc.title.trim() };
    let mut html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{CSS}</style></head><body><main>",
        escape(title)
    );

    html.push_str(&format!(
        "<header><h1>{}</h1><div class=\"meta\">{} · generated by episteme deep research</div></header>",
        escape(title),
        escape(generated),
    ));
    if !doc.intro.trim().is_empty() {
        html.push_str(&format!("<p class=\"intro\">{}</p>", escape(&doc.intro)));
    }

    for section in &doc.sections {
        if !section.heading.trim().is_empty() {
            html.push_str(&format!("<h2>{}</h2>", escape(&section.heading)));
        }
        for para in &section.paragraphs {
            html.push_str(&format!(
                "<p>{}{}</p>",
                escape(&para.text),
                citation_links(&para.cites, sources)
            ));
        }
    }

    for table in &doc.tables {
        html.push_str("<table>");
        if !table.title.trim().is_empty() {
            html.push_str(&format!("<caption>{}</caption>", escape(&table.title)));
        }
        if !table.columns.is_empty() {
            html.push_str("<thead><tr>");
            for col in &table.columns {
                html.push_str(&format!("<th>{}</th>", escape(col)));
            }
            html.push_str("</tr></thead>");
        }
        html.push_str("<tbody>");
        for row in &table.rows {
            html.push_str("<tr>");
            for cell in row {
                html.push_str(&format!("<td>{}</td>", escape(cell)));
            }
            html.push_str("</tr>");
        }
        html.push_str("</tbody></table>");
    }

    for chart in &doc.charts {
        let svg = bar_chart_svg(chart);
        if svg.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<div class=\"chart\"><h3>{}</h3>{}</div>",
            escape(&chart.title),
            svg
        ));
    }

    for img in images {
        html.push_str(&format!(
            "<figure><img src=\"{}\" alt=\"{}\"><figcaption>{}</figcaption></figure>",
            img.data_uri, // built from bytes we fetched ourselves, never model text
            escape(&img.caption),
            escape(&img.caption),
        ));
    }

    if !sources.is_empty() {
        html.push_str("<div class=\"sources\"><h2>Sources</h2><ol>");
        for (i, src) in sources.iter().enumerate() {
            let body = match &src.url {
                Some(url) => format!(
                    "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                    escape(url),
                    escape(&src.label)
                ),
                None => format!(
                    "{} <span class=\"src-kind\">({})</span>",
                    escape(&src.label),
                    escape(src.id.split(':').next().unwrap_or("internal"))
                ),
            };
            html.push_str(&format!("<li id=\"src-{}\">{}</li>", i + 1, body));
        }
        html.push_str("</ol></div>");
    }

    html.push_str("<footer>Self-contained report — no external scripts or trackers.</footer>");
    html.push_str("</main></body></html>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Vec<Source> {
        vec![
            Source { id: "S1".into(), label: "Example Blog".into(), url: Some("https://a.example/x".into()) },
            Source { id: "doc:contract.pdf".into(), label: "contract.pdf".into(), url: None },
        ]
    }

    #[test]
    fn markdown_rendering_carries_the_substance_without_boilerplate() {
        let doc = ReportDoc {
            title: "NVR options".into(),
            intro: "Three contenders.".into(),
            sections: vec![Section {
                heading: "Frigate".into(),
                paragraphs: vec![Paragraph { text: "Runs on Coral.".into(), cites: vec!["S1".into()] }],
            }],
            tables: vec![Table {
                title: "Pricing".into(),
                columns: vec!["Option".into(), "Cost".into()],
                rows: vec![vec!["Frigate".into(), "$0".into()]],
            }],
            charts: vec![Chart {
                title: "RAM use".into(),
                labels: vec!["Frigate".into()],
                values: vec![1.5],
                unit: Some(" GB".into()),
            }],
            ..Default::default()
        };
        let md = render_markdown(&doc, &sources());
        assert!(md.starts_with("# NVR options"));
        assert!(md.contains("Three contenders."));
        assert!(md.contains("## Frigate"));
        assert!(md.contains("Runs on Coral."));
        assert!(md.contains("| Frigate | $0 |"));
        assert!(md.contains("- Frigate: 1.5 GB"));
        assert!(md.contains("- Example Blog (https://a.example/x)"));
        assert!(md.contains("- contract.pdf"));
        // No HTML or citation anchors leak into the corpus text.
        assert!(!md.contains('<') && !md.contains("[1]"));
    }

    #[test]
    fn escape_covers_the_critical_set() {
        assert_eq!(escape(r#"<b>&"it's"</b>"#), "&lt;b&gt;&amp;&quot;it&#39;s&quot;&lt;/b&gt;");
        // & first: no double-escaping.
        assert_eq!(escape("&lt;"), "&amp;lt;");
    }

    #[test]
    fn model_text_never_becomes_live_html() {
        let doc = ReportDoc {
            title: "<script>alert(1)</script>".into(),
            intro: "<img onerror=x>".into(),
            sections: vec![Section {
                heading: "<style>".into(),
                paragraphs: vec![Paragraph { text: "<iframe>".into(), cites: vec![] }],
            }],
            ..Default::default()
        };
        let html = render_report(&doc, &[], &[], "7 Jun 2026");
        assert!(!html.contains("<script>alert"));
        assert!(!html.contains("<img onerror"));
        assert!(!html.contains("<iframe>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn citations_anchor_into_the_sources_list() {
        let doc = ReportDoc {
            title: "T".into(),
            sections: vec![Section {
                heading: "H".into(),
                paragraphs: vec![Paragraph {
                    text: "claim".into(),
                    cites: vec!["doc:contract.pdf".into(), "S1".into(), "nonexistent".into()],
                }],
            }],
            ..Default::default()
        };
        let html = render_report(&doc, &sources(), &[], "7 Jun 2026");
        // doc:contract.pdf is source #2, S1 is #1; unknown ids render nothing.
        assert!(html.contains("<sup><a href=\"#src-2\">[2]</a></sup>"));
        assert!(html.contains("<sup><a href=\"#src-1\">[1]</a></sup>"));
        assert!(html.contains("<li id=\"src-1\">"));
        assert!(html.contains("<li id=\"src-2\">"));
        assert!(!html.contains("nonexistent"));
        // Web source links out; internal source shows its kind.
        assert!(html.contains("href=\"https://a.example/x\""));
        assert!(html.contains("<span class=\"src-kind\">(doc)</span>"));
    }

    #[test]
    fn bar_chart_geometry() {
        let chart = Chart {
            title: "Counts".into(),
            labels: vec!["A".into(), "B".into()],
            values: vec![10.0, 5.0],
            unit: None,
        };
        let svg = bar_chart_svg(&chart);
        // Max bar fills the available width (640 - 170 gutter - 64 pad = 406).
        assert!(svg.contains("width=\"406.0\""));
        assert!(svg.contains("width=\"203.0\""));
        assert!(svg.contains(">10<") && svg.contains(">5<"));
        assert!(!svg.contains("<script"));
    }

    #[test]
    fn bar_chart_degenerate_inputs() {
        // All-zero values: no division by zero, zero-width bars.
        let zero = Chart { title: "Z".into(), labels: vec!["a".into()], values: vec![0.0], unit: None };
        assert!(bar_chart_svg(&zero).contains("width=\"0.0\""));
        // Negative clamps to zero.
        let neg = Chart { title: "N".into(), labels: vec!["a".into()], values: vec![-3.0], unit: None };
        assert!(bar_chart_svg(&neg).contains("width=\"0.0\""));
        // Empty: renders nothing.
        let empty = Chart::default();
        assert!(bar_chart_svg(&empty).is_empty());
    }

    #[test]
    fn partial_synthesis_json_still_deserializes() {
        let doc: ReportDoc = serde_json::from_str(r#"{"title":"X","sections":[{"heading":"H"}]}"#).unwrap();
        assert_eq!(doc.title, "X");
        assert!(doc.sections[0].paragraphs.is_empty());
        assert!(doc.tables.is_empty());
        let html = render_report(&doc, &[], &[], "today");
        assert!(html.contains("<h2>H</h2>"));
    }
}
