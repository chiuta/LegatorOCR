//! The page-cleaning core as a WebAssembly module with a plain C interface, so a web page (or the
//! phone-capture page in `web/`) can use it without any build tool: copy bytes in, call
//! `lgr_process`, read the PNG bytes of the first resulting page out.
//!
//! Calls are not thread-safe (one page at a time, state kept in the module). The module has no
//! OCR; it does what the command line does before OCR: orientation, deskew, content selection,
//! binarisation. Memory is limited by the browser (4 GiB of wasm32 address space at most).

use legatorocr_core::pipeline::{process_page, OutputMode, Settings};
use std::sync::Mutex;

static OUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Allocate `n` bytes inside the module for the caller to fill.
#[no_mangle]
pub extern "C" fn lgr_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::new();
    // a request the module cannot satisfy gives a null pointer instead of aborting the module
    if v.try_reserve_exact(n.max(1)).is_err() {
        return std::ptr::null_mut();
    }
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` must come from `lgr_alloc(n)` with the same `n`, and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn lgr_free(ptr: *mut u8, n: usize) {
    if ptr.is_null() {
        return;
    }
    drop(Vec::from_raw_parts(ptr, 0, n.max(1)));
}

fn decode(bytes: &[u8]) -> Result<image::DynamicImage, ()> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1 << 30);
    r.limits(limits);
    use image::ImageDecoder;
    let dec = r.into_decoder().map_err(|_| ())?;
    // refuse a picture whose buffer alone would not fit (a hostile 60 000 x 60 000 header)
    if dec.total_bytes() > (1u64 << 30) {
        return Err(());
    }
    image::DynamicImage::from_decoder(dec).map_err(|_| ())
}

fn settings(mode: u32, dpi: u32) -> Settings {
    Settings {
        mode: match mode {
            0 => OutputMode::Color,
            1 => OutputMode::Gray,
            2 => OutputMode::BlackWhite,
            3 => OutputMode::Mixed,
            _ => OutputMode::Hybrid,
        },
        dpi: if dpi == 0 { 300 } else { dpi },
        ..Settings::default()
    }
}

/// Clean the picture at `ptr`/`len` (PNG, JPEG, BMP or TIFF). `mode`: 0 colour, 1 grey, 2 black &
/// white, 3 mixed, 4 hybrid. Returns the number of pages produced (a double-page spread can give
/// two) or a negative number: -1 the bytes are not a readable image, -2 no page came out.
/// The first page is available as PNG through `lgr_out_ptr` / `lgr_out_len`.
///
/// # Safety
/// `ptr` must point to `len` readable bytes inside this module.
#[no_mangle]
pub unsafe extern "C" fn lgr_process(ptr: *const u8, len: usize, mode: u32, dpi: u32) -> i32 {
    if ptr.is_null() || len == 0 {
        return -1;
    }
    let bytes = std::slice::from_raw_parts(ptr, len);
    let Ok(img) = decode(bytes) else { return -1 };
    let pages = process_page(&img, &settings(mode, dpi));
    let Some(first) = pages.first() else {
        return -2;
    };
    let mut png = Vec::new();
    if first
        .image
        .to_dynamic()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_err()
    {
        return -2;
    }
    *OUT.lock().unwrap() = png;
    pages.len() as i32
}

/// Sharpness of the picture (higher = sharper) times 1000, rounded; -1 when unreadable. For a live
/// "is this photo sharp enough" verdict before the shutter is released.
///
/// # Safety
/// `ptr` must point to `len` readable bytes inside this module.
#[no_mangle]
pub unsafe extern "C" fn lgr_sharpness(ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() || len == 0 {
        return -1;
    }
    let Ok(img) = decode(std::slice::from_raw_parts(ptr, len)) else {
        return -1;
    };
    (legatorocr_core::quality::sharpness(&img.to_luma8()) * 1000.0).round() as i32
}

#[no_mangle]
pub extern "C" fn lgr_out_ptr() -> *const u8 {
    OUT.lock().unwrap().as_ptr()
}

#[no_mangle]
pub extern "C" fn lgr_out_len() -> usize {
    OUT.lock().unwrap().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_a_png_and_read_the_result_back() {
        let mut img = image::GrayImage::from_pixel(300, 400, image::Luma([240]));
        for y in 100..110 {
            for x in 40..260 {
                img.put_pixel(x, y, image::Luma([20]));
            }
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        unsafe {
            let p = lgr_alloc(png.len());
            std::ptr::copy_nonoverlapping(png.as_ptr(), p, png.len());
            assert!(lgr_sharpness(p, png.len()) >= 0);
            assert_eq!(lgr_process(p, png.len(), 2, 300), 1);
            lgr_free(p, png.len());
            assert_eq!(lgr_process(b"nope".as_ptr(), 4, 2, 300), -1);
        }
        let out = unsafe { std::slice::from_raw_parts(lgr_out_ptr(), lgr_out_len()) }.to_vec();
        assert_eq!(&out[..4], b"\x89PNG");
    }

    #[test]
    fn null_empty_and_absurd_requests_are_refused_not_fatal() {
        unsafe {
            assert_eq!(lgr_process(std::ptr::null(), 10, 2, 300), -1);
            assert_eq!(lgr_sharpness(std::ptr::null(), 10), -1);
            let p = lgr_alloc(4);
            assert_eq!(lgr_process(p, 0, 2, 300), -1);
            lgr_free(p, 4);
            lgr_free(std::ptr::null_mut(), 0);
        }
        assert!(lgr_alloc(usize::MAX / 2).is_null());
        // bytes that are no picture at all
        let junk = [0x42u8; 64];
        assert_eq!(unsafe { lgr_process(junk.as_ptr(), junk.len(), 2, 300) }, -1);
    }

    #[test]
    fn hostile_dpi_and_tiny_pictures_do_not_panic() {
        for (w, h) in [(1u32, 1u32), (2, 700), (700, 2)] {
            let img = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(w, h, image::Luma([200])));
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            for dpi in [1u32, 72, 100_000, u32::MAX] {
                for mode in 0..6u32 {
                    let _ = unsafe { lgr_process(png.as_ptr(), png.len(), mode, dpi) };
                }
            }
        }
    }
}
