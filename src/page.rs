//! Static landing page generated from the product registry.

use crate::keys::{alias_key, object_key, public_url};
use crate::products::Product;

/// Render the full `index.html` listing every product and its public URL.
pub fn render_index_html(items: &[Product]) -> String {
    let mut rows = String::new();
    for p in items {
        let status = if p.active { "active" } else { "discontinued (frozen)" };
        push_row(&mut rows, p.category, p.source, p.name, &public_url(&object_key(p)), status);
        if let Some(akey) = alias_key(p) {
            let alias = p.alias_name.unwrap_or("");
            push_row(&mut rows, p.category, p.source, &format!("{alias} (alias)"), &public_url(&akey), status);
        }
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Simple Space Data Mirror</title>
<style>
body{{font-family:system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;line-height:1.5}}
table{{border-collapse:collapse;width:100%}}
th,td{{border:1px solid #ccc;padding:.4rem .6rem;text-align:left;font-size:.9rem}}
code{{background:#f4f4f4;padding:.1rem .3rem;border-radius:3px}}
</style>
</head>
<body>
<h1>Simple Space Data Mirror (SSDM)</h1>
<p>A public mirror of Earth Orientation Parameter (EOP) and space-weather data files,
plus selected CelesTrak orbital element sets, maintained for use with
<a href="https://github.com/duncaneddy/brahe">Brahe</a>.</p>
<p>Files are refreshed once daily (UTC). Each product is served at a stable URL of the form
<code>/&lt;category&gt;/&lt;source&gt;/&lt;name&gt;/latest/&lt;filename&gt;</code>.</p>
<table>
<thead><tr><th>Category</th><th>Source</th><th>Name</th><th>URL</th><th>Status</th></tr></thead>
<tbody>
{rows}</tbody>
</table>
</body>
</html>
"#
    )
}

fn push_row(out: &mut String, category: &str, source: &str, name: &str, url: &str, status: &str) {
    out.push_str(&format!(
        "<tr><td>{category}</td><td>{source}</td><td>{name}</td><td><a href=\"{url}\">{url}</a></td><td>{status}</td></tr>\n"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::Product;

    fn sample() -> Vec<Product> {
        vec![
            Product {
                category: "eop", source: "iers", name: "c04_20u24",
                url: "https://example.test/x".into(),
                filename: "EOP_C04_one_file_1962-now.txt".into(),
                content_type: "text/plain", active: true, alias_name: Some("c04"),
            },
            Product {
                category: "eop", source: "iers", name: "c04_19u20",
                url: "https://example.test/old".into(),
                filename: "EOP_C04_one_file_1962-now.txt".into(),
                content_type: "text/plain", active: false, alias_name: None,
            },
        ]
    }

    #[test]
    fn lists_active_product_url() {
        let html = render_index_html(&sample());
        assert!(html.contains("https://simplespacedata.org/eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt"));
    }

    #[test]
    fn lists_alias_url() {
        let html = render_index_html(&sample());
        assert!(html.contains("https://simplespacedata.org/eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt"));
    }

    #[test]
    fn marks_discontinued_products() {
        let html = render_index_html(&sample());
        assert!(html.contains("discontinued"));
    }

    #[test]
    fn references_brahe() {
        let html = render_index_html(&sample());
        assert!(html.contains("https://github.com/duncaneddy/brahe"));
    }
}
