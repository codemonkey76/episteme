//! Pure HTML renderer for research reports. Consumes the structured output of
//! the synthesize stage and produces one self-contained document: a sticky
//! table-of-contents sidebar, an editorial hero + run-stats bar, category-aware
//! theming, per-claim citation anchors, styled comparison tables (with
//! server-classified win/lose cells), inline-SVG bar charts, embedded data-URI
//! images, and a numbered sources list.
//!
//! JavaScript: exactly one small inline script drives the TOC active-section
//! highlight (IntersectionObserver). It is built only from code and references
//! only ids we generate — it never touches model text.
//!
//! Security invariant: every model-provided string passes through [`escape`]
//! before entering the document. Structure (tags, anchors, SVG, the script) is
//! built only from code — model text can never inject live HTML.

use std::collections::HashMap;

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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

/// Run statistics surfaced in the report's stats bar. All fields are optional —
/// a zero/empty field is simply omitted from the bar.
#[derive(Debug, Default)]
pub struct ReportStats {
    /// "quick" | "standard" | "deep" — shown capitalised.
    pub depth: String,
    pub rounds: usize,
    pub queries: usize,
    pub sources: usize,
    /// Writer model id (e.g. claude-opus-4-8).
    pub model: String,
}

/// HTML-escape model-provided text. `&` first, then the rest.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A URL-safe, unique anchor slug for a heading. Lowercases, turns runs of
/// non-alphanumerics into single hyphens, and disambiguates repeats with a
/// numeric suffix so two identically-named sections still get distinct ids.
fn slugify(text: &str, seen: &mut HashMap<String, usize>) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in text.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let base = slug.trim_matches('-').to_string();
    let base = if base.is_empty() { "section".to_string() } else { base };
    let n = seen.entry(base.clone()).or_insert(0);
    let out = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
    *n += 1;
    out
}

/// Title-case the first letter of a single word (for the depth label).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Classify a comparison-table cell as positive / negative / neutral by its
/// leading word, so winning and losing cells colour themselves. Done in code
/// (we build the `<td>`s) so no model markup is ever trusted. Empty = no class.
fn comparison_cell_class(text: &str) -> &'static str {
    let t = text.trim().to_lowercase();
    let first = t.split([' ', '/', ',', '(', '-']).next().unwrap_or("");
    const POS: &[&str] = &[
        "yes", "excellent", "best", "great", "strong", "fast", "high", "superior", "winner",
        "free", "unlimited", "native", "full", "advanced", "✓", "✅", "⭐",
    ];
    const NEG: &[&str] = &[
        "no", "none", "poor", "weak", "slow", "low", "limited", "lacking", "missing", "basic",
        "minimal", "✗", "❌",
    ];
    const MID: &[&str] =
        &["moderate", "average", "fair", "partial", "some", "decent", "okay", "mixed", "varies", "depends"];
    if POS.contains(&first) {
        "cmp-pos"
    } else if NEG.contains(&first) || t == "n/a" {
        "cmp-neg"
    } else if MID.contains(&first) {
        "cmp-mid"
    } else {
        ""
    }
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
--accent:#2456a6;--accent-bg:rgba(36,86,166,.07);--bar:#5b8def;--max-w:760px;
--sidebar-w:220px;color-scheme:light dark}
@media (prefers-color-scheme:dark){:root{--bg:#121214;--fg:#dededf;--muted:#8d8d93;
--rule:#2c2c30;--surface:#1c1c1f;--accent:#7ab0ff;--accent-bg:rgba(122,176,255,.10);--bar:#3a6adf}}

/* Per-category accents — only the colour shifts, so each report type reads as
   its own publication while the structure stays consistent. */
body.category-product{--accent:#2a8a8c;--accent-bg:rgba(42,138,140,.08);--bar:#3fb0b2}
body.category-comparison{--accent:#7a4cb8;--accent-bg:rgba(122,76,184,.08);--bar:#9d76d0}
body.category-howto{--accent:#3d8a3d;--accent-bg:rgba(61,138,61,.08);--bar:#62b162}
body.category-factcheck{--accent:#b8543a;--accent-bg:rgba(184,84,58,.08);--bar:#d97a5e}
@media (prefers-color-scheme:dark){
body.category-product{--accent:#5cc8cb;--accent-bg:rgba(92,200,203,.12);--bar:#5cc8cb}
body.category-comparison{--accent:#b896e8;--accent-bg:rgba(184,150,232,.12);--bar:#b896e8}
body.category-howto{--accent:#82c882;--accent-bg:rgba(130,200,130,.12);--bar:#82c882}
body.category-factcheck{--accent:#e88f73;--accent-bg:rgba(232,143,115,.12);--bar:#e88f73}}

*{box-sizing:border-box}
html{scroll-behavior:smooth;scroll-padding-top:1.5rem}
body{margin:0;background:var(--bg);color:var(--fg);
font:16px/1.6 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif}

/* ── Hero ─────────────────────────────────────────────── */
.hero{text-align:center;padding:3.2rem 1.25rem 2rem;position:relative;overflow:hidden}
.hero::before{content:'';position:absolute;inset:0;pointer-events:none;
background:radial-gradient(ellipse 70% 70% at 50% 30%,var(--accent-bg) 0%,transparent 70%)}
.hero-label{position:relative;text-transform:uppercase;letter-spacing:.26em;font-size:.66rem;
font-weight:600;color:var(--accent);opacity:.85;margin-bottom:1rem}
.hero h1{position:relative;font-size:clamp(1.7rem,3.5vw,2.5rem);line-height:1.18;margin:0 auto;
max-width:720px;letter-spacing:-.02em;font-weight:650}
.hero .meta{position:relative;color:var(--muted);font-size:.8rem;margin-top:.7rem}

/* ── Stats bar ────────────────────────────────────────── */
.stats-bar{display:flex;justify-content:center;gap:1.4rem;flex-wrap:wrap;
padding:.8rem 1.25rem;background:var(--surface);border-top:1px solid var(--rule);
border-bottom:1px solid var(--rule);font-size:.78rem;color:var(--muted)}
.stat{display:flex;align-items:center;gap:.35rem}
.stat-value{font-weight:600;color:var(--fg)}

/* ── Layout: sticky TOC + content ─────────────────────── */
.layout{display:grid;grid-template-columns:var(--sidebar-w) 1fr;
max-width:calc(var(--max-w) + var(--sidebar-w) + 60px);margin:0 auto;gap:0}
.toc-sidebar{position:sticky;top:0;align-self:start;height:100vh;overflow-y:auto;
padding:2.4rem 1rem 2rem 1.4rem;font-size:.8rem}
.toc-sidebar nav{display:flex;flex-direction:column;gap:1px}
.toc-sidebar a{position:relative;display:block;color:var(--muted);text-decoration:none;
padding:.36rem .6rem;border-radius:6px;line-height:1.35;
transition:color .15s,background .15s,padding-left .15s}
.toc-sidebar a::before{content:'';position:absolute;left:0;top:50%;width:2px;height:0;
background:var(--accent);transform:translateY(-50%);border-radius:1px;
transition:height .15s,opacity .15s;opacity:0}
.toc-sidebar a:hover{color:var(--fg);background:var(--accent-bg);padding-left:.8rem}
.toc-sidebar a:hover::before{height:55%;opacity:1}
.toc-sidebar a.active{color:var(--accent);font-weight:600;background:var(--accent-bg)}
.toc-sidebar a.active::before{height:75%;opacity:1}
.toc-sources{margin-top:.6rem;border-top:1px solid var(--rule);padding-top:.6rem!important}

main.content{max-width:var(--max-w);padding:2.4rem 1.25rem 4rem}
main.nocols{margin:0 auto}

h2{font-size:1.22rem;margin:2.1rem 0 .6rem;padding-top:.3rem;
border-bottom:1px solid transparent;border-image:linear-gradient(90deg,var(--accent) 0%,transparent 60%) 1;
padding-bottom:.4rem;scroll-margin-top:1.5rem}
h2:first-child{margin-top:0}
.intro{font-size:1.05rem}
p{margin:.6rem 0}
sup a{color:var(--accent);text-decoration:none;font-size:.72em;padding:0 .08em}
a{color:var(--accent)}
figure{margin:1.4rem 0}
figcaption{color:var(--muted);font-size:.82rem;margin-top:.35rem;text-align:center}
figure img{max-width:100%;border-radius:8px;display:block;margin:0 auto}
table{border-collapse:collapse;width:100%;margin:1.2rem 0;font-size:.9rem}
caption{caption-side:top;text-align:left;font-weight:600;padding-bottom:.45rem}
th,td{border:1px solid var(--rule);padding:.45rem .6rem;text-align:left;vertical-align:top}
th{background:var(--surface)}
td.cmp-pos{color:#2e7d32;font-weight:600;background:rgba(76,175,80,.10)}
td.cmp-neg{color:#c62828;font-weight:600;background:rgba(244,67,54,.08)}
td.cmp-mid{color:#e68a00;background:rgba(255,167,38,.08)}
@media (prefers-color-scheme:dark){
td.cmp-pos{color:#82c882}td.cmp-neg{color:#e88f73}td.cmp-mid{color:#e8c05a}}
.chart{margin:1.4rem 0;background:var(--surface);border:1px solid var(--rule);
border-radius:8px;padding:1rem}
.chart h3{margin:0 0 .6rem;font-size:.95rem}
.bar{fill:var(--bar)}
.bar-label{font-size:12px;fill:var(--fg)}
.bar-value{font-size:12px;fill:var(--muted)}
.sources{border-top:1px solid var(--rule);margin-top:2.5rem;padding-top:1rem}
.sources h2{margin-top:0;border-image:none}
.sources ol{padding-left:1.4rem;font-size:.88rem}
.sources li{margin:.3rem 0}
.sources a{color:var(--accent)}
.src-kind{color:var(--muted)}
footer{color:var(--muted);font-size:.75rem;text-align:center;padding:2rem 1.25rem;
border-top:1px solid var(--rule)}

@media (max-width:900px){
.layout{grid-template-columns:1fr}
.toc-sidebar{display:none}}
@media print{.toc-sidebar{display:none}.layout{grid-template-columns:1fr}}
"#;

/// The lone inline script: highlights the TOC entry for whichever heading is
/// currently in view. References only ids we generated; contains no model text.
const TOC_JS: &str = r##"
(function(){
  var links=document.querySelectorAll('.toc-sidebar a[href^="#"]');
  if(!links.length||!('IntersectionObserver' in window))return;
  var map={};links.forEach(function(l){map[l.getAttribute('href').slice(1)]=l;});
  var heads=document.querySelectorAll('.content h2[id],#sources');
  if(!heads.length)return;
  var active=null;
  function setActive(id){if(id===active)return;
    if(active&&map[active])map[active].classList.remove('active');
    if(id&&map[id])map[id].classList.add('active');active=id;}
  var vis={};
  var io=new IntersectionObserver(function(es){
    es.forEach(function(e){vis[e.target.id]=e.isIntersecting;});
    for(var i=0;i<heads.length;i++){if(vis[heads[i].id]){setActive(heads[i].id);break;}}
  },{rootMargin:'-10% 0px -75% 0px',threshold:0});
  heads.forEach(function(h){io.observe(h);});
})();
"##;

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

/// One `<div class="stat">` per non-empty stat, or an empty string if there's
/// nothing to show (so the bar is omitted entirely).
fn stats_bar(stats: &ReportStats) -> String {
    let mut items: Vec<(String, &str)> = Vec::new();
    if !stats.depth.trim().is_empty() {
        items.push((capitalize(stats.depth.trim()), "Depth"));
    }
    if stats.rounds > 0 {
        items.push((stats.rounds.to_string(), "Rounds"));
    }
    if stats.queries > 0 {
        items.push((stats.queries.to_string(), "Queries"));
    }
    if stats.sources > 0 {
        items.push((stats.sources.to_string(), "Sources"));
    }
    if !stats.model.trim().is_empty() {
        items.push((stats.model.trim().to_string(), "Model"));
    }
    if items.is_empty() {
        return String::new();
    }
    let mut html = String::from("<div class=\"stats-bar\">");
    for (value, label) in items {
        html.push_str(&format!(
            "<div class=\"stat\"><span class=\"stat-value\">{}</span> {}</div>",
            escape(&value),
            label
        ));
    }
    html.push_str("</div>");
    html
}

/// Render the complete self-contained report document.
pub fn render_report(
    doc: &ReportDoc,
    category: &str,
    stats: &ReportStats,
    sources: &[Source],
    images: &[EmbeddedImage],
    generated: &str,
) -> String {
    let title = if doc.title.trim().is_empty() { "Research report" } else { doc.title.trim() };

    // Category drives only a body class (CSS retints from there); "general" /
    // empty stay on the default palette.
    let body_attr = match category.trim() {
        "" | "general" => String::new(),
        cat => format!(" class=\"category-{}\"", escape(cat)),
    };

    let mut html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{CSS}</style></head><body{body_attr}>",
        escape(title)
    );

    // Hero header (full-width, above the layout grid).
    html.push_str(&format!(
        "<header class=\"hero\"><div class=\"hero-label\">Episteme · Deep Research</div>\
         <h1>{}</h1><div class=\"meta\">{} · generated by episteme deep research</div></header>",
        escape(title),
        escape(generated),
    ));

    html.push_str(&stats_bar(stats));

    // Stable, unique anchor slug per section — shared by the TOC and the <h2>.
    let mut seen = HashMap::new();
    let section_slugs: Vec<String> =
        doc.sections.iter().map(|s| slugify(&s.heading, &mut seen)).collect();
    let has_toc = doc.sections.iter().any(|s| !s.heading.trim().is_empty());

    if has_toc {
        html.push_str("<div class=\"layout\"><aside class=\"toc-sidebar\"><nav>");
        for (i, section) in doc.sections.iter().enumerate() {
            if !section.heading.trim().is_empty() {
                html.push_str(&format!(
                    "<a href=\"#{}\">{}</a>",
                    section_slugs[i],
                    escape(&section.heading)
                ));
            }
        }
        if !sources.is_empty() {
            html.push_str("<a href=\"#sources\" class=\"toc-sources\">Sources</a>");
        }
        html.push_str("</nav></aside><main class=\"content\">");
    } else {
        html.push_str("<main class=\"content nocols\">");
    }

    if !doc.intro.trim().is_empty() {
        html.push_str(&format!("<p class=\"intro\">{}</p>", escape(&doc.intro)));
    }

    for (i, section) in doc.sections.iter().enumerate() {
        if !section.heading.trim().is_empty() {
            html.push_str(&format!(
                "<h2 id=\"{}\">{}</h2>",
                section_slugs[i],
                escape(&section.heading)
            ));
        }
        for para in &section.paragraphs {
            html.push_str(&format!(
                "<p>{}{}</p>",
                escape(&para.text),
                citation_links(&para.cites, sources)
            ));
        }
    }

    let is_comparison = category.trim() == "comparison";
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
            for (col_idx, cell) in row.iter().enumerate() {
                // In comparison reports, colour the data cells (not the first,
                // which names the row) by their verdict.
                let class = if is_comparison && col_idx > 0 {
                    comparison_cell_class(cell)
                } else {
                    ""
                };
                if class.is_empty() {
                    html.push_str(&format!("<td>{}</td>", escape(cell)));
                } else {
                    html.push_str(&format!("<td class=\"{class}\">{}</td>", escape(cell)));
                }
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
        html.push_str("<div class=\"sources\" id=\"sources\"><h2>Sources</h2><ol>");
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

    html.push_str("</main>");
    if has_toc {
        html.push_str("</div>");
    }

    html.push_str("<footer>Self-contained report — no external scripts or trackers.</footer>");
    html.push_str(&format!("<script>{TOC_JS}</script>"));
    html.push_str("</body></html>");
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
    fn slugify_is_url_safe_and_deduped() {
        let mut seen = HashMap::new();
        assert_eq!(slugify("Best Overall — The Pick!", &mut seen), "best-overall-the-pick");
        assert_eq!(slugify("Frigate", &mut seen), "frigate");
        // Repeat headings disambiguate with a numeric suffix.
        assert_eq!(slugify("Frigate", &mut seen), "frigate-1");
        // All-punctuation falls back rather than producing an empty id.
        assert_eq!(slugify("!!!", &mut seen), "section");
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
        let html = render_report(&doc, "", &ReportStats::default(), &[], &[], "7 Jun 2026");
        assert!(!html.contains("<script>alert"));
        assert!(!html.contains("<img onerror"));
        assert!(!html.contains("<iframe>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        // The one legitimate script we emit is our own TOC highlighter.
        assert!(html.contains("IntersectionObserver"));
    }

    #[test]
    fn toc_sidebar_anchors_match_section_ids() {
        let doc = ReportDoc {
            title: "T".into(),
            sections: vec![
                Section { heading: "Setup".into(), paragraphs: vec![] },
                Section { heading: "Setup".into(), paragraphs: vec![] },
            ],
            ..Default::default()
        };
        let html = render_report(&doc, "howto", &ReportStats::default(), &sources(), &[], "today");
        // Layout + sidebar present, body retinted by category.
        assert!(html.contains("class=\"category-howto\""));
        assert!(html.contains("<div class=\"layout\">"));
        assert!(html.contains("<aside class=\"toc-sidebar\">"));
        // Duplicate headings get distinct ids, and the TOC links match them.
        assert!(html.contains("<a href=\"#setup\">Setup</a>"));
        assert!(html.contains("<a href=\"#setup-1\">Setup</a>"));
        assert!(html.contains("<h2 id=\"setup\">Setup</h2>"));
        assert!(html.contains("<h2 id=\"setup-1\">Setup</h2>"));
        // Sources gets a TOC entry + matching anchor.
        assert!(html.contains("<a href=\"#sources\""));
        assert!(html.contains("id=\"sources\""));
    }

    #[test]
    fn no_headings_means_no_sidebar() {
        let doc = ReportDoc { title: "T".into(), intro: "Body only.".into(), ..Default::default() };
        let html = render_report(&doc, "", &ReportStats::default(), &[], &[], "today");
        // The class lives in the CSS regardless; assert the *element* isn't built.
        assert!(!html.contains("<aside class=\"toc-sidebar\">"));
        assert!(!html.contains("<div class=\"layout\">"));
        assert!(html.contains("class=\"content nocols\""));
    }

    #[test]
    fn stats_bar_shows_only_present_values() {
        let stats = ReportStats {
            depth: "deep".into(),
            rounds: 4,
            queries: 12,
            sources: 0, // omitted
            model: "claude-opus-4-8".into(),
        };
        let doc = ReportDoc { title: "T".into(), ..Default::default() };
        let html = render_report(&doc, "", &stats, &[], &[], "today");
        assert!(html.contains("stats-bar"));
        // Depth is capitalised; present stats render, zero ones are omitted.
        assert!(html.contains("<span class=\"stat-value\">Deep</span> Depth"));
        assert!(html.contains("<span class=\"stat-value\">12</span> Queries"));
        assert!(html.contains("<span class=\"stat-value\">claude-opus-4-8</span> Model"));
        // sources == 0 → no "Sources" stat anywhere in the document.
        assert!(!html.contains("Sources"));
    }

    #[test]
    fn comparison_tables_colour_their_cells() {
        let doc = ReportDoc {
            title: "T".into(),
            tables: vec![Table {
                title: "Face-off".into(),
                columns: vec!["Tool".into(), "Cost".into(), "Speed".into()],
                rows: vec![vec!["A".into(), "Free".into(), "Slow".into()]],
            }],
            ..Default::default()
        };
        let html = render_report(&doc, "comparison", &ReportStats::default(), &[], &[], "today");
        // First column (row name) is never coloured; data cells are.
        assert!(html.contains("<td>A</td>"));
        assert!(html.contains("<td class=\"cmp-pos\">Free</td>"));
        assert!(html.contains("<td class=\"cmp-neg\">Slow</td>"));
        // Non-comparison categories leave the same cells plain.
        let plain = render_report(&doc, "", &ReportStats::default(), &[], &[], "today");
        assert!(plain.contains("<td>Free</td>"));
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
        let html = render_report(&doc, "", &ReportStats::default(), &sources(), &[], "7 Jun 2026");
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
        let html = render_report(&doc, "", &ReportStats::default(), &[], &[], "today");
        assert!(html.contains("<h2 id=\"h\">H</h2>"));
    }
}
