//! Product registry: the single source of truth for everything the mirror fetches.

use std::collections::HashSet;

/// One mirrored file.
pub struct Product {
    pub category: &'static str,    // e.g. "eop", "space_weather", "catalog"
    pub source: &'static str,      // e.g. "iers", "celestrak"
    pub name: &'static str,        // public path segment
    pub url: String,               // upstream HTTPS source
    pub filename: String,          // stable served filename
    pub content_type: &'static str,
    pub active: bool,              // false → not fetched; existing object stays frozen
    pub alias_name: Option<&'static str>, // also written under this stable path segment
}

/// CelesTrak GP groups mirrored as JSON (latest-only).
const CELESTRAK_GROUPS: &[&str] = &[
    "active", "stations", "visual", "last-30-days", "starlink",
    "gnss", "gps-ops", "geo", "weather", "science",
];

/// Build the full registry: fixed EOP/SW entries + generated CelesTrak groups.
pub fn products() -> Vec<Product> {
    let mut items = vec![
        Product {
            category: "eop", source: "iers", name: "finals_all",
            url: "https://datacenter.iers.org/data/latestVersion/finals.all.iau2000.txt".into(),
            filename: "finals.all.iau2000.txt".into(),
            content_type: "text/plain", active: true, alias_name: None,
        },
        Product {
            category: "eop", source: "iers", name: "c04_20u24",
            url: "https://datacenter.iers.org/data/latestVersion/EOP_20u24_C04_one_file_1962-now.txt".into(),
            filename: "EOP_C04_one_file_1962-now.txt".into(),
            content_type: "text/plain", active: true, alias_name: Some("c04"),
        },
        Product {
            category: "space_weather", source: "celestrak", name: "sw_all",
            url: "https://celestrak.org/SpaceData/sw19571001.txt".into(),
            filename: "sw19571001.txt".into(),
            content_type: "text/plain", active: true, alias_name: None,
        },
    ];

    for slug in CELESTRAK_GROUPS {
        items.push(Product {
            category: "catalog", source: "celestrak", name: slug,
            url: format!("https://celestrak.org/NORAD/elements/gp.php?GROUP={slug}&FORMAT=json"),
            filename: format!("{slug}.json"),
            content_type: "application/json", active: true, alias_name: None,
        });
    }

    items
}

/// Enforce: at most one active product per (category, source, alias_name).
pub fn validate_registry(items: &[Product]) -> Result<(), String> {
    let mut seen: HashSet<(&str, &str, &str)> = HashSet::new();
    for p in items {
        if !p.active {
            continue;
        }
        if let Some(alias) = p.alias_name {
            if !seen.insert((p.category, p.source, alias)) {
                return Err(format!("duplicate active alias: {}/{}/{}", p.category, p.source, alias));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_13_active_products() {
        let items = products();
        assert_eq!(items.iter().filter(|p| p.active).count(), 13);
    }

    #[test]
    fn c04_versioned_entry_aliases_to_c04() {
        let items = products();
        let c04 = items.iter().find(|p| p.name == "c04_20u24").expect("c04_20u24 present");
        assert_eq!(c04.category, "eop");
        assert_eq!(c04.source, "iers");
        assert_eq!(c04.filename, "EOP_C04_one_file_1962-now.txt");
        assert_eq!(c04.alias_name, Some("c04"));
        assert!(c04.url.contains("EOP_20u24_C04_one_file_1962-now.txt"));
    }

    #[test]
    fn celestrak_groups_are_json_under_catalog() {
        let items = products();
        let starlink = items.iter().find(|p| p.name == "starlink").expect("starlink present");
        assert_eq!(starlink.category, "catalog");
        assert_eq!(starlink.source, "celestrak");
        assert_eq!(starlink.filename, "starlink.json");
        assert_eq!(starlink.content_type, "application/json");
        assert!(starlink.url.contains("GROUP=starlink"));
        assert!(starlink.url.contains("FORMAT=json"));
    }

    #[test]
    fn default_registry_passes_validation() {
        assert!(validate_registry(&products()).is_ok());
    }

    #[test]
    fn duplicate_active_alias_is_rejected() {
        let dupes = vec![
            Product { category: "eop", source: "iers", name: "c04_a", url: "u".into(),
                filename: "f".into(), content_type: "text/plain", active: true, alias_name: Some("c04") },
            Product { category: "eop", source: "iers", name: "c04_b", url: "u".into(),
                filename: "f".into(), content_type: "text/plain", active: true, alias_name: Some("c04") },
        ];
        assert!(validate_registry(&dupes).is_err());
    }

}
