use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose};
use screenshots::Screen;
use screenshots::image::{DynamicImage, ImageFormat};
use std::io::Cursor;

pub struct ScreenCapture;

impl ScreenCapture {
    pub fn capture_main() -> Result<Vec<u8>> {
        let screens = Screen::all().context("Failed to get screens")?;
        let screen = screens.first().context("No screens found")?;
        let image = screen.capture().context("Failed to capture screen")?;

        // Logical scale factor (Retina handling)
        let scale_factor = screen.display_info.scale_factor;
        let logical_w = (screen.display_info.width as f32 / scale_factor) as u32;
        let logical_h = (screen.display_info.height as f32 / scale_factor) as u32;

        let dynamic_image = DynamicImage::ImageRgba8(image);
        let resized = dynamic_image.resize(
            logical_w,
            logical_h,
            screenshots::image::imageops::FilterType::Triangle,
        );

        let mut buffer = Cursor::new(Vec::new());
        resized
            .write_to(&mut buffer, ImageFormat::Jpeg)
            .context("Failed to encode image")?;

        Ok(buffer.into_inner())
    }

    pub fn capture_main_base64() -> Result<String> {
        let bytes = Self::capture_main()?;
        Ok(general_purpose::STANDARD.encode(&bytes))
    }
}
