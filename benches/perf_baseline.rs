#[allow(dead_code)]
#[path = "../src/image_loader.rs"]
mod image_loader;

#[allow(dead_code)]
#[path = "../src/async_runtime.rs"]
mod async_runtime;

#[allow(dead_code)]
#[path = "../src/media_index.rs"]
mod media_index;

#[allow(dead_code)]
#[path = "../src/manga_spatial.rs"]
mod manga_spatial;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use manga_spatial::{MangaSpatialIndex, SpatialRect, STRIP_QUERY_HALF_WIDTH};
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
        group.bench_with_input(
            BenchmarkId::new("get_media_in_directory", size),
            &anchor,
            |b, p| {
                b.iter(|| {
                    let files = image_loader::get_media_in_directory(black_box(p));
                    black_box(files.len());
                });
            },
        );
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

fn build_strip_bounds(count: usize) -> Vec<(f32, f32)> {
    let mut bounds = Vec::with_capacity(count);
    let mut y = 0.0f32;

    for idx in 0..count {
        let h = 96.0 + ((idx.saturating_mul(37) % 280) as f32);
        let start = y;
        y += h;
        bounds.push((start, y));
    }

    bounds
}

fn strip_rects(bounds: &[(f32, f32)]) -> Vec<SpatialRect> {
    bounds
        .iter()
        .enumerate()
        .map(|(idx, (min_y, max_y))| {
            SpatialRect::new(
                idx,
                -STRIP_QUERY_HALF_WIDTH,
                *min_y,
                STRIP_QUERY_HALF_WIDTH,
                *max_y,
            )
        })
        .collect()
}

fn strip_linear_visible(bounds: &[(f32, f32)], min_y: f32, max_y: f32) -> Vec<usize> {
    let (min_y, max_y) = if min_y <= max_y {
        (min_y, max_y)
    } else {
        (max_y, min_y)
    };

    bounds
        .iter()
        .enumerate()
        .filter_map(|(idx, (start, end))| {
            if *start < max_y && *end > min_y {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

fn build_masonry_bounds(count: usize, columns: usize) -> Vec<(f32, f32, f32, f32)> {
    let columns = columns.max(1);
    let mut column_heights = vec![8.0f32; columns];
    let mut bounds = Vec::with_capacity(count);

    let col_width = 220.0f32;
    let gutter = 12.0f32;

    for idx in 0..count {
        let col = idx % columns;
        let x = col as f32 * (col_width + gutter);
        let y = column_heights[col];
        let h = 80.0 + ((idx.saturating_mul(53) % 260) as f32);
        let max_x = x + col_width;
        let max_y = y + h;

        bounds.push((x, y, max_x, max_y));
        column_heights[col] = max_y + gutter;
    }

    bounds
}

fn masonry_rects(bounds: &[(f32, f32, f32, f32)]) -> Vec<SpatialRect> {
    bounds
        .iter()
        .enumerate()
        .map(|(idx, (min_x, min_y, max_x, max_y))| {
            SpatialRect::new(idx, *min_x, *min_y, *max_x, *max_y)
        })
        .collect()
}

fn masonry_linear_visible(bounds: &[(f32, f32, f32, f32)], min_y: f32, max_y: f32) -> Vec<usize> {
    let (min_y, max_y) = if min_y <= max_y {
        (min_y, max_y)
    } else {
        (max_y, min_y)
    };

    bounds
        .iter()
        .enumerate()
        .filter_map(|(idx, (_, item_min_y, _, item_max_y))| {
            if *item_min_y < max_y && *item_max_y > min_y {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

fn bench_rtree_strip_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtree_strip_query");

    for &size in &[1_000usize, 10_000, 50_000, 100_000] {
        let bounds = build_strip_bounds(size);
        let rects = strip_rects(&bounds);
        let index = MangaSpatialIndex::from_rects(rects);

        let total_height = bounds.last().map(|(_, end)| *end).unwrap_or(0.0);
        let viewport_h = 960.0f32;

        let mut queries = Vec::with_capacity(64);
        let mut y = (total_height * 0.13).max(0.0);
        while y < total_height && queries.len() < 64 {
            queries.push((y, y + viewport_h));
            y += 733.0;
        }

        group.bench_with_input(BenchmarkId::new("linear", size), &size, |b, _| {
            b.iter(|| {
                let mut acc = 0usize;
                for (top, bottom) in &queries {
                    let indices = strip_linear_visible(black_box(&bounds), *top, *bottom);
                    acc = acc.saturating_add(indices.len());
                }
                black_box(acc);
            });
        });

        group.bench_with_input(BenchmarkId::new("rtree", size), &size, |b, _| {
            b.iter(|| {
                let mut acc = 0usize;
                for (top, bottom) in &queries {
                    let indices = index.query_vertical_band(*top, *bottom);
                    acc = acc.saturating_add(indices.len());
                }
                black_box(acc);
            });
        });
    }

    group.finish();
}

fn bench_rtree_masonry_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtree_masonry_query");

    for &size in &[1_000usize, 10_000, 50_000, 100_000] {
        let bounds = build_masonry_bounds(size, 8);
        let rects = masonry_rects(&bounds);
        let index = MangaSpatialIndex::from_rects(rects);

        let max_y = bounds
            .iter()
            .fold(0.0f32, |acc, (_, _, _, item_max_y)| acc.max(*item_max_y));

        let viewport_h = 1200.0f32;
        let mut queries = Vec::with_capacity(64);
        let mut y = (max_y * 0.17).max(0.0);
        while y < max_y && queries.len() < 64 {
            queries.push((y, y + viewport_h));
            y += 911.0;
        }

        group.bench_with_input(BenchmarkId::new("linear", size), &size, |b, _| {
            b.iter(|| {
                let mut acc = 0usize;
                for (top, bottom) in &queries {
                    let indices = masonry_linear_visible(black_box(&bounds), *top, *bottom);
                    acc = acc.saturating_add(indices.len());
                }
                black_box(acc);
            });
        });

        group.bench_with_input(BenchmarkId::new("rtree", size), &size, |b, _| {
            b.iter(|| {
                let mut acc = 0usize;
                for (top, bottom) in &queries {
                    let indices = index.query_vertical_band(*top, *bottom);
                    acc = acc.saturating_add(indices.len());
                }
                black_box(acc);
            });
        });
    }

    group.finish();
}

fn bench_rtree_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtree_rebuild");

    for &size in &[1_000usize, 10_000, 50_000, 100_000] {
        let strip_rects_template = strip_rects(&build_strip_bounds(size));
        let masonry_rects_template = masonry_rects(&build_masonry_bounds(size, 8));

        group.bench_with_input(BenchmarkId::new("strip_bulk_load", size), &size, |b, _| {
            b.iter_batched(
                || strip_rects_template.clone(),
                |rects| {
                    let index = MangaSpatialIndex::from_rects(rects);
                    black_box(index.len());
                },
                BatchSize::LargeInput,
            );
        });

        group.bench_with_input(
            BenchmarkId::new("masonry_bulk_load", size),
            &size,
            |b, _| {
                b.iter_batched(
                    || masonry_rects_template.clone(),
                    |rects| {
                        let index = MangaSpatialIndex::from_rects(rects);
                        black_box(index.len());
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    perf_baseline,
    bench_directory_scan,
    bench_directory_index_cache,
    bench_gif_decode,
    bench_rtree_strip_query,
    bench_rtree_masonry_query,
    bench_rtree_rebuild
);
criterion_main!(perf_baseline);
