#[allow(dead_code)]
#[path = "../src/image_loader.rs"]
mod image_loader;

#[allow(dead_code)]
#[path = "../src/media_index.rs"]
mod media_index;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs::File;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn create_scan_dataset(file_count: usize) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let mut first_media: Option<PathBuf> = None;

    for idx in 0..file_count {
        let ext = match idx % 5 {
            0 => "jpg",
            1 => "png",
            2 => "webp",
            3 => "mp4",
            _ => "txt",
        };

        let path = dir.path().join(format!("page_{idx:05}.{ext}"));
        File::create(&path).expect("failed to create dataset file");

        if first_media.is_none() && ext != "txt" {
            first_media = Some(path);
        }
    }

    (
        dir,
        first_media.expect("dataset must contain at least one media file"),
    )
}

fn create_test_gif(path: &Path, width: u16, height: u16, frames: usize) {
    let mut output = File::create(path).expect("failed to create gif file");
    let mut encoder = gif::Encoder::new(&mut output, width, height, &[]).expect("gif encoder");
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .expect("gif repeat");

    for frame_idx in 0..frames {
        let mut rgba = vec![0u8; width as usize * height as usize * 4];

        for px in rgba.chunks_exact_mut(4) {
            let value = (frame_idx % 255) as u8;
            px[0] = value;
            px[1] = 255u8.wrapping_sub(value);
            px[2] = value / 2;
            px[3] = 255;
        }

        let mut frame = gif::Frame::from_rgba_speed(width, height, &mut rgba, 10);
        frame.delay = 4;
        encoder.write_frame(&frame).expect("write gif frame");
    }
}

fn bench_directory_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("directory_scan");

    for &size in &[1_000usize, 10_000usize] {
        let (_dir, anchor) = create_scan_dataset(size);
        group.bench_with_input(BenchmarkId::new("get_media_in_directory", size), &anchor, |b, p| {
            b.iter(|| {
                let files = image_loader::get_media_in_directory(black_box(p));
                black_box(files.len());
            });
        });
    }

    group.finish();
}

fn bench_directory_index_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("directory_index_cache");

    for &size in &[1_000usize, 10_000usize] {
        let (_dir, anchor) = create_scan_dataset(size);

        group.bench_with_input(BenchmarkId::new("cache_hit", size), &anchor, |b, p| {
            let mut index = media_index::MediaDirectoryIndex::new(64);
            let _ = index.media_in_directory_for_path(p);

            b.iter(|| {
                let files = index.media_in_directory_for_path(black_box(p));
                black_box(files.len());
            });
        });

        group.bench_with_input(BenchmarkId::new("cache_miss", size), &anchor, |b, p| {
            let mut index = media_index::MediaDirectoryIndex::new(64);

            b.iter(|| {
                if let Some(parent) = p.parent() {
                    index.invalidate_directory(parent);
                }
                let files = index.media_in_directory_for_path(black_box(p));
                black_box(files.len());
            });
        });
    }

    group.finish();
}

fn bench_gif_decode(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("failed to create gif tempdir");
    let gif_path = dir.path().join("animated_bench.gif");
    create_test_gif(&gif_path, 640, 360, 120);

    c.bench_function("gif_decode_120_frames", |b| {
        b.iter(|| {
            let image = image_loader::LoadedImage::load_with_max_texture_side(
                black_box(&gif_path),
                Some(2048),
                image::imageops::FilterType::Lanczos3,
                image::imageops::FilterType::Triangle,
            )
            .expect("gif decode failed");
            black_box(image.frame_count());
        });
    });
}

criterion_group!(
    perf_baseline,
    bench_directory_scan,
    bench_directory_index_cache,
    bench_gif_decode
);
criterion_main!(perf_baseline);
