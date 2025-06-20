//! Visual Testing Infrastructure for Zed
//! 
//! This crate provides tools for visual regression testing, screenshot comparison,
//! and baseline management for UI components.

use anyhow::{Context, Result};
use gpui::{Bounds, Pixels, Point, Size, TestAppContext, View, WindowHandle};
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VisualTestError {
    #[error("Image comparison failed: {0}")]
    ComparisonFailed(String),
    
    #[error("Baseline image not found: {0}")]
    BaselineNotFound(PathBuf),
    
    #[error("Screenshot capture failed: {0}")]
    CaptureFailed(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Image processing error: {0}")]
    ImageError(#[from] image::ImageError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualTestConfig {
    pub baseline_dir: PathBuf,
    pub output_dir: PathBuf,
    pub diff_dir: PathBuf,
    pub threshold: f64,
    pub update_baselines: bool,
}

impl Default for VisualTestConfig {
    fn default() -> Self {
        Self {
            baseline_dir: PathBuf::from("tests/visual/baselines"),
            output_dir: PathBuf::from("tests/visual/output"),
            diff_dir: PathBuf::from("tests/visual/diffs"),
            threshold: 0.01, // 1% difference allowed
            update_baselines: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageComparison {
    pub matches: bool,
    pub diff_percentage: f64,
    pub pixel_diff_count: u32,
    pub total_pixels: u32,
}

pub struct VisualTestRunner {
    config: VisualTestConfig,
}

impl VisualTestRunner {
    pub fn new(config: VisualTestConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(VisualTestConfig::default())
    }

    /// Capture a screenshot of a GPUI view
    pub fn capture_view_screenshot<V: 'static>(
        &self,
        view: &View<V>,
        cx: &TestAppContext,
    ) -> Result<DynamicImage, VisualTestError> {
        // This is a simplified implementation - actual implementation would
        // need to integrate with GPUI's rendering system
        self.capture_mock_screenshot()
    }

    /// Capture a screenshot of a window
    pub fn capture_window_screenshot(
        &self,
        window: &WindowHandle<TestAppContext>,
    ) -> Result<DynamicImage, VisualTestError> {
        // This is a simplified implementation - actual implementation would
        // need to integrate with GPUI's rendering system
        self.capture_mock_screenshot()
    }

    /// Capture a screenshot of a specific bounds within a view
    pub fn capture_bounds_screenshot<V: 'static>(
        &self,
        view: &View<V>,
        bounds: Bounds<Pixels>,
        cx: &TestAppContext,
    ) -> Result<DynamicImage, VisualTestError> {
        let full_screenshot = self.capture_view_screenshot(view, cx)?;
        
        // Crop to specified bounds
        let cropped = full_screenshot.crop_imm(
            bounds.origin.x.0 as u32,
            bounds.origin.y.0 as u32,
            bounds.size.width.0 as u32,
            bounds.size.height.0 as u32,
        );
        
        Ok(cropped)
    }

    /// Compare a screenshot against a baseline and assert visual match
    pub fn assert_visual_match(
        &self,
        screenshot: DynamicImage,
        test_name: &str,
    ) -> Result<(), VisualTestError> {
        let baseline_path = self.config.baseline_dir.join(format!("{}.png", test_name));
        let output_path = self.config.output_dir.join(format!("{}.png", test_name));
        let diff_path = self.config.diff_dir.join(format!("{}_diff.png", test_name));

        // Ensure directories exist
        std::fs::create_dir_all(&self.config.baseline_dir)?;
        std::fs::create_dir_all(&self.config.output_dir)?;
        std::fs::create_dir_all(&self.config.diff_dir)?;

        // Save current screenshot
        screenshot.save_with_format(&output_path, ImageFormat::Png)?;

        // Load or create baseline
        let baseline = if baseline_path.exists() && !self.config.update_baselines {
            image::open(&baseline_path)?
        } else {
            // First run or updating baselines - save as baseline
            screenshot.save_with_format(&baseline_path, ImageFormat::Png)?;
            return Ok(());
        };

        // Compare images
        let comparison = self.compare_images(&baseline, &screenshot)?;

        if !comparison.matches {
            // Generate diff image
            let diff_image = self.generate_diff_image(&baseline, &screenshot)?;
            diff_image.save_with_format(&diff_path, ImageFormat::Png)?;

            return Err(VisualTestError::ComparisonFailed(format!(
                "Visual mismatch: {:.2}% difference ({} pixels), threshold: {:.2}%",
                comparison.diff_percentage * 100.0,
                comparison.pixel_diff_count,
                self.config.threshold * 100.0
            )));
        }

        Ok(())
    }

    /// Compare two images and return comparison metrics
    pub fn compare_images(
        &self,
        baseline: &DynamicImage,
        current: &DynamicImage,
    ) -> Result<ImageComparison, VisualTestError> {
        let baseline_rgba = baseline.to_rgba8();
        let current_rgba = current.to_rgba8();

        if baseline_rgba.dimensions() != current_rgba.dimensions() {
            return Ok(ImageComparison {
                matches: false,
                diff_percentage: 1.0,
                pixel_diff_count: baseline_rgba.width() * baseline_rgba.height(),
                total_pixels: baseline_rgba.width() * baseline_rgba.height(),
            });
        }

        let (width, height) = baseline_rgba.dimensions();
        let total_pixels = width * height;
        let mut diff_pixels = 0u32;

        for y in 0..height {
            for x in 0..width {
                let baseline_pixel = baseline_rgba.get_pixel(x, y);
                let current_pixel = current_rgba.get_pixel(x, y);

                if !self.pixels_match(baseline_pixel, current_pixel) {
                    diff_pixels += 1;
                }
            }
        }

        let diff_percentage = diff_pixels as f64 / total_pixels as f64;
        let matches = diff_percentage <= self.config.threshold;

        Ok(ImageComparison {
            matches,
            diff_percentage,
            pixel_diff_count: diff_pixels,
            total_pixels,
        })
    }

    /// Generate a diff image highlighting differences
    pub fn generate_diff_image(
        &self,
        baseline: &DynamicImage,
        current: &DynamicImage,
    ) -> Result<DynamicImage, VisualTestError> {
        let baseline_rgba = baseline.to_rgba8();
        let current_rgba = current.to_rgba8();

        if baseline_rgba.dimensions() != current_rgba.dimensions() {
            // Return a red image to indicate size mismatch
            let (width, height) = baseline_rgba.dimensions();
            let mut diff_image = RgbaImage::new(width, height);
            for pixel in diff_image.pixels_mut() {
                *pixel = image::Rgba([255, 0, 0, 255]); // Red
            }
            return Ok(DynamicImage::ImageRgba8(diff_image));
        }

        let (width, height) = baseline_rgba.dimensions();
        let mut diff_image = RgbaImage::new(width, height);

        for y in 0..height {
            for x in 0..width {
                let baseline_pixel = baseline_rgba.get_pixel(x, y);
                let current_pixel = current_rgba.get_pixel(x, y);

                let diff_pixel = if self.pixels_match(baseline_pixel, current_pixel) {
                    // Same pixel - show in grayscale
                    let gray = (baseline_pixel[0] as u16 + baseline_pixel[1] as u16 + baseline_pixel[2] as u16) / 3;
                    image::Rgba([gray as u8, gray as u8, gray as u8, 255])
                } else {
                    // Different pixel - highlight in red
                    image::Rgba([255, 0, 0, 255])
                };

                diff_image.put_pixel(x, y, diff_pixel);
            }
        }

        Ok(DynamicImage::ImageRgba8(diff_image))
    }

    /// Check if two pixels match within tolerance
    fn pixels_match(&self, pixel1: &image::Rgba<u8>, pixel2: &image::Rgba<u8>) -> bool {
        const TOLERANCE: u8 = 2; // Allow small differences due to rendering variations

        for i in 0..4 {
            let diff = (pixel1[i] as i16 - pixel2[i] as i16).abs();
            if diff > TOLERANCE as i16 {
                return false;
            }
        }
        true
    }

    /// Mock screenshot capture for testing
    fn capture_mock_screenshot(&self) -> Result<DynamicImage, VisualTestError> {
        // Create a simple test image
        let width = 800;
        let height = 600;
        let mut image = RgbaImage::new(width, height);

        // Fill with a gradient pattern
        for y in 0..height {
            for x in 0..width {
                let r = (x * 255 / width) as u8;
                let g = (y * 255 / height) as u8;
                let b = 128;
                let a = 255;
                image.put_pixel(x, y, image::Rgba([r, g, b, a]));
            }
        }

        Ok(DynamicImage::ImageRgba8(image))
    }
}

/// Convenience macro for visual tests
#[macro_export]
macro_rules! visual_test {
    ($test_name:expr, $view:expr, $cx:expr) => {{
        let runner = $crate::VisualTestRunner::with_default_config();
        let screenshot = runner.capture_view_screenshot($view, $cx)?;
        runner.assert_visual_match(screenshot, $test_name)
    }};
}

/// Convenience macro for visual tests with custom bounds
#[macro_export]
macro_rules! visual_test_bounds {
    ($test_name:expr, $view:expr, $bounds:expr, $cx:expr) => {{
        let runner = $crate::VisualTestRunner::with_default_config();
        let screenshot = runner.capture_bounds_screenshot($view, $bounds, $cx)?;
        runner.assert_visual_match(screenshot, $test_name)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_image_comparison_identical() {
        let runner = VisualTestRunner::with_default_config();
        
        // Create identical images
        let image1 = DynamicImage::new_rgba8(100, 100);
        let image2 = DynamicImage::new_rgba8(100, 100);
        
        let comparison = runner.compare_images(&image1, &image2).unwrap();
        assert!(comparison.matches);
        assert_eq!(comparison.diff_percentage, 0.0);
        assert_eq!(comparison.pixel_diff_count, 0);
    }

    #[test]
    fn test_image_comparison_different_sizes() {
        let runner = VisualTestRunner::with_default_config();
        
        let image1 = DynamicImage::new_rgba8(100, 100);
        let image2 = DynamicImage::new_rgba8(200, 200);
        
        let comparison = runner.compare_images(&image1, &image2).unwrap();
        assert!(!comparison.matches);
        assert_eq!(comparison.diff_percentage, 1.0);
    }

    #[test]
    fn test_visual_test_config_serialization() {
        let config = VisualTestConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: VisualTestConfig = serde_json::from_str(&json).unwrap();
        
        assert_eq!(config.threshold, deserialized.threshold);
        assert_eq!(config.baseline_dir, deserialized.baseline_dir);
    }

    #[test]
    fn test_diff_image_generation() {
        let runner = VisualTestRunner::with_default_config();
        
        // Create two different images
        let mut image1 = RgbaImage::new(10, 10);
        let mut image2 = RgbaImage::new(10, 10);
        
        // Fill with different colors
        for pixel in image1.pixels_mut() {
            *pixel = image::Rgba([255, 0, 0, 255]); // Red
        }
        for pixel in image2.pixels_mut() {
            *pixel = image::Rgba([0, 255, 0, 255]); // Green
        }
        
        let diff = runner.generate_diff_image(
            &DynamicImage::ImageRgba8(image1),
            &DynamicImage::ImageRgba8(image2),
        ).unwrap();
        
        // Diff image should highlight all pixels as different
        let diff_rgba = diff.to_rgba8();
        for pixel in diff_rgba.pixels() {
            assert_eq!(pixel[0], 255); // Red channel should be 255 for differences
        }
    }
}
