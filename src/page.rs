//! Static landing page generated from the product registry.

use crate::keys::{alias_key, object_key, public_url};
use crate::products::Product;
use crate::schedule::{Schedule, Weekday};

/// Render the full `index.html`: a centered, self-contained page listing every
/// product. Freshness/hash cells are filled client-side from `/status.json`.
pub fn render_index_html(domain: &str, items: &[Product]) -> String {
    let sections = render_sections(domain, items);

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
details{{margin:1rem 0;border:1px solid rgba(127,127,127,.25);border-radius:8px;padding:.5rem 1rem}}
summary{{font-size:1.15rem;font-weight:600;cursor:pointer;padding:.3rem 0}}
h2.prov{{font-size:.95rem;font-weight:600;color:#666;margin:1rem 0 .3rem}}
@media(prefers-color-scheme:dark){{h2.prov{{color:#aaa}}}}
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
.links a{{font-size:.78rem;margin-left:.35rem}}
.freq{{white-space:nowrap}}
td.dotcell{{width:1rem;padding-right:0}}
.dot{{display:inline-block;width:.6rem;height:.6rem;border-radius:50%;background:#bbb;vertical-align:middle}}
.lvl0{{color:#16a34a}} .lvl1{{color:#d97706}} .lvl2{{color:#dc2626}}
.dot.lvl0{{background:#22c55e}} .dot.lvl1{{background:#f59e0b}} .dot.lvl2{{background:#ef4444}}
@media(prefers-color-scheme:dark){{
.lvl0{{color:#4ade80}} .lvl1{{color:#fbbf24}} .lvl2{{color:#f87171}}}}
footer{{text-align:center;margin-top:2rem;color:#888;font-size:.8rem}}
footer a{{color:inherit;text-decoration:underline}}
</style>
</head>
<body>
<header>
<h1>Simple Space Data Mirror</h1>
<p>A public mirror of major public space data sources commonly used in astrodynamics
computations. Developed and maintained for use with
<a href="https://github.com/duncaneddy/brahe">brahe</a> to provide redundancy in parameter
sources. Each file refreshes on its own schedule
(from every few hours to weekly) and are served at stable URLs of the form
<code>/&lt;category&gt;/&lt;source&gt;/&lt;name&gt;/latest/&lt;filename&gt;</code>.</p>
</header>
{sections}<footer>Source on <a href="https://github.com/duncaneddy/ssdm">GitHub</a> — found a bug or have a suggestion?
<a href="https://github.com/duncaneddy/ssdm/issues/new">Open an issue</a>.</footer>
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
function fmtSize(b){{
  if(!b) return "—";
  if(b<1024) return b+" B";
  var u=["KB","MB","GB","TB"], i=-1;
  do {{ b/=1024; i++; }} while(b>=1024 && i<u.length-1);
  return b.toFixed(1)+" "+u[i];
}}
fetch("/status.json").then(function(r){{return r.ok?r.json():{{}};}}).then(function(st){{
  var now=Date.now();
  document.querySelectorAll("tr[data-key]").forEach(function(tr){{
    var e=st[tr.getAttribute("data-key")]; if(!e) return;
    var up=tr.querySelector(".updated"), ck=tr.querySelector(".checked"),
        hs=tr.querySelector(".hashval"), bt=tr.querySelector(".hash .copy"),
        sz=tr.querySelector(".sizeval"), dot=tr.querySelector(".dot");
    var iv=parseFloat(tr.getAttribute("data-interval-ms"))||0;
    // staleness level vs the product cadence: 0 green (<=1x), 1 amber (<=3x),
    // 2 red (beyond), -1 unknown. Returns -1 when the timestamp or cadence is absent.
    function lvl(ms){{ if(!ms||!iv) return -1; var r=(now-ms)/iv; return r<=1?0:(r<=3?1:2); }}
    function paint(el,l){{ if(l>=0) el.classList.add("lvl"+l); }}

    up.textContent=rel(e.last_updated); up.title=abs(e.last_updated);
    // "Last checked" shows the most recent attempt (advances every cycle, success
    // or failure); its color tracks the last SUCCESSFUL check, so a source that is
    // attempted but failing reads e.g. "5m ago" in amber/red.
    var attempt=e.last_attempt||e.last_checked;
    ck.textContent=rel(attempt);
    ck.title=(e.last_checked && e.last_checked!==attempt)
      ? "attempted "+abs(attempt)+"; last success "+abs(e.last_checked)
      : abs(attempt);
    if(sz){{ sz.textContent=fmtSize(e.size); if(e.size) sz.title=e.size+" bytes"; }}

    var c=lvl(e.last_checked), u=lvl(e.last_updated);
    paint(ck,c); paint(up,u);
    paint(dot,Math.max(c,u));  // dot = worse of connectivity and data freshness

    if(e.hash){{ hs.textContent=e.hash.slice(0,12)+"…"; bt.dataset.copy=e.hash; bt.hidden=false; }}
  }});
}}).catch(function(){{}});
document.addEventListener("click",function(ev){{
  var b=ev.target.closest(".copy"); if(!b||!b.dataset.copy) return;
  navigator.clipboard.writeText(b.dataset.copy).then(function(){{
    var t=b.textContent; b.textContent="copied"; setTimeout(function(){{b.textContent=t;}},1200);
  }});
}});
</script>
</body>
</html>
"#
    )
}

/// Minimal escaping for values placed inside a double-quoted HTML attribute.
fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

fn push_row(out: &mut String, p: &Product, key: &str, label: &str, url: &str) {
    let cls = if p.active { "" } else { " class=\"discontinued\"" };
    let interval_ms = p.schedule.nominal_period().as_millis();
    let url_attr = esc_attr(url);

    let mut links = format!(
        " <a class=\"src\" href=\"{}\" title=\"Upstream source\">source</a>",
        esc_attr(&p.url)
    );
    if let Some(info) = p.info_url {
        links.push_str(&format!(
            " <a class=\"info\" href=\"{}\" title=\"Product information\">\u{24D8}</a>",
            esc_attr(info)
        ));
    }

    let freq = p
        .cadence_label
        .map(|s| s.to_string())
        .unwrap_or_else(|| humanize_schedule(&p.schedule));

    out.push_str(&format!(
        "<tr data-key=\"{key}\" data-interval-ms=\"{interval_ms}\"{cls}>\
<td class=\"dotcell\"><span class=\"dot\"></span></td>\
<td>{label}<span class=\"links\">{links}</span></td>\
<td class=\"freq\">{freq}</td>\
<td class=\"dl\"><a href=\"{url}\">{url}</a>\
<button class=\"copy\" data-copy=\"{url_attr}\">copy</button></td>\
<td class=\"size\"><span class=\"sizeval\">\u{2014}</span></td>\
<td class=\"rel updated\">\u{2014}</td>\
<td class=\"rel checked\">\u{2014}</td>\
<td class=\"hash\"><span class=\"hashval\">\u{2014}</span>\
<button class=\"copy\" hidden>copy</button></td>\
</tr>\n"
    ));
}

fn category_label(cat: &str) -> &str {
    match cat {
        "eop" => "Earth Orientation Parameters",
        "space_weather" => "Space Weather",
        "catalog" => "Ephemeris",
        other => other,
    }
}

fn provider_label(src: &str) -> &str {
    match src {
        "iers" => "IERS",
        "usno" => "USNO",
        "obspm" => "Paris Observatory",
        "celestrak" => "CelesTrak",
        other => other,
    }
}

/// Group products by category (→ collapsible section) then provider (→ table),
/// preserving first-seen order.
fn render_sections(domain: &str, items: &[Product]) -> String {
    let mut cats: Vec<&str> = Vec::new();
    for p in items {
        if !cats.contains(&p.category) {
            cats.push(p.category);
        }
    }

    let mut out = String::new();
    for cat in cats {
        out.push_str(&format!(
            "<details open><summary>{}</summary>\n",
            category_label(cat)
        ));

        let mut provs: Vec<&str> = Vec::new();
        for p in items.iter().filter(|p| p.category == cat) {
            if !provs.contains(&p.source) {
                provs.push(p.source);
            }
        }

        for prov in provs {
            out.push_str(&format!("<h2 class=\"prov\">{}</h2>\n", provider_label(prov)));
            out.push_str(
                "<div class=\"tw\"><table>\n<thead><tr>\
<th class=\"dh\"></th><th>Product</th><th>Frequency</th><th>Mirror URL</th><th>Size</th><th>Last updated</th><th>Last checked</th><th>Hash (md5)</th>\
</tr></thead>\n<tbody>\n",
            );
            for p in items.iter().filter(|p| p.category == cat && p.source == prov) {
                let key = object_key(p);
                let label = format!("{}/{}/{}", p.category, p.source, p.name);
                push_row(&mut out, p, &key, &label, &public_url(domain, &key));
                if let Some(akey) = alias_key(p) {
                    let alias = p.alias_name.unwrap_or("");
                    let alias_label = format!("{}/{}/{} (alias)", p.category, p.source, alias);
                    push_row(&mut out, p, &key, &alias_label, &public_url(domain, &akey));
                }
            }
            out.push_str("</tbody>\n</table></div>\n");
        }

        out.push_str("</details>\n");
    }
    out
}

/// Short human label for a polling interval, e.g. `daily`, `weekly`, `6h`, `90m`.
fn humanize_interval(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs % 86_400 == 0 {
        match secs / 86_400 {
            1 => "daily".to_string(),
            7 => "weekly".to_string(),
            n => format!("{n}d"),
        }
    } else if secs % 3_600 == 0 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}m", secs / 60)
    }
}

fn humanize_schedule(s: &Schedule) -> String {
    match s {
        Schedule::Every(d) => humanize_interval(*d),
        Schedule::WeeklyAt { weekday, time } => {
            format!("{} from {} UTC", weekday_plural(*weekday), fmt_time_of_day(*time))
        }
    }
}

fn weekday_plural(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Mondays",
        Weekday::Tue => "Tuesdays",
        Weekday::Wed => "Wednesdays",
        Weekday::Thu => "Thursdays",
        Weekday::Fri => "Fridays",
        Weekday::Sat => "Saturdays",
        Weekday::Sun => "Sundays",
    }
}

fn fmt_time_of_day(d: std::time::Duration) -> String {
    let total_min = d.as_secs() / 60;
    format!("{:02}:{:02}", total_min / 60, total_min % 60)
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
                info_url: Some("https://iers.example/info"), cadence_label: None,
                schedule: Schedule::Every(Duration::from_secs(3600)),
            },
            Product {
                category: "eop", source: "iers", name: "c04_19u20",
                url: "https://example.test/old".into(),
                filename: "EOP_C04_one_file_1962-now.txt".into(),
                content_type: "text/plain", active: false, alias_name: None,
                info_url: None, cadence_label: None,
                schedule: Schedule::Every(Duration::from_secs(3600)),
            },
        ]
    }

    #[test]
    fn lists_active_product_url() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("https://example.org/eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt"));
    }

    #[test]
    fn lists_alias_url() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("https://example.org/eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt"));
    }

    #[test]
    fn alias_row_uses_canonical_data_key() {
        let html = render_index_html("example.org", &sample());
        // The alias is served at its own stable URL...
        assert!(html.contains("https://example.org/eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt"));
        // ...but the alias path must NEVER appear as a status-lookup data-key:
        // status.json is keyed by canonical path only, so the alias row must
        // carry the PRIMARY product's key or its freshness/hash never resolve.
        assert!(!html.contains("data-key=\"eop/iers/c04/latest/"));
        assert!(html.contains("data-key=\"eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt\""));
    }

    #[test]
    fn references_brahe() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("https://github.com/duncaneddy/brahe"));
    }

    #[test]
    fn header_describes_astrodynamics_purpose() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("astrodynamics"));
        assert!(html.contains("redundancy in parameter"));
    }

    #[test]
    fn has_freshness_and_hash_columns() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("Last updated"));
        assert!(html.contains("Last checked"));
        assert!(html.contains("Hash"));
    }

    #[test]
    fn rows_carry_data_key_for_status_lookup() {
        let html = render_index_html("example.org", &sample());
        // the active product's object key, used by the client JS to match status.json
        assert!(html.contains("data-key=\"eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt\""));
    }

    #[test]
    fn includes_copy_button_and_status_fetch() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("class=\"copy\""));
        assert!(html.contains("/status.json"));
    }

    #[test]
    fn has_size_column() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains(">Size<"), "Size column header present");
        assert!(html.contains("class=\"sizeval\""), "size cell populated client-side");
        assert!(html.contains("function fmtSize"), "human-readable size formatter present");
    }

    #[test]
    fn url_has_copy_button() {
        let html = render_index_html("example.org", &sample());
        // the active product's full mirror URL is copyable via a data-copy button
        assert!(html.contains(
            "data-copy=\"https://example.org/eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt\""
        ));
    }

    #[test]
    fn humanizes_common_intervals() {
        use std::time::Duration;
        assert_eq!(humanize_interval(Duration::from_secs(24 * 3600)), "daily");
        assert_eq!(humanize_interval(Duration::from_secs(7 * 24 * 3600)), "weekly");
        assert_eq!(humanize_interval(Duration::from_secs(6 * 3600)), "6h");
        assert_eq!(humanize_interval(Duration::from_secs(2 * 3600)), "2h");
        assert_eq!(humanize_interval(Duration::from_secs(3 * 24 * 3600)), "3d");
        assert_eq!(humanize_interval(Duration::from_secs(90 * 60)), "90m");
    }

    #[test]
    fn renders_collapsible_section_with_display_name() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("<details"), "sections are collapsible");
        assert!(html.contains("<summary"), "sections have a summary header");
        assert!(html.contains("Earth Orientation Parameters"), "category display name");
    }

    #[test]
    fn renders_provider_subheading() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("IERS"), "provider display name shown");
    }

    #[test]
    fn shows_frequency_column_and_values() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("Frequency"), "frequency column header");
        // sample c04 has interval 3600s => hourly => "1h"
        assert!(html.contains(">1h<"), "humanized interval rendered");
    }

    #[test]
    fn frequency_prefers_cadence_label() {
        let mut items = sample();
        items[0].cadence_label = Some("Thursdays ~10:00 UTC");
        let html = render_index_html("example.org", &items);
        assert!(html.contains("Thursdays ~10:00 UTC"), "cadence_label overrides interval");
    }

    #[test]
    fn product_cell_has_source_and_info_links() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("href=\"https://example.test/x\""), "upstream source link");
        assert!(html.contains("href=\"https://iers.example/info\""), "info link when info_url set");
    }

    #[test]
    fn rows_carry_interval_ms() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("data-interval-ms=\"3600000\""), "interval emitted in ms");
    }

    #[test]
    fn rows_have_status_dot_cell() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("class=\"dot\""), "status dot present on rows");
    }

    #[test]
    fn js_color_thresholds_present() {
        let html = render_index_html("example.org", &sample());
        // staleness levels keyed off the cadence: <=1x green, <=3x amber, else red
        assert!(html.contains("r<=1?0:(r<=3?1:2)"), "1x/3x staleness thresholds");
        assert!(html.contains(".lvl0"), "green level class");
        assert!(html.contains(".lvl1"), "amber level class");
        assert!(html.contains(".lvl2"), "red level class");
    }

    #[test]
    fn last_checked_shows_attempt_time() {
        // "Last checked" must reflect the most recent attempt (last_attempt), which
        // advances every cycle even when a fetch fails or the content is unchanged.
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("e.last_attempt"), "checked cell reads last_attempt");
    }

    #[test]
    fn checked_color_tracks_last_success() {
        // The checked cell's COLOR derives from last_checked (last successful
        // download), so a failing source shows a recent attempt time in red/amber.
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("c=lvl(e.last_checked)"), "checked color from last_checked");
        assert!(html.contains("u=lvl(e.last_updated)"), "updated color from last_updated");
    }

    #[test]
    fn status_dot_combines_checked_and_updated() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("Math.max(c,u)"), "dot = worse of connectivity and freshness");
    }

    #[test]
    fn footer_links_to_repo_and_issues() {
        let html = render_index_html("example.org", &sample());
        assert!(html.contains("https://github.com/duncaneddy/ssdm"), "repo link");
        assert!(html.contains("https://github.com/duncaneddy/ssdm/issues/new"), "open-an-issue link");
    }

    #[test]
    fn weekly_schedule_renders_weekday_and_time() {
        let items = vec![Product {
            category: "eop", source: "usno", name: "finals2000a_all",
            url: "https://maia.usno.navy.mil/ser7/finals2000A.all".into(),
            filename: "finals2000A.all".into(),
            content_type: "text/plain", active: true, alias_name: None,
            info_url: None, cadence_label: None,
            schedule: Schedule::WeeklyAt {
                weekday: Weekday::Thu,
                time: std::time::Duration::from_secs(18 * 3600 + 15 * 60),
            },
        }];
        let html = render_index_html("example.org", &items);
        assert!(html.contains("Thursdays from 18:15 UTC"), "weekly frequency text");
        assert!(html.contains("data-interval-ms=\"604800000\""), "weekly nominal period = 7d");
    }
}
