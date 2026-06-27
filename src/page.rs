//! Static landing page generated from the product registry.

use crate::keys::{alias_key, object_key, public_url};
use crate::products::Product;

/// Render the full `index.html`: a centered, self-contained page listing every
/// product. Freshness/hash cells are filled client-side from `/status.json`.
pub fn render_index_html(items: &[Product]) -> String {
    let mut rows = String::new();
    for p in items {
        let key = object_key(p);
        let label = format!("{}/{}/{}", p.category, p.source, p.name);
        push_row(&mut rows, &key, &label, &public_url(&key), p.active);
        if let Some(akey) = alias_key(p) {
            let alias = p.alias_name.unwrap_or("");
            let alias_label = format!("{}/{}/{} (alias)", p.category, p.source, alias);
            // alias shares the primary key's status (same bytes)
            push_row(&mut rows, &key, &alias_label, &public_url(&akey), p.active);
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
:root{{color-scheme:light dark}}
*{{box-sizing:border-box}}
body{{font-family:system-ui,-apple-system,sans-serif;max-width:96rem;margin:0 auto;
padding:2.5rem 1.25rem 4rem;line-height:1.55}}
header{{text-align:center;margin-bottom:2rem}}
h1{{margin:.2rem 0;font-size:1.7rem}}
header p{{max-width:42rem;margin:.6rem auto;color:#555}}
@media(prefers-color-scheme:dark){{header p{{color:#aaa}}}}
code{{background:rgba(127,127,127,.18);padding:.1rem .35rem;border-radius:4px;font-size:.9em}}
table{{border-collapse:collapse;width:100%;font-size:.88rem}}
th,td{{padding:.5rem .6rem;text-align:left;border-bottom:1px solid rgba(127,127,127,.25);
vertical-align:top}}
th{{font-weight:600;white-space:nowrap}}
tr.discontinued td{{opacity:.55}}
a{{color:#2563eb;text-decoration:none}}
a:hover{{text-decoration:underline}}
.tw{{overflow-x:auto}}
td.dl a{{white-space:nowrap}}
.hash{{font-family:ui-monospace,monospace;font-size:.82rem}}
.copy{{margin-left:.4rem;cursor:pointer;border:1px solid rgba(127,127,127,.4);
background:transparent;border-radius:4px;padding:.05rem .4rem;font-size:.75rem;color:inherit}}
.copy:hover{{background:rgba(127,127,127,.15)}}
.rel{{white-space:nowrap}}
footer{{text-align:center;margin-top:2rem;color:#888;font-size:.8rem}}
</style>
</head>
<body>
<header>
<h1>Simple Space Data Mirror</h1>
<p>A public mirror of Earth Orientation Parameter (EOP) and space-weather data files,
plus selected CelesTrak orbital element sets, maintained for use with
<a href="https://github.com/duncaneddy/brahe">Brahe</a>. Each file refreshes on its own schedule
(from every few hours to weekly) and are served at stable URLs of the form
<code>/&lt;category&gt;/&lt;source&gt;/&lt;name&gt;/latest/&lt;filename&gt;</code>.</p>
</header>
<div class="tw">
<table>
<thead><tr>
<th>Product</th><th>Download</th><th>Last updated</th><th>Last checked</th><th>Hash (md5)</th>
</tr></thead>
<tbody>
{rows}</tbody>
</table>
</div>
<footer>Freshness and hashes load from <code>/status.json</code>.</footer>
<script>
function rel(ms){{
  if(!ms) return "—";
  var s=Math.max(0,(Date.now()-ms)/1000);
  if(s<90) return Math.round(s)+"s ago";
  if(s<5400) return Math.round(s/60)+"m ago";
  if(s<172800) return Math.round(s/3600)+"h ago";
  return Math.round(s/86400)+"d ago";
}}
function abs(ms){{ return ms ? new Date(ms).toISOString() : ""; }}
fetch("/status.json").then(function(r){{return r.ok?r.json():{{}};}}).then(function(st){{
  document.querySelectorAll("tr[data-key]").forEach(function(tr){{
    var e=st[tr.getAttribute("data-key")]; if(!e) return;
    var up=tr.querySelector(".updated"), ck=tr.querySelector(".checked"),
        hs=tr.querySelector(".hashval"), bt=tr.querySelector(".copy");
    up.textContent=rel(e.last_updated); up.title=abs(e.last_updated);
    ck.textContent=rel(e.last_checked); ck.title=abs(e.last_checked);
    if(e.hash){{ hs.textContent=e.hash.slice(0,12)+"…"; bt.dataset.hash=e.hash; bt.hidden=false; }}
  }});
}}).catch(function(){{}});
document.addEventListener("click",function(ev){{
  var b=ev.target.closest(".copy"); if(!b||!b.dataset.hash) return;
  navigator.clipboard.writeText(b.dataset.hash).then(function(){{
    var t=b.textContent; b.textContent="copied"; setTimeout(function(){{b.textContent=t;}},1200);
  }});
}});
</script>
</body>
</html>
"#
    )
}

fn push_row(out: &mut String, key: &str, label: &str, url: &str, active: bool) {
    let cls = if active { "" } else { " class=\"discontinued\"" };
    out.push_str(&format!(
        "<tr data-key=\"{key}\"{cls}>\
<td>{label}</td>\
<td class=\"dl\"><a href=\"{url}\">{url}</a></td>\
<td class=\"rel updated\">\u{2014}</td>\
<td class=\"rel checked\">\u{2014}</td>\
<td class=\"hash\"><span class=\"hashval\">\u{2014}</span>\
<button class=\"copy\" hidden>copy</button></td>\
</tr>\n"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::Product;
    use std::time::Duration;

    fn sample() -> Vec<Product> {
        vec![
            Product {
                category: "eop", source: "iers", name: "c04_20u24",
                url: "https://example.test/x".into(),
                filename: "EOP_C04_one_file_1962-now.txt".into(),
                content_type: "text/plain", active: true, alias_name: Some("c04"),
                interval: Duration::from_secs(3600),
            },
            Product {
                category: "eop", source: "iers", name: "c04_19u20",
                url: "https://example.test/old".into(),
                filename: "EOP_C04_one_file_1962-now.txt".into(),
                content_type: "text/plain", active: false, alias_name: None,
                interval: Duration::from_secs(3600),
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
    fn references_brahe() {
        let html = render_index_html(&sample());
        assert!(html.contains("https://github.com/duncaneddy/brahe"));
    }

    #[test]
    fn has_freshness_and_hash_columns() {
        let html = render_index_html(&sample());
        assert!(html.contains("Last updated"));
        assert!(html.contains("Last checked"));
        assert!(html.contains("Hash"));
    }

    #[test]
    fn rows_carry_data_key_for_status_lookup() {
        let html = render_index_html(&sample());
        // the active product's object key, used by the client JS to match status.json
        assert!(html.contains("data-key=\"eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt\""));
    }

    #[test]
    fn includes_copy_button_and_status_fetch() {
        let html = render_index_html(&sample());
        assert!(html.contains("class=\"copy\""));
        assert!(html.contains("/status.json"));
    }
}
