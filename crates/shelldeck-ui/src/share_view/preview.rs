//! Thumbnails of shared files, used by the Share gallery and the
//! « Partages récents » card of the User home.
//!
//! A picture is drawn only from the preview Share serves (`thumbnail_url`),
//! never from the file link: opening a file link counts as a view.

use crate::{icons::lucide_icon, scale::px, theme::ShellDeckColors};
use gpui::{prelude::*, *};
use shelldeck_core::config::share::{
    fetch_thumbnail, local_thumbnail_path, thumbnail_url, ShareSource, ShareUpload,
};
use std::{collections::HashMap, sync::Arc};

/// Previews already fetched (or being fetched), by URL. GPUI's own image
/// loader does not follow the redirect to the Share storage, so the bytes
/// are fetched here and decoded once.
#[derive(Default)]
struct ThumbCache(HashMap<String, Thumb>);
impl Global for ThumbCache {}

#[derive(Clone)]
enum Thumb {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

/// Past this many entries the cache starts over; visible pictures load again.
const THUMB_CACHE_CAP: usize = 160;

/// Decodable image format, read from the bytes themselves.
fn sniff_format(bytes: &[u8]) -> Option<ImageFormat> {
    Some(match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => ImageFormat::Png,
        image::ImageFormat::Jpeg => ImageFormat::Jpeg,
        image::ImageFormat::WebP => ImageFormat::Webp,
        image::ImageFormat::Gif => ImageFormat::Gif,
        image::ImageFormat::Bmp => ImageFormat::Bmp,
        _ => return None,
    })
}

/// The cached preview at `url`, starting its fetch on first sight.
fn thumb(url: &str, cx: &mut App) -> Thumb {
    if let Some(found) = cx.default_global::<ThumbCache>().0.get(url) {
        return found.clone();
    }
    let cache = cx.default_global::<ThumbCache>();
    if cache.0.len() >= THUMB_CACHE_CAP {
        cache.0.clear();
    }
    cache.0.insert(url.to_string(), Thumb::Loading);
    let url = url.to_string();
    cx.spawn(async move |cx| {
        let fetch_url = url.clone();
        let result = cx
            .background_executor()
            .spawn(async move { fetch_thumbnail(&fetch_url) })
            .await;
        let _ = cx.update(|cx| {
            let entry = match result {
                Ok(bytes) => match sniff_format(&bytes) {
                    Some(format) => Thumb::Ready(Arc::new(Image::from_bytes(format, bytes))),
                    None => Thumb::Failed,
                },
                Err(_) => Thumb::Failed,
            };
            cx.default_global::<ThumbCache>().0.insert(url, entry);
            cx.refresh_windows();
        });
    })
    .detach();
    Thumb::Loading
}

/// Longest side of a thumbnail Desktop makes, in pixels.
const LOCAL_THUMB_SIDE: u32 = 480;
/// Largest image Desktop reads to make a thumbnail.
const LOCAL_THUMB_SOURCE_MAX: u64 = 40 * 1024 * 1024;

/// A small PNG of `bytes` (a PNG), or `None` when it cannot be read.
pub(crate) fn make_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let picture = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let small = if picture.width().max(picture.height()) > LOCAL_THUMB_SIDE {
        picture.thumbnail(LOCAL_THUMB_SIDE, LOCAL_THUMB_SIDE)
    } else {
        picture
    };
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// After a successful upload of a PNG from this device, keep a thumbnail of
/// it in the cache so the gallery can show it without fetching the file
/// (opening the file link counts views). Runs on the upload thread; any
/// failure only means the file keeps its icon.
pub(crate) fn save_local_thumbnail(upload: &ShareUpload, id: &str) {
    if upload.content_type != "image/png" {
        return;
    }
    let Some(path) = local_thumbnail_path(id) else {
        return;
    };
    let bytes = match &upload.source {
        ShareSource::Bytes(bytes) => bytes.as_ref().clone(),
        ShareSource::File(file) => match std::fs::metadata(file) {
            Ok(meta) if meta.len() <= LOCAL_THUMB_SOURCE_MAX => match std::fs::read(file) {
                Ok(bytes) => bytes,
                Err(_) => return,
            },
            _ => return,
        },
    };
    let Some(thumbnail) = make_thumbnail(&bytes) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, thumbnail);
}

/// The thumbnail Desktop made of file `id`, read once in the background.
fn local_thumb(id: &str, cx: &mut App) -> Option<Thumb> {
    let key = format!("local:{id}");
    if let Some(found) = cx.default_global::<ThumbCache>().0.get(&key) {
        return match found {
            Thumb::Failed => None,
            other => Some(other.clone()),
        };
    }
    let path = local_thumbnail_path(id)?;
    let cache = cx.default_global::<ThumbCache>();
    if cache.0.len() >= THUMB_CACHE_CAP {
        cache.0.clear();
    }
    cache.0.insert(key.clone(), Thumb::Loading);
    cx.spawn(async move |cx| {
        let bytes = cx
            .background_executor()
            .spawn(async move { std::fs::read(path).ok() })
            .await;
        let _ = cx.update(|cx| {
            let entry = match bytes {
                Some(bytes) if sniff_format(&bytes).is_some() => {
                    Thumb::Ready(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
                }
                _ => Thumb::Failed,
            };
            cx.default_global::<ThumbCache>().0.insert(key, entry);
            cx.refresh_windows();
        });
    })
    .detach();
    Some(Thumb::Loading)
}

/// Kind of a shared file as shown under its name: its extension in capitals.
pub(crate) fn file_kind(filename: &str) -> Option<String> {
    let (stem, extension) = filename.rsplit_once('.')?;
    if stem.is_empty()
        || extension.is_empty()
        || extension.len() > 5
        || !extension.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(extension.to_ascii_uppercase())
}

/// The family of a shared file, for its icon, its tile colour and the
/// gallery filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileFamily {
    Image,
    Archive,
    Document,
}

pub(crate) fn file_family(filename: &str, kind: &str) -> FileFamily {
    match file_kind(filename).as_deref() {
        Some("ZIP" | "GZ" | "TGZ" | "TAR" | "7Z" | "RAR" | "XZ" | "BZ2") => FileFamily::Archive,
        Some("PNG" | "JPG" | "JPEG" | "GIF" | "WEBP" | "SVG" | "AVIF" | "BMP" | "HEIC" | "ICO") => {
            FileFamily::Image
        }
        _ if kind == "image" => FileFamily::Image,
        _ => FileFamily::Document,
    }
}

/// Lucide icon standing for a file that has no picture to show.
pub(crate) fn file_icon(filename: &str) -> &'static str {
    match file_family(filename, "") {
        FileFamily::Image => "file-image",
        FileFamily::Archive => "file-archive",
        FileFamily::Document => "file-text",
    }
}

/// Whether a preview may be fetched: one Share serves for this purpose.
pub(crate) fn has_picture(thumbnail: Option<&str>) -> bool {
    thumbnail.is_some_and(thumbnail_url)
}

/// Corners a thumbnail owns: the top two (picture on top of a card) or the
/// left two (picture beside the text of a flat row).
#[derive(Clone, Copy)]
pub(crate) enum Radius {
    Top(f32),
    Left(f32),
}

impl Radius {
    fn apply<E: Styled>(self, el: E) -> E {
        match self {
            Radius::Top(r) => el.rounded_t(px(r)),
            Radius::Left(r) => el.rounded_l(px(r)),
        }
    }
}

/// Tint of a file's family, so a page of files without pictures still reads
/// at a glance.
fn family_color(family: FileFamily) -> Hsla {
    match family {
        FileFamily::Image => ShellDeckColors::primary(),
        FileFamily::Archive => ShellDeckColors::warning(),
        FileFamily::Document => ShellDeckColors::success(),
    }
}

/// Placeholder for a file without a picture: its icon on a tile of its
/// family's colour, over its kind.
fn placeholder(filename: &str, kind: &str, icon_size: f32, radius: Radius) -> Div {
    let family = file_family(filename, kind);
    let color = family_color(family);
    let mut tile = div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .bg(color.opacity(0.07))
        .child(
            div()
                .size(px(icon_size * 2.0))
                .rounded(px(icon_size * 0.5))
                .flex()
                .items_center()
                .justify_center()
                .bg(color.opacity(0.14))
                .child(lucide_icon(file_icon(filename), icon_size, color)),
        );
    if icon_size >= 20.0 {
        if let Some(kind) = file_kind(filename) {
            tile = tile.child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(kind),
            );
        }
    }
    radius.apply(tile)
}

/// What a thumbnail needs to know about its file.
pub(crate) struct ThumbFile<'a> {
    pub id: &'a str,
    pub filename: &'a str,
    pub kind: &'a str,
    pub thumbnail: Option<&'a str>,
}

/// The thumbnail of a shared file, filling its parent: the preview Share
/// serves once fetched, the file's icon otherwise. The bitmap carries the
/// radius itself: a rounded parent does not clip an image in GPUI
/// (`.agents/window-rounding.md` § Rounded image cards).
pub(crate) fn thumbnail(
    file: ThumbFile<'_>,
    icon_size: f32,
    radius: Radius,
    cx: &mut App,
) -> AnyElement {
    // The thumbnail Desktop made when it shared the file, else the preview
    // Share serves, else the icon.
    let picture = match local_thumb(file.id, cx) {
        Some(thumb) => Some(thumb),
        None => file
            .thumbnail
            .filter(|url| has_picture(Some(url)))
            .map(|url| thumb(url, cx)),
    };
    let Some(picture) = picture else {
        return placeholder(file.filename, file.kind, icon_size, radius).into_any_element();
    };
    match picture {
        // The box clips: an image laid out by its own aspect otherwise
        // paints past the tile.
        Thumb::Ready(image) => div()
            .size_full()
            .overflow_hidden()
            .child(radius.apply(img(image).size_full().object_fit(ObjectFit::Cover)))
            .into_any_element(),
        // While loading, a quiet tile, so a picture does not flash an icon.
        Thumb::Loading => radius
            .apply(div().size_full().bg(ShellDeckColors::hover_bg()))
            .into_any_element(),
        Thumb::Failed => {
            placeholder(file.filename, file.kind, icon_size, radius).into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{file_family, file_icon, file_kind, has_picture, FileFamily};

    // SDTEST-3189
    #[test]
    fn sdtest_3189_previews_come_only_from_share_previews_and_files_have_a_family() {
        assert!(has_picture(Some("https://share.inklura.fr/t/abcdef.webp")));
        // The file link counts views: never a picture source.
        assert!(!has_picture(Some("https://share.inklura.fr/r/abcdef.png")));
        assert!(!has_picture(Some("https://attacker.example/t/x.png")));
        assert!(!has_picture(None));
        assert_eq!(file_family("photo.JPG", ""), FileFamily::Image);
        assert_eq!(file_family("capture", "image"), FileFamily::Image);
        assert_eq!(file_family("backup.tar.gz", "file"), FileFamily::Archive);
        assert_eq!(file_family("devis.pdf", "pdf"), FileFamily::Document);
        assert_eq!(file_icon("photo.JPG"), "file-image");
        assert_eq!(file_kind(".env"), None);
    }

    // SDTEST-3193
    #[test]
    fn sdtest_3193_desktop_makes_a_small_png_of_what_it_shares() {
        let mut source = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1200, 600, image::Rgba([10, 120, 200, 255]))
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        let thumb = super::make_thumbnail(source.get_ref()).expect("thumbnail");
        let small = image::load_from_memory(&thumb).unwrap();
        assert_eq!((small.width(), small.height()), (480, 240));
        assert!(super::make_thumbnail(b"not a png").is_none());
    }

    // SDTEST-3190
    #[test]
    fn sdtest_3190_gallery_fits_as_many_cards_as_the_width_allows() {
        use crate::share_view::page::columns_for_width;
        assert_eq!(columns_for_width(200.0), 1);
        assert_eq!(columns_for_width(476.0), 2);
        assert_eq!(columns_for_width(1088.0), 4);
        assert_eq!(columns_for_width(5000.0), 6);
    }
}
