//! Product registry: the single source of truth for everything the mirror fetches.

use std::collections::HashSet;
use std::time::Duration;

use crate::schedule::{Schedule, Weekday};

/// Whether a product is fetched, and whether it is advertised on the landing page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    /// Fetched on its schedule and listed on the landing page.
    Active,
    /// No longer fetched, but the last object stays served and the row stays
    /// listed (greyed out). For superseded realizations, e.g. an old C04 series
    /// whose consumers still resolve the versioned path.
    Frozen,
    /// Not fetched and not listed. The object stays in the bucket but is
    /// unadvertised. For upstreams that are broken — restore by flipping to
    /// `Active`; no other change is needed.
    Disabled,
}

impl Product {
    /// The source shown on the landing page and used for host-level identity.
    /// Multi-part products are all one host, so the first URL represents them.
    pub fn primary_url(&self) -> &str {
        &self.urls[0]
    }
}

impl Availability {
    /// Fetched on its schedule by the daemon and by `sync --all`.
    pub fn is_fetched(self) -> bool {
        matches!(self, Availability::Active)
    }

    /// Rendered as a row on the landing page.
    pub fn is_listed(self) -> bool {
        matches!(self, Availability::Active | Availability::Frozen)
    }
}

/// One mirrored file.
pub struct Product {
    pub category: &'static str,    // e.g. "eop", "space_weather", "star_catalog"
    pub source: &'static str,      // e.g. "iers", "celestrak", "cds"
    pub name: &'static str,        // public path segment (a dataset may span several files)
    /// Upstream sources, fetched in order and concatenated into the single served
    /// object. Exactly one element for every product except Tycho-2, whose main
    /// catalog CDS splits across 20 files.
    pub urls: Vec<String>,
    pub filename: String,          // stable served filename
    pub content_type: &'static str,
    /// Upstream serves this file gzip-archived; decompress it before serving so
    /// the mirrored bytes are the plain-text form the product's ReadMe documents.
    /// (Not HTTP Content-Encoding — these are `.gz` files, which reqwest's own
    /// gzip support does not touch.)
    pub gunzip: bool,
    pub availability: Availability,
    pub alias_name: Option<&'static str>, // also written under this stable path segment
    pub info_url: Option<&'static str>,   // human-readable docs page (display only)
    pub cadence_label: Option<&'static str>, // named publish schedule (display only)
    pub schedule: Schedule,
}

/// CelesTrak GP groups mirrored as JSON (latest-only).
const CELESTRAK_GROUPS: &[&str] = &[
    "active", "stations", "visual", "last-30-days", "starlink",
    "gnss", "gps-ops", "geo", "weather", "science",
];

/// Availability of every CelesTrak product (the GP groups above and the space
/// weather file). Disabled since 2026-07-15: celestrak.org connection-times-out
/// for all of them, so they would only ever render as failing rows. Set this
/// back to `Active` to resume the whole provider in one edit.
const CELESTRAK: Availability = Availability::Disabled;

/// Number of files CDS splits the Tycho-2 main catalog across (tyc2.dat.00 … .19).
const TYCHO2_PARTS: u32 = 20;

/// The star catalogs are frozen historical artifacts (FK5 1993, Hipparcos 1997,
/// Tycho-2 2000), so this cadence is an availability check rather than a change
/// check.
const STAR_CATALOG_CHECK: Schedule = Schedule::Every(Duration::from_secs(30 * 24 * 3600));

/// Build the full registry: fixed EOP/SW/star-catalog entries + generated
/// CelesTrak groups.
pub fn products() -> Vec<Product> {
    let mut items = vec![
        Product {
            category: "eop", source: "iers", name: "finals_all",
            urls: vec!["https://datacenter.iers.org/data/latestVersion/finals.all.iau2000.txt".into()],
            filename: "finals.all.iau2000.txt".into(),
            content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: None,
            info_url: Some("https://www.iers.org/IERS/EN/DataProducts/EarthOrientationData/eop.html"),
            cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(24 * 3600)),
        },
        Product {
            category: "eop", source: "iers", name: "c04_20u24",
            urls: vec!["https://datacenter.iers.org/data/latestVersion/EOP_20u24_C04_one_file_1962-now.txt".into()],
            filename: "EOP_C04_one_file_1962-now.txt".into(),
            content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: Some("c04"),
            info_url: Some("https://www.iers.org/IERS/EN/DataProducts/EarthOrientationData/eop.html"),
            cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(7 * 24 * 3600)),
        },
        Product {
            category: "eop", source: "usno", name: "finals2000a_all",
            urls: vec!["https://maia.usno.navy.mil/ser7/finals2000A.all".into()],
            filename: "finals2000A.all".into(),
            content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: None,
            info_url: Some("https://maia.usno.navy.mil/ser7/readme"),
            cadence_label: None,
            schedule: Schedule::WeeklyAt {
                weekday: Weekday::Thu,
                time: Duration::from_secs(18 * 3600 + 15 * 60),
            },
        },
        Product {
            category: "eop", source: "usno", name: "finals2000a_daily",
            urls: vec!["https://maia.usno.navy.mil/ser7/finals2000A.daily".into()],
            filename: "finals2000A.daily".into(),
            content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: None,
            info_url: Some("https://maia.usno.navy.mil/ser7/readme"),
            cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(24 * 3600)),
        },
        Product {
            category: "eop", source: "obspm", name: "c04_1962now",
            urls: vec!["https://hpiers.obspm.fr/iers/eop/eopc04/eopc04.1962-now".into()],
            filename: "eopc04.1962-now".into(),
            content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: None,
            info_url: Some("https://hpiers.obspm.fr/iers/eop/eopc04/readme"),
            cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(24 * 3600)),
        },
        Product {
            category: "space_weather", source: "celestrak", name: "sw_all",
            urls: vec!["https://celestrak.org/SpaceData/sw19571001.txt".into()],
            filename: "sw19571001.txt".into(),
            content_type: "text/plain", gunzip: false, availability: CELESTRAK, alias_name: None,
            info_url: Some("https://celestrak.org/SpaceData/"),
            cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(8 * 3600)),
        },
    ];

    // Star catalogs from CDS/VizieR. Each dataset shares one `name` segment
    // across its files (the ReadMe defines the fixed-width byte columns, so it
    // is mirrored alongside the data rather than merely linked); `object_key`
    // includes the filename, so they do not collide.
    //
    // Every file is served as plain text under a uniform
    // <Dataset>_Catalog.txt / <Dataset>_Readme.txt name, so all three datasets are
    // consumed identically. That uniformity is not free upstream: CDS archives FK5
    // and Tycho-2 only gzipped and Hipparcos only uncompressed, and it splits the
    // Tycho-2 catalog across 20 files. VizieR's nph-Cat/txt endpoint is
    // deliberately not used as a plain-text shortcut — it re-renders the table
    // with a column-ruler header and does not reproduce the archive bytes the
    // ReadMe describes.
    //
    // Order here is the display order on the landing page: each catalog followed
    // by its ReadMe.
    //
    // Host note: cdsarc.u-strasbg.fr serves the same bytes but presents a
    // self-signed certificate, so only cdsarc.cds.unistra.fr works over https.
    const FK5_INFO: &str = "https://cdsarc.cds.unistra.fr/viz-bin/cat/I/149A";
    const HIP_INFO: &str = "https://cdsarc.cds.unistra.fr/viz-bin/cat/I/239";
    const TYC_INFO: &str = "https://cdsarc.cds.unistra.fr/viz-bin/cat/I/259";

    let cds_file = |name, filename: &str, path: &str, gunzip, info_url| Product {
        category: "star_catalog", source: "cds", name,
        urls: vec![format!("https://cdsarc.cds.unistra.fr/ftp/{path}")],
        filename: filename.into(),
        content_type: "text/plain", gunzip,
        availability: Availability::Active, alias_name: None,
        info_url: Some(info_url),
        cadence_label: None,
        schedule: STAR_CATALOG_CHECK,
    };

    items.push(cds_file("fk5", "FK5_Catalog.txt", "I/149A/catalog.gz", true, FK5_INFO));
    items.push(cds_file("fk5", "FK5_Readme.txt", "I/149A/ReadMe", false, FK5_INFO));
    items.push(cds_file("hipparcos", "Hipparcos_Catalog.txt", "cats/I/239/hip_main.dat", false, HIP_INFO));
    items.push(cds_file("hipparcos", "Hipparcos_Readme.txt", "cats/I/239/ReadMe", false, HIP_INFO));

    // Tycho-2 main catalog. CDS splits it across 20 gzipped parts of ~26 MB each
    // (~501 MB in total); they are fetched in index order and concatenated into
    // one file, because a split catalog is not what a consumer wants to reassemble
    // and the row order across parts is meaningful. The supplements (suppl_1,
    // suppl_2) are deliberately excluded: they use a different column layout, so
    // appending them would put two incompatible record formats in one file.
    items.push(Product {
        category: "star_catalog", source: "cds", name: "tycho2",
        urls: (0..TYCHO2_PARTS)
            .map(|i| format!("https://cdsarc.cds.unistra.fr/ftp/cats/I/259/tyc2.dat.{i:02}.gz"))
            .collect(),
        filename: "Tycho2_Catalog.txt".into(),
        content_type: "text/plain", gunzip: true,
        availability: Availability::Active, alias_name: None,
        info_url: Some(TYC_INFO),
        cadence_label: None,
        schedule: STAR_CATALOG_CHECK,
    });
    items.push(cds_file("tycho2", "Tycho2_Readme.txt", "cats/I/259/ReadMe", false, TYC_INFO));

    for slug in CELESTRAK_GROUPS {
        items.push(Product {
            category: "catalog", source: "celestrak", name: slug,
            urls: vec![format!("https://celestrak.org/NORAD/elements/gp.php?GROUP={slug}&FORMAT=json")],
            filename: format!("{slug}.json"),
            content_type: "application/json", gunzip: false, availability: CELESTRAK, alias_name: None,
            info_url: Some("https://celestrak.org/NORAD/documentation/gp-data-formats.php"),
            cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(8 * 3600)),
        });
    }

    items
}

/// Enforce registry invariants: every product has at least one source, and at
/// most one fetched product claims each (category, source, alias_name).
pub fn validate_registry(items: &[Product]) -> Result<(), String> {
    let mut seen: HashSet<(&str, &str, &str)> = HashSet::new();
    for p in items {
        // `primary_url` indexes urls[0], and a product with no source could never
        // be fetched anyway — fail loudly at startup rather than panicking while
        // rendering the landing page.
        if p.urls.is_empty() {
            return Err(format!("product has no urls: {}/{}/{}", p.category, p.source, p.name));
        }
        if !p.availability.is_fetched() {
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
    use std::time::Duration;

    #[test]
    fn availability_separates_fetching_from_listing() {
        // Frozen is the superseded-realization case: stop fetching, keep the row
        // so existing consumers still find the path. Disabled is the broken-
        // upstream case: stop fetching AND stop advertising. The distinction is
        // the whole reason this is an enum and not a bool.
        assert!(Availability::Active.is_fetched() && Availability::Active.is_listed());
        assert!(!Availability::Frozen.is_fetched() && Availability::Frozen.is_listed());
        assert!(!Availability::Disabled.is_fetched() && !Availability::Disabled.is_listed());
    }

    #[test]
    fn validation_ignores_unfetched_duplicate_aliases() {
        // A superseded realization keeps its alias-free entry, but even if two
        // non-fetched products collided on an alias, nothing would fetch them.
        let items = vec![
            Product { category: "eop", source: "iers", name: "c04_old", urls: vec!["u".into()],
                filename: "f".into(), content_type: "text/plain", gunzip: false, availability: Availability::Frozen,
                alias_name: Some("c04"), info_url: None, cadence_label: None,
                schedule: Schedule::Every(Duration::from_secs(3600)) },
            Product { category: "eop", source: "iers", name: "c04_new", urls: vec!["u".into()],
                filename: "f".into(), content_type: "text/plain", gunzip: false, availability: Availability::Active,
                alias_name: Some("c04"), info_url: None, cadence_label: None,
                schedule: Schedule::Every(Duration::from_secs(3600)) },
        ];
        assert!(validate_registry(&items).is_ok(), "only the active product claims the alias");
    }

    #[test]
    fn registry_fetches_only_eop_and_star_catalogs() {
        let items = products();
        let fetched: Vec<&str> = items
            .iter()
            .filter(|p| p.availability.is_fetched())
            .map(|p| p.category)
            .collect();
        assert_eq!(fetched.len(), 11, "5 EOP + 6 star catalog files");
        assert!(
            fetched.iter().all(|c| *c == "eop" || *c == "star_catalog"),
            "no other category is fetched while CelesTrak is disabled: {fetched:?}"
        );
    }

    #[test]
    fn every_celestrak_product_is_disabled_but_retained() {
        let items = products();
        let celestrak: Vec<&Product> = items.iter().filter(|p| p.source == "celestrak").collect();
        assert_eq!(celestrak.len(), 11, "10 GP groups + space weather kept in the registry");
        for p in &celestrak {
            assert_eq!(p.availability, Availability::Disabled, "{} must be disabled", p.name);
            assert!(!p.availability.is_fetched(), "{} must not be fetched", p.name);
            assert!(!p.availability.is_listed(), "{} must not be listed", p.name);
        }
    }

    #[test]
    fn star_catalog_products_are_present() {
        let items = products();
        let cds: Vec<&Product> = items.iter().filter(|p| p.category == "star_catalog").collect();
        assert_eq!(cds.len(), 6, "three catalogs, each with its ReadMe");

        let keyed = |filename: &str| -> &Product {
            cds.iter()
                .find(|p| p.filename == filename)
                .unwrap_or_else(|| panic!("{filename} present"))
        };

        let fk5 = keyed("FK5_Catalog.txt");
        assert_eq!(fk5.source, "cds");
        assert_eq!(fk5.primary_url(), "https://cdsarc.cds.unistra.fr/ftp/I/149A/catalog.gz");
        assert_eq!(crate::keys::object_key(fk5), "star_catalog/cds/fk5/latest/FK5_Catalog.txt");
        assert!(fk5.gunzip, "CDS archives FK5 only as .gz; we serve the plain text it contains");

        let hip = keyed("Hipparcos_Catalog.txt");
        assert_eq!(hip.primary_url(), "https://cdsarc.cds.unistra.fr/ftp/cats/I/239/hip_main.dat");
        assert_eq!(
            crate::keys::object_key(hip),
            "star_catalog/cds/hipparcos/latest/Hipparcos_Catalog.txt"
        );
        assert!(!hip.gunzip, "hip_main.dat is archived uncompressed");

        // The ReadMe defines the fixed-width byte columns; it is mirrored, not just linked.
        for (filename, url) in [
            ("FK5_Readme.txt", "https://cdsarc.cds.unistra.fr/ftp/I/149A/ReadMe"),
            ("Hipparcos_Readme.txt", "https://cdsarc.cds.unistra.fr/ftp/cats/I/239/ReadMe"),
            ("Tycho2_Readme.txt", "https://cdsarc.cds.unistra.fr/ftp/cats/I/259/ReadMe"),
        ] {
            let readme = keyed(filename);
            assert_eq!(readme.primary_url(), url);
            assert!(!readme.gunzip);
        }

        // Tycho-2's main catalog is split across 20 gzipped parts upstream and
        // must be served as one concatenated file, in index order — the row order
        // across parts is meaningful, so a shuffled or short join is a real defect.
        let tyc = keyed("Tycho2_Catalog.txt");
        assert_eq!(tyc.name, "tycho2");
        assert!(tyc.gunzip, "every part is gzipped");
        assert_eq!(tyc.urls.len(), 20, "tyc2.dat.00 … .19");
        assert_eq!(
            crate::keys::object_key(tyc),
            "star_catalog/cds/tycho2/latest/Tycho2_Catalog.txt",
            "20 upstream parts, one served object"
        );
        let expected: Vec<String> = (0..20)
            .map(|i| format!("https://cdsarc.cds.unistra.fr/ftp/cats/I/259/tyc2.dat.{i:02}.gz"))
            .collect();
        assert_eq!(tyc.urls, expected, "parts are listed in ascending index order");
        assert!(
            !tyc.urls.iter().any(|u| u.contains("suppl_")),
            "supplements use a different column layout and must not be concatenated in"
        );

        for p in &cds {
            assert_eq!(p.availability, Availability::Active, "{} active", p.filename);
            assert_eq!(p.schedule, Schedule::Every(Duration::from_secs(30 * 24 * 3600)));
            assert!(!p.urls.is_empty(), "{} has at least one source", p.filename);
            for u in &p.urls {
                assert!(u.starts_with("https://cdsarc.cds.unistra.fr/"),
                    "cdsarc.u-strasbg.fr serves a self-signed cert over https: {u}");
            }
            // The point of the naming/format scheme: both datasets are consumed
            // identically, regardless of how CDS happens to archive each one.
            assert_eq!(p.content_type, "text/plain", "{} is served as plain text", p.filename);
            assert!(p.filename.ends_with(".txt"), "{} has a uniform .txt name", p.filename);
        }
    }

    #[test]
    fn star_catalogs_are_listed_catalog_then_readme() {
        // Registry order is display order on the landing page; each catalog should
        // be followed by its own ReadMe rather than the pairs being interleaved.
        let items = products();
        let names: Vec<&str> = items
            .iter()
            .filter(|p| p.category == "star_catalog")
            .map(|p| p.filename.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "FK5_Catalog.txt", "FK5_Readme.txt",
                "Hipparcos_Catalog.txt", "Hipparcos_Readme.txt",
                "Tycho2_Catalog.txt", "Tycho2_Readme.txt",
            ]
        );
    }

    #[test]
    fn star_catalog_files_share_a_name_without_colliding() {
        let items = products();
        let keys: Vec<String> = items
            .iter()
            .filter(|p| p.name == "fk5")
            .map(crate::keys::object_key)
            .collect();
        assert_eq!(keys.len(), 2, "two files under one dataset name");
        assert_ne!(keys[0], keys[1], "filename disambiguates the object key");
    }

    #[test]
    fn usno_finals2000a_entries_present() {
        let items = products();

        let all = items.iter().find(|p| p.name == "finals2000a_all").expect("finals2000a_all present");
        assert_eq!(all.category, "eop");
        assert_eq!(all.source, "usno");
        assert_eq!(all.filename, "finals2000A.all");
        assert_eq!(all.primary_url(), "https://maia.usno.navy.mil/ser7/finals2000A.all");
        assert_eq!(
            all.schedule,
            Schedule::WeeklyAt {
                weekday: Weekday::Thu,
                time: Duration::from_secs(18 * 3600 + 15 * 60),
            }
        );
        assert_eq!(all.alias_name, None);

        let daily = items.iter().find(|p| p.name == "finals2000a_daily").expect("finals2000a_daily present");
        assert_eq!(daily.category, "eop");
        assert_eq!(daily.source, "usno");
        assert_eq!(daily.filename, "finals2000A.daily");
        assert_eq!(daily.primary_url(), "https://maia.usno.navy.mil/ser7/finals2000A.daily");
        assert_eq!(daily.schedule, Schedule::Every(Duration::from_secs(24 * 3600)));
        assert_eq!(daily.alias_name, None);
    }

    #[test]
    fn obspm_c04_entry_present() {
        let items = products();
        let c04 = items.iter().find(|p| p.name == "c04_1962now").expect("c04_1962now present");
        assert_eq!(c04.category, "eop");
        assert_eq!(c04.source, "obspm");
        assert_eq!(c04.filename, "eopc04.1962-now");
        assert_eq!(c04.primary_url(), "https://hpiers.obspm.fr/iers/eop/eopc04/eopc04.1962-now");
        assert_eq!(c04.schedule, Schedule::Every(Duration::from_secs(24 * 3600)));
        assert_eq!(c04.alias_name, None);
    }

    #[test]
    fn c04_versioned_entry_aliases_to_c04() {
        let items = products();
        let c04 = items.iter().find(|p| p.name == "c04_20u24").expect("c04_20u24 present");
        assert_eq!(c04.category, "eop");
        assert_eq!(c04.source, "iers");
        assert_eq!(c04.filename, "EOP_C04_one_file_1962-now.txt");
        assert_eq!(c04.alias_name, Some("c04"));
        assert!(c04.primary_url().contains("EOP_20u24_C04_one_file_1962-now.txt"));
    }

    #[test]
    fn celestrak_groups_are_json_under_catalog() {
        let items = products();
        let starlink = items.iter().find(|p| p.name == "starlink").expect("starlink present");
        assert_eq!(starlink.category, "catalog");
        assert_eq!(starlink.source, "celestrak");
        assert_eq!(starlink.filename, "starlink.json");
        assert_eq!(starlink.content_type, "application/json");
        assert!(starlink.primary_url().contains("GROUP=starlink"));
        assert!(starlink.primary_url().contains("FORMAT=json"));
    }

    #[test]
    fn default_registry_passes_validation() {
        assert!(validate_registry(&products()).is_ok());
    }

    #[test]
    fn products_have_expected_schedules() {
        let items = products();
        let get = |name: &str| &items.iter().find(|p| p.name == name).unwrap().schedule;
        assert_eq!(get("finals_all"), &Schedule::Every(Duration::from_secs(24 * 3600)));
        assert_eq!(get("c04_20u24"), &Schedule::Every(Duration::from_secs(7 * 24 * 3600)));
        assert_eq!(get("sw_all"), &Schedule::Every(Duration::from_secs(8 * 3600)));
        assert_eq!(get("starlink"), &Schedule::Every(Duration::from_secs(8 * 3600)));
    }

    #[test]
    fn product_without_a_source_is_rejected() {
        let none = vec![Product {
            category: "eop", source: "iers", name: "sourceless", urls: vec![],
            filename: "f".into(), content_type: "text/plain", gunzip: false,
            availability: Availability::Active, alias_name: None,
            info_url: None, cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(3600)),
        }];
        let err = validate_registry(&none).expect_err("a product with no urls is invalid");
        assert!(err.contains("sourceless"), "{err}");
    }

    #[test]
    fn duplicate_active_alias_is_rejected() {
        let dupes = vec![
            Product { category: "eop", source: "iers", name: "c04_a", urls: vec!["u".into()],
                filename: "f".into(), content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: Some("c04"),
                info_url: None, cadence_label: None,
                schedule: Schedule::Every(Duration::from_secs(3600)) },
            Product { category: "eop", source: "iers", name: "c04_b", urls: vec!["u".into()],
                filename: "f".into(), content_type: "text/plain", gunzip: false, availability: Availability::Active, alias_name: Some("c04"),
                info_url: None, cadence_label: None,
                schedule: Schedule::Every(Duration::from_secs(3600)) },
        ];
        assert!(validate_registry(&dupes).is_err());
    }

    #[test]
    fn known_products_carry_info_urls() {
        let items = products();
        let finals = items.iter().find(|p| p.name == "finals_all").unwrap();
        assert_eq!(finals.info_url, Some("https://www.iers.org/IERS/EN/DataProducts/EarthOrientationData/eop.html"));
        let starlink = items.iter().find(|p| p.name == "starlink").unwrap();
        assert_eq!(starlink.info_url, Some("https://celestrak.org/NORAD/documentation/gp-data-formats.php"));
        // cadence_label defaults to None (interval fallback covers current products)
        assert_eq!(finals.cadence_label, None);
    }

}
