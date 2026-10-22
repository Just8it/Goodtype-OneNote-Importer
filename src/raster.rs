//! Pictures from OneNote: a bounded full decode proves each one is what it claims, and formats
//! Goodtype does not display (GIF, BMP, TIFF) are re-encoded as PNG. The section's original
//! `.one` file, kept in every imported notebook, still holds the source bytes.

use std::io::Cursor;

use image::{ImageFormat, ImageReader, Limits};

pub const MAX_IMAGE_BYTES: usize = goodtype_core::storage::MAX_IMAGE_BYTES;
const MAX_SIDE_PX: u32 = 20_000;
const MAX_PIXELS: u64 = 100_000_000;
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub struct Picture {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
    pub width_px: u32,
    pub height_px: u32,
}

pub fn normalize(bytes: Vec<u8>) -> Result<Picture, String> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("picture is empty or larger than 20 MiB".into());
    }
    let format = image::guess_format(&bytes)
        .map_err(|_| "picture format is not supported (for example EMF or WMF)".to_owned())?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::Bmp
            | ImageFormat::Tiff
    ) {
        return Err(format!("picture format {format:?} is not supported"));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE_PX);
    limits.max_image_height = Some(MAX_SIDE_PX);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("picture cannot be decoded ({error})"))?;
    let (width_px, height_px) = (decoded.width(), decoded.height());
    if width_px == 0 || height_px == 0 || u64::from(width_px) * u64::from(height_px) > MAX_PIXELS {
        return Err("picture dimensions are out of range".into());
    }
    let (bytes, extension) = match format {
        ImageFormat::Png => (bytes, "png"),
        ImageFormat::Jpeg => (bytes, "jpg"),
        _ => {
            let mut out = Cursor::new(Vec::new());
            decoded
                .write_to(&mut out, ImageFormat::Png)
                .map_err(|error| format!("picture cannot be converted to PNG ({error})"))?;
            (out.into_inner(), "png")
        }
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("converted picture is larger than 20 MiB".into());
    }
    Ok(Picture {
        bytes,
        extension,
        width_px,
        height_px,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        RgbImage::from_pixel(3, 2, Rgb([10, 20, 30]))
            .write_to(&mut out, format)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn keeps_png_and_jpeg_and_converts_the_rest() {
        let png = encoded(ImageFormat::Png);
        let kept = normalize(png.clone()).unwrap();
        assert_eq!((kept.extension, kept.bytes, kept.width_px), ("png", png, 3));
        assert_eq!(
            normalize(encoded(ImageFormat::Jpeg)).unwrap().extension,
            "jpg"
        );
        for format in [ImageFormat::Gif, ImageFormat::Bmp, ImageFormat::Tiff] {
            let converted = normalize(encoded(format)).unwrap();
            assert_eq!(converted.extension, "png");
            assert_eq!(
                image::guess_format(&converted.bytes).unwrap(),
                ImageFormat::Png
            );
        }
    }

    #[test]
    fn refuses_unknown_truncated_and_oversized_pictures() {
        assert!(normalize(b"\x01\x00\x00\x00 EMF".to_vec()).is_err());
        let mut png = encoded(ImageFormat::Png);
        png.truncate(png.len() / 2);
        assert!(normalize(png).is_err());
        assert!(normalize(Vec::new()).is_err());
        // A header claiming 30,000 px per side is refused before any pixel buffer exists.
        let mut huge = encoded(ImageFormat::Png);
        huge[16..20].copy_from_slice(&30_000_u32.to_be_bytes());
        huge[20..24].copy_from_slice(&30_000_u32.to_be_bytes());
        assert!(normalize(huge).is_err());
    }
}
