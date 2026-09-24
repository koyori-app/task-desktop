//! 担当者・コメント投稿者のアイコン。
//!
//! `avatar_url` の画像は URL ごとに 1 度だけ取得してプロセス内にキャッシュする。
//! 未設定・取得中・取得失敗の間は gpui-kit の Avatar が名前の頭文字で描くので、
//! 画面はアイコンの有無を気にしなくてよい。

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::{Sizable, Size};
use gpui_kit::*;

enum Slot {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

#[derive(Default)]
struct AvatarCache(HashMap<String, Slot>);

impl Global for AvatarCache {}

/// `name` の頭文字をフォールバックにしたアイコン。`url` があれば画像を出す。
///
/// `size` は名前付きのサイズだけを使う（`Size::Size(px)` は頭文字の文字サイズが
/// 要素サイズになってはみ出す）。XSmall は 2 文字が収まらないので 1 文字にする。
pub fn user_avatar(name: &str, url: Option<&str>, size: Size, cx: &mut App) -> Avatar {
    let label = if matches!(size, Size::XSmall) {
        name.chars().next().map(String::from).unwrap_or_default()
    } else {
        name.to_string()
    };
    let avatar = Avatar::new().name(label).with_size(size);
    let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) else {
        return avatar;
    };
    let cache = cx.default_global::<AvatarCache>();
    match cache.0.get(url) {
        Some(Slot::Ready(image)) => avatar.src(image.clone()),
        Some(Slot::Loading | Slot::Failed) => avatar,
        None => {
            cache.0.insert(url.to_string(), Slot::Loading);
            let url = url.to_string();
            cx.spawn(async move |cx| {
                let slot = match api::fetch_avatar(&url).await {
                    Ok(fetched) => {
                        match image_format(fetched.content_type.as_deref(), &fetched.bytes) {
                            Some(format) => {
                                Slot::Ready(Arc::new(Image::from_bytes(format, fetched.bytes)))
                            }
                            None => Slot::Failed,
                        }
                    }
                    Err(_) => Slot::Failed,
                };
                cx.update(|cx| {
                    let ready = matches!(slot, Slot::Ready(_));
                    cx.default_global::<AvatarCache>().0.insert(url, slot);
                    // 取得できた時だけ描き直す（失敗時は頭文字のまま変わらない）。
                    if ready {
                        cx.refresh_windows();
                    }
                });
            })
            .detach();
            avatar
        }
    }
}

/// Content-Type を優先し、無い・不明なら先頭バイトで形式を判定する。
fn image_format(content_type: Option<&str>, bytes: &[u8]) -> Option<ImageFormat> {
    if let Some(format) = content_type.and_then(ImageFormat::from_mime_type) {
        return Some(format);
    }
    let head = &bytes[..bytes.len().min(512)];
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if head.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if String::from_utf8_lossy(head).contains("<svg") {
        Some(ImageFormat::Svg)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    // `super::*` だと gpui の `#[test]` マクロが std のものを隠すので個別に import する。
    use super::image_format;
    use gpui_kit::ImageFormat;

    #[test]
    fn detects_format_from_content_type_then_magic_bytes() {
        assert_eq!(image_format(Some("image/png"), b""), Some(ImageFormat::Png));
        assert_eq!(
            image_format(None, b"\x89PNG\r\n\x1a\n...."),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            image_format(Some("application/octet-stream"), &[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            image_format(None, b"RIFF\0\0\0\0WEBPVP8 "),
            Some(ImageFormat::Webp)
        );
        assert_eq!(
            image_format(None, b"<?xml version=\"1.0\"?><svg xmlns=\"\"/>"),
            Some(ImageFormat::Svg)
        );
        assert_eq!(image_format(None, b"not an image"), None);
    }
}
