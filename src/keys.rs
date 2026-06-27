//! Mapping from a `Product` to its R2 object key and public URL.

use crate::products::Product;

/// Public serving domain (R2 public bucket custom domain).
pub const DOMAIN: &str = "simplespacedata.org";

/// R2 object key (== public URL path) for a product's latest file.
pub fn object_key(p: &Product) -> String {
    format!("{}/{}/{}/latest/{}", p.category, p.source, p.name, p.filename)
}

/// R2 object key for the product's stable alias, if it declares one.
pub fn alias_key(p: &Product) -> Option<String> {
    p.alias_name
        .map(|alias| format!("{}/{}/{}/latest/{}", p.category, p.source, alias, p.filename))
}

/// Full public URL for an R2 key.
pub fn public_url(key: &str) -> String {
    format!("https://{DOMAIN}/{key}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::Product;
    use std::time::Duration;

    fn c04() -> Product {
        Product {
            category: "eop", source: "iers", name: "c04_20u24",
            url: "https://example.test/x".into(),
            filename: "EOP_C04_one_file_1962-now.txt".into(),
            content_type: "text/plain", active: true, alias_name: Some("c04"),
            interval: Duration::from_secs(3600),
        }
    }

    fn finals() -> Product {
        Product {
            category: "eop", source: "iers", name: "finals_all",
            url: "https://example.test/y".into(),
            filename: "finals.all.iau2000.txt".into(),
            content_type: "text/plain", active: true, alias_name: None,
            interval: Duration::from_secs(3600),
        }
    }

    #[test]
    fn object_key_uses_latest_layout() {
        assert_eq!(object_key(&c04()), "eop/iers/c04_20u24/latest/EOP_C04_one_file_1962-now.txt");
    }

    #[test]
    fn alias_key_uses_alias_segment() {
        assert_eq!(alias_key(&c04()), Some("eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt".to_string()));
    }

    #[test]
    fn no_alias_means_none() {
        assert_eq!(alias_key(&finals()), None);
    }

    #[test]
    fn public_url_prefixes_domain() {
        assert_eq!(
            public_url("eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt"),
            "https://simplespacedata.org/eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt"
        );
    }
}
