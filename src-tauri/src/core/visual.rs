use std::cell::RefCell;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    IWICImagingFactory, CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

use crate::error::{AppError, AppResult};

pub const HASH_VERSION: i64 = 1;
pub const SAMPLE_SIZE: u32 = 64;
pub const MIN_DIMENSION: u32 = 16;
pub const DEFAULT_THRESHOLD: u32 = 8;
pub const MAX_THRESHOLD: u32 = 24;
pub const ASPECT_TOLERANCE: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualHashes {
    pub rotations: [u64; 4],
    pub width: u32,
    pub height: u32,
}

impl VisualHashes {
    pub fn aspect_ratio(&self) -> f32 {
        let (long, short) = if self.width >= self.height {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        };
        long as f32 / short.max(1) as f32
    }
}

pub fn aspect_compatible(a: f32, b: f32) -> bool {
    let largest = a.max(b);
    largest > 0.0 && (a - b).abs() <= largest * ASPECT_TOLERANCE
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn best_distance(rotations: &[u64; 4], other: u64) -> u32 {
    rotations
        .iter()
        .map(|value| hamming(*value, other))
        .min()
        .unwrap_or(u32::MAX)
}

pub fn similarity(distance: u32) -> f32 {
    1.0 - distance as f32 / 64.0
}

pub fn compute(path: &Path) -> AppResult<VisualHashes> {
    compute_wic(path)
}

fn wic_error(error: windows::core::Error) -> AppError {
    AppError::user(
        "VISUAL_DECODE_FAILED",
        format!("无法解码图片进行视觉比对：{error}"),
    )
}

thread_local! {
    static WIC_FACTORY: RefCell<Option<IWICImagingFactory>> = const { RefCell::new(None) };
}

fn wic_factory() -> AppResult<IWICImagingFactory> {
    WIC_FACTORY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(factory) = slot.as_ref() {
            return Ok(factory.clone());
        }
        // COM is apartment-scoped. Workers live for the whole process, so no CoUninitialize.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let factory: IWICImagingFactory = unsafe {
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
        }
        .map_err(|error| {
            AppError::user(
                "VISUAL_DECODER_UNAVAILABLE",
                format!("系统图片解码器不可用：{error}"),
            )
        })?;
        *slot = Some(factory.clone());
        Ok(factory)
    })
}

fn compute_wic(path: &Path) -> AppResult<VisualHashes> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let factory = wic_factory()?;
    let (width, height, gray) = unsafe {
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(wic_error)?;
        let frame = decoder.GetFrame(0).map_err(wic_error)?;
        let mut width = 0u32;
        let mut height = 0u32;
        frame.GetSize(&mut width, &mut height).map_err(wic_error)?;
        if width < MIN_DIMENSION || height < MIN_DIMENSION {
            return Err(AppError::user(
                "IMAGE_TOO_SMALL",
                format!("图片尺寸过小（{width}×{height}），跳过视觉比对。"),
            ));
        }

        let converter = factory.CreateFormatConverter().map_err(wic_error)?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(wic_error)?;

        let scaler = factory.CreateBitmapScaler().map_err(wic_error)?;
        scaler
            .Initialize(
                &converter,
                SAMPLE_SIZE,
                SAMPLE_SIZE,
                WICBitmapInterpolationModeFant,
            )
            .map_err(wic_error)?;
        let mut pixels = vec![0u8; (SAMPLE_SIZE * SAMPLE_SIZE * 4) as usize];
        scaler
            .CopyPixels(std::ptr::null(), SAMPLE_SIZE * 4, &mut pixels)
            .map_err(wic_error)?;
        (width, height, grayscale(&pixels))
    };

    if variance(&gray) < 4.0 {
        return Err(AppError::user(
            "IMAGE_LOW_DETAIL",
            "图片缺少可辨识细节，跳过视觉比对。",
        ));
    }

    let mut rotations = [0u64; 4];
    let mut current = gray;
    for value in &mut rotations {
        *value = dhash(&current);
        current = rotate_cw(&current);
    }
    Ok(VisualHashes {
        rotations,
        width,
        height,
    })
}

fn grayscale(pixels: &[u8]) -> [[u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize] {
    let mut result = [[0u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize];
    for y in 0..SAMPLE_SIZE as usize {
        for x in 0..SAMPLE_SIZE as usize {
            let offset = (y * SAMPLE_SIZE as usize + x) * 4;
            let alpha = pixels[offset + 3] as u32;
            let blend = |value: u8| -> u32 {
                (value as u32 * alpha + 255 * (255 - alpha) + 127) / 255
            };
            let blue = blend(pixels[offset]);
            let green = blend(pixels[offset + 1]);
            let red = blend(pixels[offset + 2]);
            let luma = (red * 299 + green * 587 + blue * 114 + 500) / 1000;
            result[y][x] = luma.min(255) as u8;
        }
    }
    result
}

fn variance(gray: &[[u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize]) -> f64 {
    let count = (SAMPLE_SIZE * SAMPLE_SIZE) as f64;
    let sum: f64 = gray.iter().flat_map(|row| row.iter()).map(|v| *v as f64).sum();
    let mean = sum / count;
    let total: f64 = gray
        .iter()
        .flat_map(|row| row.iter())
        .map(|v| {
            let delta = *v as f64 - mean;
            delta * delta
        })
        .sum();
    total / count
}

fn rotate_cw(
    gray: &[[u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize],
) -> [[u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize] {
    let size = SAMPLE_SIZE as usize;
    let mut rotated = [[0u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize];
    for y in 0..size {
        for x in 0..size {
            rotated[size - 1 - x][y] = gray[y][x];
        }
    }
    rotated
}

fn dhash(gray: &[[u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize]) -> u64 {
    let mut small = [[0u8; 9]; 8];
    for (y, row) in small.iter_mut().enumerate() {
        let y0 = y * SAMPLE_SIZE as usize / 8;
        let y1 = ((y + 1) * SAMPLE_SIZE as usize / 8).max(y0 + 1);
        for (x, value) in row.iter_mut().enumerate() {
            let x0 = x * SAMPLE_SIZE as usize / 9;
            let x1 = ((x + 1) * SAMPLE_SIZE as usize / 9).max(x0 + 1);
            let mut sum = 0u32;
            let mut count = 0u32;
            for line in gray.iter().take(y1).skip(y0) {
                for pixel in line.iter().take(x1).skip(x0) {
                    sum += *pixel as u32;
                    count += 1;
                }
            }
            *value = (sum / count.max(1)) as u8;
        }
    }

    let mut hash = 0u64;
    for row in &small {
        for x in 0..8 {
            hash = (hash << 1) | u64::from(row[x] > row[x + 1]);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_are_cycle_of_four() {
        let mut gray = [[0u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize];
        gray[0][0] = 255;
        gray[0][63] = 128;
        let once = rotate_cw(&gray);
        let twice = rotate_cw(&once);
        let thrice = rotate_cw(&twice);
        let four = rotate_cw(&thrice);
        assert_eq!(four, gray);
        assert_eq!(once[63][0], 255);
        assert_eq!(once[0][0], 128);
    }

    #[test]
    fn dhash_is_stable_for_solid_image_and_changes_with_pattern() {
        let mut gray = [[32u8; SAMPLE_SIZE as usize]; SAMPLE_SIZE as usize];
        assert_eq!(dhash(&gray), 0);
        for y in 0..SAMPLE_SIZE as usize {
            for x in 0..SAMPLE_SIZE as usize / 2 {
                gray[y][x] = 220;
            }
        }
        let first = dhash(&gray);
        assert_ne!(first, 0);
        assert_eq!(first, dhash(&gray));
    }

    #[test]
    fn hamming_matches_expectations() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(0, 0b1011), 3);
        assert_eq!(best_distance(&[0, u64::MAX, 0, 0], 0), 0);
        assert!((similarity(0) - 1.0).abs() < f32::EPSILON);
        assert!((similarity(64) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn aspect_compatibility_allows_small_changes() {
        assert!(aspect_compatible(1.5, 1.55));
        assert!(!aspect_compatible(1.0, 1.2));
    }

    fn fixture(name: &str) -> VisualHashes {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/visual")
            .join(name);
        compute(&path).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    #[test]
    fn wic_hashes_compressed_and_rotated_images_close() {
        let base = fixture("base.png");
        let compressed = fixture("base-q60.jpg");
        let resized = fixture("base-resized-q55.jpg");
        let rotated = fixture("base-rot90.png");
        let other = fixture("other.png");

        let compressed_distance = best_distance(&base.rotations, compressed.rotations[0]);
        let resized_distance = best_distance(&base.rotations, resized.rotations[0]);
        let rotated_distance = best_distance(&base.rotations, rotated.rotations[0]);
        let other_distance = best_distance(&base.rotations, other.rotations[0]);
        eprintln!(
            "compressed={compressed_distance} resized={resized_distance} rotated={rotated_distance} other={other_distance}"
        );

        assert!(compressed_distance <= DEFAULT_THRESHOLD);
        assert!(resized_distance <= DEFAULT_THRESHOLD);
        assert!(rotated_distance <= 2);
        assert!(other_distance > DEFAULT_THRESHOLD);
    }

    #[test]
    fn low_detail_image_is_rejected() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/visual/solid.png");
        let error = compute(&path).expect_err("solid image should be rejected");
        assert_eq!(error.code(), "IMAGE_LOW_DETAIL");
    }
}
