//! What the Share gallery shows: filter, order and the dates under a card.
//! Pure functions, so the rules are tested without rendering.

use super::preview::{file_family, FileFamily};
use crate::t;
use shelldeck_core::config::share::ShareFile;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FileFilter {
    #[default]
    All,
    Images,
    Documents,
    Archives,
}

impl FileFilter {
    pub(crate) const ALL: [FileFilter; 4] = [
        FileFilter::All,
        FileFilter::Images,
        FileFilter::Documents,
        FileFilter::Archives,
    ];

    pub(crate) fn label(self) -> String {
        match self {
            FileFilter::All => t!("share.filter.all"),
            FileFilter::Images => t!("share.filter.images"),
            FileFilter::Documents => t!("share.filter.documents"),
            FileFilter::Archives => t!("share.filter.archives"),
        }
        .to_string()
    }

    pub(crate) fn icon(self) -> Option<&'static str> {
        match self {
            FileFilter::All => None,
            FileFilter::Images => Some("image"),
            FileFilter::Documents => Some("file-text"),
            FileFilter::Archives => Some("file-archive"),
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            FileFilter::All => "all",
            FileFilter::Images => "images",
            FileFilter::Documents => "documents",
            FileFilter::Archives => "archives",
        }
    }

    fn keeps(self, file: &ShareFile) -> bool {
        let family = file_family(&file.filename, &file.kind);
        match self {
            FileFilter::All => true,
            FileFilter::Images => family == FileFamily::Image,
            FileFilter::Documents => family == FileFamily::Document,
            FileFilter::Archives => family == FileFamily::Archive,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FileSort {
    #[default]
    Newest,
    Oldest,
    Name,
    Largest,
}

impl FileSort {
    pub(crate) const ALL: [FileSort; 4] = [
        FileSort::Newest,
        FileSort::Oldest,
        FileSort::Name,
        FileSort::Largest,
    ];

    pub(crate) fn label(self) -> String {
        match self {
            FileSort::Newest => t!("share.sort.newest"),
            FileSort::Oldest => t!("share.sort.oldest"),
            FileSort::Name => t!("share.sort.name"),
            FileSort::Largest => t!("share.sort.largest"),
        }
        .to_string()
    }
}

/// The files the gallery shows, in order: name matching `query` (ignoring
/// case), kept by `filter`, sorted by `sort`.
pub(crate) fn gallery_files(
    files: &[ShareFile],
    query: &str,
    filter: FileFilter,
    sort: FileSort,
) -> Vec<ShareFile> {
    let query = query.trim().to_lowercase();
    let mut out: Vec<ShareFile> = files
        .iter()
        .filter(|file| query.is_empty() || file.filename.to_lowercase().contains(&query))
        .filter(|file| filter.keeps(file))
        .cloned()
        .collect();
    match sort {
        // RFC 3339 timestamps from one server sort lexicographically.
        FileSort::Newest => out.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        FileSort::Oldest => out.sort_by(|a, b| a.created_at.cmp(&b.created_at)),
        FileSort::Name => out.sort_by_cached_key(|file| file.filename.to_lowercase()),
        FileSort::Largest => out.sort_by_key(|file| std::cmp::Reverse(file.bytes)),
    }
    out
}

fn local_date(value: &str) -> Option<chrono::NaiveDate> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&chrono::Local).date_naive())
}

/// When a file was shared, by calendar day: « Aujourd’hui », « Hier »,
/// « Il y a N jours », then the date.
pub(crate) fn day_label(created_at: &str, today: chrono::NaiveDate) -> Option<String> {
    let day = local_date(created_at)?;
    let days = (today - day).num_days();
    Some(match days {
        i64::MIN..=0 => t!("share.day.today").to_string(),
        1 => t!("share.day.yesterday").to_string(),
        2..=6 => t!("share.day.days_ago", count = days).to_string(),
        _ => day.format("%d/%m/%Y").to_string(),
    })
}

/// How long a link still works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Expiry {
    Never,
    /// Within [`EXPIRY_SOON_DAYS`]: shown in the warning colour.
    Soon(String),
    Later(String),
}

pub(crate) const EXPIRY_SOON_DAYS: i64 = 3;

pub(crate) fn expiry(expires_at: Option<&str>, today: chrono::NaiveDate) -> Expiry {
    let Some(day) = expires_at.and_then(local_date) else {
        return Expiry::Never;
    };
    let days = (day - today).num_days();
    let label = match days {
        i64::MIN..=0 => t!("share.expires.today").to_string(),
        1 => t!("share.expires.tomorrow").to_string(),
        _ => t!("share.expires.in_days", count = days).to_string(),
    };
    if days <= EXPIRY_SOON_DAYS {
        Expiry::Soon(label)
    } else {
        Expiry::Later(label)
    }
}

#[cfg(test)]
mod tests {
    use super::{day_label, expiry, gallery_files, Expiry, FileFilter, FileSort};
    use shelldeck_core::config::share::ShareFile;

    fn file(name: &str, kind: &str, bytes: u64, created: &str) -> ShareFile {
        ShareFile {
            id: name.into(),
            filename: name.into(),
            kind: kind.into(),
            bytes,
            created_at: created.into(),
            ..Default::default()
        }
    }

    // SDTEST-3192
    #[test]
    fn sdtest_3192_gallery_filters_sorts_and_dates_files() {
        let files = vec![
            file("Capture.png", "image", 300, "2026-11-14T12:00:00Z"),
            file("devis.pdf", "pdf", 900, "2026-11-16T09:00:00Z"),
            file("export.zip", "file", 100, "2026-11-10T09:00:00Z"),
            file("arbre.jpg", "image", 500, "2026-11-15T09:00:00Z"),
        ];
        let names = |q: &str, f, s| -> Vec<String> {
            gallery_files(&files, q, f, s)
                .into_iter()
                .map(|f| f.filename)
                .collect()
        };
        assert_eq!(
            names("", FileFilter::All, FileSort::Newest),
            ["devis.pdf", "arbre.jpg", "Capture.png", "export.zip"]
        );
        assert_eq!(
            names("", FileFilter::Images, FileSort::Name),
            ["arbre.jpg", "Capture.png"]
        );
        assert_eq!(
            names("", FileFilter::Archives, FileSort::Oldest),
            ["export.zip"]
        );
        assert_eq!(
            names("", FileFilter::Documents, FileSort::Largest),
            ["devis.pdf"]
        );
        assert_eq!(
            names(" CAPT ", FileFilter::All, FileSort::Newest),
            ["Capture.png"]
        );

        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let noon = |d: &str| format!("{d}T12:00:00+00:00");
        assert_eq!(
            day_label(&noon("2026-11-16"), today).as_deref(),
            Some(crate::t!("share.day.today").as_ref())
        );
        assert_eq!(
            day_label(&noon("2026-11-15"), today).as_deref(),
            Some(crate::t!("share.day.yesterday").as_ref())
        );
        assert_eq!(
            day_label(&noon("2026-10-01"), today).as_deref(),
            Some("01/10/2026")
        );
        assert_eq!(expiry(None, today), Expiry::Never);
        assert!(matches!(
            expiry(Some(&noon("2026-11-18")), today),
            Expiry::Soon(_)
        ));
        assert!(matches!(
            expiry(Some(&noon("2026-12-16")), today),
            Expiry::Later(_)
        ));
    }
}
