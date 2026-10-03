use criterion::{criterion_group, criterion_main, Criterion};
use ergotop::canvas::Canvas;
use ergotop::viz::{Visualizer, VizItem};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const W: u16 = 200;
const H_CELLS: u16 = 50;

fn items(n: usize) -> Vec<VizItem> {
    let mut seed: u64 = 42;
    (0..n)
        .map(|i| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            VizItem {
                id: format!("{i:064}"),
                size_bytes: 200 + (seed >> 33) as u32 % 20_000,
                fee: 1_000_000 + (seed >> 20) % 10_000_000,
                color: Color::Rgb((seed >> 8) as u8, (seed >> 16) as u8, (seed >> 24) as u8),
            }
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let txs = items(10_000);
    c.bench_function("relayout_10k", |b| {
        b.iter(|| {
            let mut v = Visualizer::new();
            v.set_size(W, H_CELLS * 2);
            v.relayout(&txs, 1_271_009, false);
        })
    });
    let mut v = Visualizer::new();
    v.set_size(W, H_CELLS * 2);
    v.relayout(&txs, 1_271_009, false);
    let area = Rect::new(0, 0, W, H_CELLS);
    c.bench_function("render_frame_10k", |b| {
        b.iter(|| {
            let mut canvas = Canvas::new(W, H_CELLS * 2);
            v.render(&mut canvas, 0, Color::Gray);
            let mut buf = Buffer::empty(area);
            canvas.render(area, &mut buf);
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
